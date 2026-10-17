//! `database::on-row-changed`: the function whose invocations are committed
//! write statements, and its owner side.
//!
//! The function is owner-delivered. Its observers' args (`{ db, table?, ops? }`)
//! say which events they want; [`NativeCapture`] is its [`ObserveHandler`]:
//! for each observer added it validates the args against the live config,
//! installs native capture for the table when the database needs it, and
//! files the observer in a routing index. The function's handler
//! ([`NativeCapture::route`]) looks the event up in that index and hands it to
//! each routed observer with `obs.emit`; the observer's own filters run in the
//! SDK, so a miss sends nothing.
//!
//! Old consumers keep working two ways:
//!
//! * against an engine with observe, the function declares the old trigger
//!   type as its `legacy_trigger_type`, and the engine turns each
//!   `registerTrigger({ type: 'database::row-changed' })` into an observer
//!   whose `legacy.config` is the old config (the legacy shim);
//! * against an older engine, `legacy.rs` registers the old trigger type and
//!   files each binding in the same index as a [`Sink::Trigger`].
//!
//! Either way a legacy binding receives the event in its pre-0.25 shape
//! ([`RowChangedEvent::legacy_view`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction, TriggerAction,
};
use serde_json::Value;

use super::bus::{RowChangedArgs, RowChangedEvent, ROW_CHANGED_TYPE};
use super::native;
use super::sql::{self, Op};
use crate::config::{CaptureMode, WorkerConfig};
use crate::pool::Pool;

/// The function this worker announces its writes through.
pub const ROW_CHANGED_FN: &str = "database::on-row-changed";

pub const ROW_CHANGED_DESCRIPTION: &str =
    "Invoked once per committed write statement. Observe it with args { db, table?, ops? } \
     (table canonical: bare, unquoted, lowercase) and filter on input.returning / caller. \
     On a `capture: statements` database (the default) it reports only writes made THROUGH \
     this worker, with the writer's own RETURNING rows as `returning`; on a `capture: native` \
     database it reports any client's committed writes, with the changed rows' primary keys \
     as `returning` (capped; `truncated` flags the rest). Best-effort delivery, no replay.";

/// Registers `database::on-row-changed` with `capture` as its handler and
/// hooks, and attaches the returned ref to `capture`. Its observers' args are
/// what the trigger config was; the old trigger type is its
/// `legacy_trigger_type`, so the engine turns old `registerTrigger` calls into
/// observers (the legacy shim).
pub fn register(iii: &IIIClient, capture: &Arc<NativeCapture>) -> FunctionRef {
    let function = iii.register_function(
        ROW_CHANGED_FN,
        RegisterFunction::new_async({
            let capture = capture.clone();
            move |event: RowChangedEvent| {
                let capture = capture.clone();
                async move {
                    capture.route(&event).await;
                    Ok::<(), iii_sdk::errors::Error>(())
                }
            }
        })
        .description(ROW_CHANGED_DESCRIPTION)
        .observe(
            Observe::owner()
                .args_format::<RowChangedArgs>()
                .legacy_trigger_type(ROW_CHANGED_TYPE)
                .handler(capture.clone()),
        ),
    );
    capture.attach(function.clone());
    function
}

/// Rejection codes an observer's `ready()` fails with.
pub mod codes {
    pub const UNKNOWN_DB: &str = "UNKNOWN_DB";
    pub const NATIVE_CAPTURE_REQUIRES_TABLE: &str = "NATIVE_CAPTURE_REQUIRES_TABLE";
    pub const NON_CANONICAL_TABLE: &str = "NON_CANONICAL_TABLE";
    pub const CAPTURE_INSTALL_FAILED: &str = "CAPTURE_INSTALL_FAILED";
    /// A legacy binding's config that does not parse (the shim skips the
    /// engine's `args_format` check for it).
    pub const CONFIG_ERROR: &str = "CONFIG_ERROR";
}

/// Where a routed event goes.
#[derive(Clone)]
pub enum Sink {
    /// An observer (new, or made by the engine's legacy shim).
    Observer(Observer),
    /// A pre-0.25 binding, against an engine without observe (`legacy.rs`):
    /// called with `void`, its stored metadata riding along (a harness
    /// binding's `__binding` pointer).
    Trigger {
        function_id: String,
        metadata: Option<Value>,
    },
}

/// One entry of the routing index.
#[derive(Clone)]
pub struct Route {
    pub id: String,
    pub db: String,
    /// Canonical.
    pub table: Option<String>,
    pub ops: Option<Vec<Op>>,
    pub sink: Sink,
    /// Whether the event goes out in its pre-0.25 shape.
    pub legacy: bool,
    /// `(db, table as installed)` whose native capture this route holds.
    installed: Option<(String, String)>,
}

impl Route {
    fn accepts(&self, op: Op) -> bool {
        self.ops.as_ref().is_none_or(|ops| ops.contains(&op))
    }
}

type Key = (String, Option<String>);

/// `(db, table)` → routes: the args half of what the old `RowChangeBus`
/// matched per subscriber. The filters half belongs to the observers.
#[derive(Default)]
struct RouteIndex {
    inner: RwLock<IndexInner>,
    trigger_sinks: AtomicUsize,
}

#[derive(Default)]
struct IndexInner {
    by_key: HashMap<Key, Vec<Arc<Route>>>,
    by_id: HashMap<String, Arc<Route>>,
}

impl RouteIndex {
    /// Files `route`, replacing one with the same id (a re-registered
    /// trigger). Returns the replaced route.
    fn insert(&self, route: Route) -> Option<Arc<Route>> {
        let route = Arc::new(route);
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let replaced = Self::take(&mut inner, &route.id);
        if let Some(old) = &replaced {
            if matches!(old.sink, Sink::Trigger { .. }) {
                self.trigger_sinks.fetch_sub(1, Ordering::AcqRel);
            }
        }
        if matches!(route.sink, Sink::Trigger { .. }) {
            self.trigger_sinks.fetch_add(1, Ordering::AcqRel);
        }
        inner
            .by_key
            .entry((route.db.clone(), route.table.clone()))
            .or_default()
            .push(route.clone());
        inner.by_id.insert(route.id.clone(), route);
        replaced
    }

    fn remove(&self, id: &str) -> Option<Arc<Route>> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let removed = Self::take(&mut inner, id);
        if let Some(route) = &removed {
            if matches!(route.sink, Sink::Trigger { .. }) {
                self.trigger_sinks.fetch_sub(1, Ordering::AcqRel);
            }
        }
        removed
    }

    fn take(inner: &mut IndexInner, id: &str) -> Option<Arc<Route>> {
        let route = inner.by_id.remove(id)?;
        let key = (route.db.clone(), route.table.clone());
        if let Some(list) = inner.by_key.get_mut(&key) {
            list.retain(|r| r.id != id);
            if list.is_empty() {
                inner.by_key.remove(&key);
            }
        }
        Some(route)
    }

    /// The routes on `(db, table)` and those on `db` with no table, keeping
    /// those whose `ops` are absent or include `op`. An event whose table is
    /// unknown reaches only the db-wide routes: silence beats a wrong match.
    fn matching(&self, db: &str, table: Option<&str>, op: Op) -> Vec<Arc<Route>> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        if let Some(table) = table {
            if let Some(list) = inner.by_key.get(&(db.to_string(), Some(table.to_string()))) {
                out.extend(list.iter().filter(|r| r.accepts(op)).cloned());
            }
        }
        if let Some(list) = inner.by_key.get(&(db.to_string(), None)) {
            out.extend(list.iter().filter(|r| r.accepts(op)).cloned());
        }
        out
    }

    fn has_trigger_sinks(&self) -> bool {
        self.trigger_sinks.load(Ordering::Acquire) > 0
    }

    fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .by_id
            .len()
    }
}

/// Native capture and the routing index, driven by the function's observers.
pub struct NativeCapture {
    iii: Arc<IIIClient>,
    /// Live configuration, swapped together with the pools on hot reload.
    config: Arc<tokio::sync::RwLock<WorkerConfig>>,
    /// Live pools: a native observer installs its database triggers through
    /// the bound database's pool.
    pools: Arc<tokio::sync::RwLock<HashMap<String, Pool>>>,
    index: RouteIndex,
    /// `(db, table as installed)` → routes holding its native capture. The first
    /// installs; the DDL stays when the last one leaves (it is idempotent to
    /// reinstall and an orphan notify per write is near-free).
    installed: Mutex<HashMap<(String, String), usize>>,
    function: OnceLock<FunctionRef>,
    dispatch_timeout_ms: u64,
}

impl NativeCapture {
    pub fn new(
        iii: Arc<IIIClient>,
        config: Arc<tokio::sync::RwLock<WorkerConfig>>,
        pools: Arc<tokio::sync::RwLock<HashMap<String, Pool>>>,
        dispatch_timeout_ms: u64,
    ) -> Self {
        Self {
            iii,
            config,
            pools,
            index: RouteIndex::default(),
            installed: Mutex::new(HashMap::new()),
            function: OnceLock::new(),
            dispatch_timeout_ms,
        }
    }

    /// A capture with no databases, for tests of the buffer.
    pub(crate) fn detached(iii: Arc<IIIClient>, dispatch_timeout_ms: u64) -> Self {
        Self::new(
            iii,
            Arc::new(tokio::sync::RwLock::new(WorkerConfig {
                databases: HashMap::new(),
                ..WorkerConfig::default()
            })),
            Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            dispatch_timeout_ms,
        )
    }

    /// The registered function. Set once, right after `register_function`
    /// returns (the handler and the hooks hold this capture, so it exists
    /// before the function does).
    pub fn attach(&self, function: FunctionRef) {
        let _ = self.function.set(function);
    }

    pub fn function(&self) -> Option<&FunctionRef> {
        self.function.get()
    }

    pub(crate) fn dispatch_timeout_ms(&self) -> u64 {
        self.dispatch_timeout_ms
    }

    /// Whether any event can reach anyone: an active observer, or a binding
    /// through the pre-0.25 fallback. Two atomic loads.
    #[inline]
    pub fn is_listened(&self) -> bool {
        self.function.get().is_some_and(FunctionRef::is_observed) || self.index.has_trigger_sinks()
    }

    /// Whether a `trigger` of the function runs in-process (the engine
    /// confirmed it with local dispatch).
    pub fn local_dispatch_active(&self) -> bool {
        self.function
            .get()
            .is_some_and(FunctionRef::local_dispatch_active)
    }

    pub fn route_count(&self) -> usize {
        self.index.len()
    }

    /// The function's handler: hand `event` to every routed observer. A miss
    /// on an observer's filters, or an observer removed meanwhile, is
    /// `Ok(false)` from `emit`, so the loop needs no guard.
    pub async fn route(&self, event: &RowChangedEvent) {
        let routes = self
            .index
            .matching(&event.db, event.table.as_deref(), event.op);
        if routes.is_empty() {
            return;
        }
        let mut legacy_value: Option<Value> = None;
        for route in routes {
            match &route.sink {
                Sink::Observer(obs) => {
                    let sent = if route.legacy {
                        obs.emit(&event.legacy_view())
                    } else {
                        obs.emit(event)
                    };
                    if let Err(e) = sent {
                        tracing::warn!(observer = %obs.id(), error = %e, "row-changed emit failed");
                    }
                }
                Sink::Trigger {
                    function_id,
                    metadata,
                } => {
                    let payload = legacy_value
                        .get_or_insert_with(|| {
                            serde_json::to_value(event.legacy_view()).unwrap_or(Value::Null)
                        })
                        .clone();
                    let request = TriggerRequest {
                        function_id: function_id.clone(),
                        payload,
                        action: Some(TriggerAction::Void),
                        timeout_ms: Some(self.dispatch_timeout_ms),
                    };
                    let res = match metadata {
                        Some(m) => self.iii.trigger(request.metadata(m.clone())).await,
                        None => self.iii.trigger(request).await,
                    };
                    if let Err(e) = res {
                        tracing::warn!(function_id = %function_id, error = %e, "row-changed dispatch failed");
                    }
                }
            }
        }
    }

    /// A new observer's args: `table` must already be canonical, so the
    /// mismatch fails at creation instead of matching nothing.
    pub fn check_canonical(args: &RowChangedArgs) -> Result<(), ObserveRejection> {
        if let Some(table) = &args.table {
            let canonical = sql::canonical_table(table);
            if &canonical != table {
                return Err(ObserveRejection::new(
                    codes::NON_CANONICAL_TABLE,
                    format!(
                        "args.table `{table}` is not canonical; use `{canonical}` (bare name, \
                         unquoted, lowercase; the schema is in the event's `schema`)"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// A legacy binding's config, validated as the pre-0.25 trigger handler
    /// did, and its args with the canonical table (so it joins the index as
    /// a new observer with those args would). Also returns the table as the
    /// binding spelled it: native capture installs on that spelling, as before.
    pub fn legacy_args(
        config: &Value,
    ) -> Result<(RowChangedArgs, Option<String>), ObserveRejection> {
        let raw: RowChangedArgs = serde_json::from_value(config.clone()).map_err(|e| {
            ObserveRejection::new(codes::CONFIG_ERROR, format!("row-changed config: {e}"))
        })?;
        let install = raw.table.clone();
        let args = RowChangedArgs {
            table: raw.table.as_deref().map(sql::canonical_table),
            ..raw
        };
        Ok((args, install))
    }

    /// Validates `args` against the live config and, on a native database,
    /// installs capture for the table (once per `(db, table)`). Returns the
    /// `(db, table)` the caller now holds, to release on removal.
    pub async fn admit(
        &self,
        args: &RowChangedArgs,
        install_table: Option<&str>,
    ) -> Result<Option<(String, String)>, ObserveRejection> {
        let live = self.config.read().await;
        let Some(db_cfg) = live.databases.get(&args.db) else {
            let mut known = live.databases.keys().cloned().collect::<Vec<_>>();
            known.sort();
            return Err(ObserveRejection::new(
                codes::UNKNOWN_DB,
                format!(
                    "unknown db `{}`; available: [{}]",
                    args.db,
                    known.join(", ")
                ),
            ));
        };
        let native = db_cfg.capture == CaptureMode::Native;
        drop(live);
        if !native {
            return Ok(None);
        }
        let (Some(_), Some(install_table)) = (args.table.as_deref(), install_table) else {
            return Err(ObserveRejection::new(
                codes::NATIVE_CAPTURE_REQUIRES_TABLE,
                format!(
                    "db `{}` uses `capture: native`, which requires args.table — per-table \
                     database triggers are what make external writes visible",
                    args.db
                ),
            ));
        };
        // Keyed by the spelling capture is installed on, not the canonical
        // name: on postgres `"ORDERS"` and `"orders"` are different relations,
        // and a legacy binding spelled in the wrong case must still fail its
        // own install (as it did before) instead of riding on another's.
        let key = (args.db.clone(), install_table.to_string());
        let held = self
            .installed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .copied()
            .unwrap_or(0);
        if held == 0 {
            // Hooks run one at a time per function, so no second install of
            // the same table races this one in this worker; the advisory lock
            // in `install` covers other processes.
            self.install(&args.db, install_table).await?;
        }
        *self
            .installed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_default() += 1;
        Ok(Some(key))
    }

    /// Files a route admitted with [`admit`](Self::admit).
    pub fn file(
        &self,
        route_id: String,
        args: RowChangedArgs,
        sink: Sink,
        legacy: bool,
        installed: Option<(String, String)>,
    ) {
        let replaced = self.index.insert(Route {
            id: route_id,
            db: args.db,
            table: args.table,
            ops: args.ops,
            sink,
            legacy,
            installed,
        });
        if let Some(old) = replaced {
            self.release(&old);
        }
    }

    /// Removes a route and releases its hold on native capture.
    pub fn unfile(&self, route_id: &str) {
        if let Some(route) = self.index.remove(route_id) {
            self.release(&route);
        }
    }

    fn release(&self, route: &Route) {
        let Some(key) = &route.installed else {
            return;
        };
        let mut installed = self.installed.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = installed.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                // ponytail: native-capture triggers stay installed — an orphan
                // pg_notify per write statement is near-free and idempotent to
                // reinstall; add DDL teardown when someone actually needs it.
                installed.remove(key);
            }
        }
    }

    /// After a config reload: late-rejects every observer the new config no
    /// longer supports (its database was removed, or became native while the
    /// observer names no table). Fallback bindings stay filed, as before.
    pub async fn revalidate(&self) {
        let Some(function) = self.function.get() else {
            return;
        };
        let live = self.config.read().await;
        for obs in function.observers() {
            let args = match obs.legacy() {
                Some(legacy) => Self::legacy_args(&legacy.config).map(|(args, _)| args),
                None => obs.args::<RowChangedArgs>(),
            };
            let Ok(args) = args else { continue };
            match live.databases.get(&args.db) {
                None => obs.reject(
                    codes::UNKNOWN_DB,
                    format!("db `{}` was removed from the configuration", args.db),
                ),
                Some(db) if db.capture == CaptureMode::Native && args.table.is_none() => obs
                    .reject(
                        codes::NATIVE_CAPTURE_REQUIRES_TABLE,
                        format!(
                            "db `{}` now uses `capture: native`; pass args.table",
                            args.db
                        ),
                    ),
                Some(_) => {}
            }
        }
    }

    /// Install the NOTIFY function and per-table triggers for a native
    /// observer. Fails loudly — an observer whose DDL did not land would sit
    /// there hearing nothing, which is the failure mode this worker refuses.
    async fn install(&self, db: &str, table: &str) -> Result<(), ObserveRejection> {
        let install_error =
            |message: String| ObserveRejection::new(codes::CAPTURE_INSTALL_FAILED, message);
        let pool = self.pools.read().await.get(db).cloned();
        match pool {
            Some(Pool::Postgres(pg)) => {
                let sql = native::install_sql(table).map_err(install_error)?;
                let mut client = pg
                    .acquire()
                    .await
                    .map_err(|e| install_error(format!("db `{db}`: acquiring connection: {e}")))?;
                let pg_error = |error: tokio_postgres::Error| {
                    let text = error
                        .as_db_error()
                        .map(|error| error.message().to_string())
                        .unwrap_or_else(|| error.to_string());
                    install_error(format!(
                        "installing native capture triggers on `{table}`: {text}{}",
                        pg_install_hint(&text, table)
                    ))
                };
                // Every install replaces the SAME notify function, including
                // installs for different tables. Concurrent CREATE OR REPLACE
                // can fail with `tuple concurrently updated`. A DB-scoped lock
                // coordinates separate pools/processes too.
                let transaction = client
                    .build_transaction()
                    .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
                    .start()
                    .await
                    .map_err(pg_error)?;
                transaction
                    .batch_execute("SELECT pg_advisory_xact_lock(1768515886, 1919907683)")
                    .await
                    .map_err(pg_error)?;
                transaction.batch_execute(&sql).await.map_err(pg_error)?;
                transaction.commit().await.map_err(pg_error)?;
            }
            Some(Pool::Sqlite(sq)) => {
                let conn = sq
                    .acquire()
                    .await
                    .map_err(|e| install_error(format!("db `{db}`: acquiring connection: {e}")))?;
                let table_owned = table.to_string();
                tokio::task::spawn_blocking(move || {
                    conn.with(|c| -> Result<(), String> {
                        super::sqlite_watch::ensure_changelog(c).map_err(|e| e.to_string())?;
                        let pk = super::sqlite_watch::pk_columns(c, &table_owned)
                            .map_err(|e| e.to_string())?;
                        let sql = super::sqlite_watch::install_sql(&table_owned, &pk)?;
                        c.execute_batch(&sql).map_err(|e| e.to_string())
                    })
                })
                .await
                .map_err(|e| install_error(format!("sqlite DDL join: {e}")))?
                .map_err(|e| {
                    install_error(format!(
                        "installing native capture triggers on `{table}`: {e}"
                    ))
                })?;
            }
            Some(Pool::Mysql(my)) => {
                // Binlog capture installs nothing — but an observer on a
                // server that cannot be streamed would sit silent forever.
                use mysql_async::prelude::Queryable as _;
                let mut conn = my
                    .acquire()
                    .await
                    .map_err(|e| install_error(format!("db `{db}`: acquiring connection: {e}")))?;
                let settings: Option<(i64, String)> = conn
                    .query_first("SELECT @@log_bin, @@binlog_format")
                    .await
                    .map_err(|e| install_error(format!("db `{db}`: {e}")))?;
                match settings {
                    Some((1, format)) if format.eq_ignore_ascii_case("ROW") => {}
                    Some((1, format)) => {
                        return Err(install_error(format!(
                            "db `{db}`: binlog_format is {format}; native capture needs ROW \
                             (SET GLOBAL binlog_format = 'ROW', the 8.x default)"
                        )));
                    }
                    _ => {
                        return Err(install_error(format!(
                            "db `{db}`: the server runs without a binary log (log_bin=OFF); \
                             native capture reads the binlog and cannot work here"
                        )));
                    }
                }
                super::mysql_binlog::binlog_position(&mut conn)
                    .await
                    .map_err(|e| install_error(format!("db `{db}`: {e}")))?;
                let meta: Option<String> = conn
                    .query_first("SELECT @@binlog_row_metadata")
                    .await
                    .ok()
                    .flatten();
                if !meta
                    .as_deref()
                    .is_some_and(|m| m.eq_ignore_ascii_case("FULL"))
                {
                    tracing::warn!(
                        db = %db,
                        table = %table,
                        "binlog_row_metadata is not FULL; row-changed events on this database \
                         carry no primary-key values (SET GLOBAL binlog_row_metadata = 'FULL', \
                         or binlog_row_metadata=FULL in my.cnf)"
                    );
                }
            }
            None => {
                return Err(install_error(format!(
                    "db `{db}`: no pool available for native capture"
                )));
            }
        }
        tracing::info!(db = %db, table = %table, "native capture triggers installed");
        Ok(())
    }
}

#[async_trait]
impl ObserveHandler for NativeCapture {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        let (args, install, legacy) = match obs.legacy() {
            // The engine's legacy shim: `config` is the old trigger config,
            // not validated against args_format. Validate it as the trigger
            // handler did and canonicalize its table.
            Some(legacy) => {
                let (args, install) = Self::legacy_args(&legacy.config)?;
                (args, install, true)
            }
            None => {
                let args = obs.args::<RowChangedArgs>()?; // db required: checked by the engine
                Self::check_canonical(&args)?;
                let install = args.table.clone();
                (args, install, false)
            }
        };
        let installed = self.admit(&args, install.as_deref()).await?;
        tracing::info!(
            observer = %obs.id(),
            target = %obs.target(),
            legacy,
            db = %args.db,
            table = ?args.table,
            "row-changed observer added"
        );
        // Indexed before the ack: the first commit after the engine marks the
        // observer active reaches it.
        self.file(
            obs.id().to_string(),
            args,
            Sink::Observer(obs.clone()),
            legacy,
            installed,
        );
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.unfile(obs.id());
        tracing::info!(observer = %obs.id(), "row-changed observer removed");
    }
}

/// Pick the guidance appended to a postgres trigger-install failure. The
/// hint must match the actual failure: dressing a `does not exist` error in
/// privilege advice sent a real operator chasing grants when the problem
/// was table-name casing (native capture quotes the name verbatim, and
/// quoted postgres identifiers are case-sensitive).
fn pg_install_hint(error_text: &str, table: &str) -> String {
    if error_text.contains("does not exist") {
        format!(
            ". Note: the table name is quoted verbatim into DDL and quoted postgres \
             identifiers are case-sensitive — `{table}` must match the table's actual spelling"
        )
    } else if error_text.contains("permission denied") || error_text.contains("must be owner") {
        ". The configured role needs TRIGGER privilege on the table (or ownership) \
         and CREATE on its schema"
            .to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(id: &str, db: &str, table: Option<&str>, ops: Option<Vec<Op>>) -> Route {
        Route {
            id: id.into(),
            db: db.into(),
            table: table.map(str::to_string),
            ops,
            sink: Sink::Trigger {
                function_id: format!("app::{id}"),
                metadata: None,
            },
            legacy: true,
            installed: None,
        }
    }

    fn ids(routes: Vec<Arc<Route>>) -> Vec<String> {
        routes.into_iter().map(|r| r.id.clone()).collect()
    }

    #[test]
    fn routing_matches_db_table_and_ops() {
        let index = RouteIndex::default();
        index.insert(route("orders", "primary", Some("orders"), None));
        index.insert(route("everything", "primary", None, None));
        index.insert(route(
            "deletes",
            "primary",
            Some("orders"),
            Some(vec![Op::Delete]),
        ));
        index.insert(route("other-db", "analytics", None, None));

        assert_eq!(
            ids(index.matching("primary", Some("orders"), Op::Insert)),
            ["orders", "everything"]
        );
        assert_eq!(
            ids(index.matching("primary", Some("orders"), Op::Delete)),
            ["orders", "deletes", "everything"]
        );
        // An observer on `orders` never receives `payments` events.
        assert_eq!(
            ids(index.matching("primary", Some("payments"), Op::Update)),
            ["everything"]
        );
        // An unclassifiable table reaches only the db-wide route.
        assert_eq!(
            ids(index.matching("primary", None, Op::Other)),
            ["everything"]
        );
        assert_eq!(
            ids(index.matching("analytics", Some("orders"), Op::Delete)),
            ["other-db"]
        );
    }

    #[test]
    fn filing_is_idempotent_on_the_route_id() {
        let index = RouteIndex::default();
        index.insert(route("i1", "primary", None, None));
        assert!(index
            .insert(route("i1", "primary", Some("orders"), None))
            .is_some());
        assert_eq!(index.len(), 1);
        assert!(index.has_trigger_sinks());
        assert_eq!(
            ids(index.matching("primary", Some("payments"), Op::Insert)),
            Vec::<String>::new()
        );
        index.remove("i1");
        assert_eq!(index.len(), 0);
        assert!(!index.has_trigger_sinks());
        assert!(index.remove("nope").is_none());
    }

    #[test]
    fn non_canonical_tables_are_refused_with_a_hint() {
        for (table, hint) in [
            ("Orders", "orders"),
            ("public.orders", "orders"),
            ("\"orders\"", "orders"),
        ] {
            let err = NativeCapture::check_canonical(&RowChangedArgs {
                db: "primary".into(),
                table: Some(table.into()),
                ops: None,
            })
            .unwrap_err();
            assert_eq!(err.code, codes::NON_CANONICAL_TABLE);
            assert!(
                err.message.contains(&format!("use `{hint}`")),
                "{}",
                err.message
            );
        }
        NativeCapture::check_canonical(&RowChangedArgs {
            db: "primary".into(),
            table: Some("orders".into()),
            ops: None,
        })
        .unwrap();
    }

    #[test]
    fn legacy_configs_join_the_index_with_the_canonical_table() {
        let (args, install) = NativeCapture::legacy_args(&serde_json::json!({
            "db": "primary", "table": "public.Orders"
        }))
        .unwrap();
        assert_eq!(args.table.as_deref(), Some("orders"));
        // Native capture still installs on the spelling the binding used.
        assert_eq!(install.as_deref(), Some("public.Orders"));
        let err =
            NativeCapture::legacy_args(&serde_json::json!({ "db": "p", "tabl": "x" })).unwrap_err();
        assert_eq!(err.code, codes::CONFIG_ERROR);
    }

    fn capture_with(yaml: &str) -> NativeCapture {
        NativeCapture::new(
            Arc::new(IIIClient::new("ws://127.0.0.1:9")),
            Arc::new(tokio::sync::RwLock::new(
                WorkerConfig::from_yaml(yaml).unwrap(),
            )),
            Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            100,
        )
    }

    #[tokio::test]
    async fn admission_checks_the_live_config() {
        let capture =
            capture_with("databases:\n  p:\n    url: postgres://u@h/db\n    capture: native\n");
        let unknown = RowChangedArgs {
            db: "nope".into(),
            table: None,
            ops: None,
        };
        assert_eq!(
            capture.admit(&unknown, None).await.unwrap_err().code,
            codes::UNKNOWN_DB
        );
        let tableless = RowChangedArgs {
            db: "p".into(),
            table: None,
            ops: None,
        };
        assert_eq!(
            capture.admit(&tableless, None).await.unwrap_err().code,
            codes::NATIVE_CAPTURE_REQUIRES_TABLE
        );
        // With a table but no live pool the DDL cannot land: refused, not
        // silently listening to nothing.
        let with_table = RowChangedArgs {
            db: "p".into(),
            table: Some("orders".into()),
            ops: None,
        };
        let err = capture
            .admit(&with_table, Some("orders"))
            .await
            .unwrap_err();
        assert_eq!(err.code, codes::CAPTURE_INSTALL_FAILED);
        assert!(err.message.contains("no pool"), "{}", err.message);
    }

    #[test]
    fn pg_install_hint_matches_the_failure_shape() {
        let hint = pg_install_hint(
            r#"relation "III_TRIGGER_TEST" does not exist"#,
            "III_TRIGGER_TEST",
        );
        assert!(hint.contains("case-sensitive"), "{hint}");
        assert!(!hint.contains("TRIGGER privilege"), "{hint}");
        let hint = pg_install_hint("permission denied for table orders", "orders");
        assert!(hint.contains("TRIGGER privilege"), "{hint}");
        assert_eq!(pg_install_hint("connection reset by peer", "orders"), "");
    }
}
