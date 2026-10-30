//! `database::on-row-changed`: one event per committed write statement, and
//! the hooks that say who hears it.
//!
//! The function is private and its handler returns the event it is called
//! with, so observers observe the event. Its observers' args
//! (`{ db, table?, ops? }`) say which events they want:
//!
//! * [`NativeCapture`] is its added and removed hook. For each observer added
//!   it validates the args against the live config and, on a `capture:
//!   native` database, installs capture for the table; for each observer
//!   removed it releases that hold.
//! * Its match hook files each observer under the keys of its db, table and
//!   ops ([`observer_keys`]), and looks each call up under the keys of the
//!   event's db, table and op, wildcards included ([`call_keys`]). The
//!   observer's own filters run after, in iii's delivery.
//!
//! The worker calls the function itself ([`NativeCapture::call`]): the bus
//! calls it from the invocation that made the write, so the call's `caller`
//! names that invocation and `caller.parent` the worker that asked for the
//! write; native listeners call it from their own tasks. A call from any
//! other worker is refused by the engine (`FORBIDDEN`): it would be a forged
//! row change carrying this worker's identity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{Call, CallRequest, IIIClient, Match, ObserveRejection, Observer, RegisterFunction};
use serde_json::{json, Value};

use super::bus::{RowChangedArgs, RowChangedEvent};
use super::native;
use super::sql;
use crate::config::{CaptureMode, WorkerConfig};
use crate::pool::Pool;

/// The function this worker announces its writes through.
pub const ROW_CHANGED_FN: &str = "database::on-row-changed";

pub const ROW_CHANGED_DESCRIPTION: &str =
    "One event per committed write statement. Observe it with args { db, table?, ops? } \
     (table canonical: bare, unquoted, lowercase) and filter on input.returning / caller. \
     On a `capture: statements` database (the default) it reports only writes made THROUGH \
     this worker, with the writer's own RETURNING rows as `returning`; on a `capture: native` \
     database it reports any client's committed writes, with the changed rows' primary keys \
     as `returning` (capped; `truncated` flags the rest). Best-effort delivery, no replay. \
     Private: only the database worker calls it.";

/// Registers `database::on-row-changed` with `capture` as its added and
/// removed hooks and the db/table/op keys as its match hook, and attaches
/// the returned ref to `capture`.
///
/// The hooks are set by id before the function registers, so they run for
/// every observer, including those the engine hands over with the
/// registration.
pub fn register(iii: &IIIClient, capture: &Arc<NativeCapture>) -> FunctionRef {
    let added = capture.clone();
    iii.on_observer_added_for(ROW_CHANGED_FN, move |obs: Observer| {
        let capture = added.clone();
        async move { capture.added(obs).await }
    });
    let removed = capture.clone();
    iii.on_observer_removed_for(ROW_CHANGED_FN, move |obs: Observer| {
        let capture = removed.clone();
        async move { capture.removed(&obs) }
    });
    iii.on_observer_match_for(
        ROW_CHANGED_FN,
        Match::keys(
            |obs| observer_keys(obs.args_value()).into(),
            |call| call_keys(&call.input).into(),
        ),
    );
    let function = iii.register_function(
        ROW_CHANGED_FN,
        RegisterFunction::new(|event: RowChangedEvent| Ok::<_, Error>(event))
            .description(ROW_CHANGED_DESCRIPTION)
            .args_format::<RowChangedArgs>()
            .private(),
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
}

/// One key of the match index: a database, a table (`None`: every table)
/// and an operation (`None`: every operation). Written as a JSON array, so
/// no database or table name collides with another key or with a wildcard.
fn key(db: &str, table: Option<&str>, op: Option<&str>) -> String {
    json!([db, table, op]).to_string()
}

/// The keys an observer is filed under, from its args: one per operation
/// in `ops` (every operation when it names none), on `table` (every table
/// when it names none). An empty `ops` files it nowhere: it hears nothing.
pub fn observer_keys(args: &Value) -> Vec<String> {
    let Some(db) = args["db"].as_str() else {
        return Vec::new();
    };
    let table = args["table"].as_str();
    match args["ops"].as_array() {
        Some(ops) => ops
            .iter()
            .filter_map(Value::as_str)
            .map(|op| key(db, table, Some(op)))
            .collect(),
        None => vec![key(db, table, None)],
    }
}

/// The keys a call matches, from its event: its table and every table, its
/// operation and every operation. An event whose table is unknown matches
/// only the observers of every table: silence beats a wrong match.
pub fn call_keys(input: &Value) -> Vec<String> {
    let Some(db) = input["db"].as_str() else {
        return Vec::new();
    };
    let op = input["op"].as_str();
    let mut keys = Vec::with_capacity(4);
    if let Some(table) = input["table"].as_str() {
        keys.push(key(db, Some(table), op));
        keys.push(key(db, Some(table), None));
    }
    keys.push(key(db, None, op));
    keys.push(key(db, None, None));
    keys
}

/// The `(db, table)` whose native capture an observer holds.
type Installed = (String, String);

/// Native capture for the function's observers, and its call.
pub struct NativeCapture {
    iii: Arc<IIIClient>,
    /// Live configuration, swapped together with the pools on hot reload.
    config: Arc<tokio::sync::RwLock<WorkerConfig>>,
    /// Live pools: a native observer installs its database triggers through
    /// the bound database's pool.
    pools: Arc<tokio::sync::RwLock<HashMap<String, Pool>>>,
    /// Accepted observers by id, each with the native capture it holds.
    observers: Mutex<HashMap<String, Option<Installed>>>,
    /// `(db, table)` → observers holding its native capture. The first
    /// installs; the DDL stays when the last one leaves (it is idempotent to
    /// reinstall and an orphan notify per write is near-free).
    installed: Mutex<HashMap<Installed, usize>>,
    function: OnceLock<FunctionRef>,
    call_timeout_ms: u64,
}

impl NativeCapture {
    pub fn new(
        iii: Arc<IIIClient>,
        config: Arc<tokio::sync::RwLock<WorkerConfig>>,
        pools: Arc<tokio::sync::RwLock<HashMap<String, Pool>>>,
        call_timeout_ms: u64,
    ) -> Self {
        Self {
            iii,
            config,
            pools,
            observers: Mutex::new(HashMap::new()),
            installed: Mutex::new(HashMap::new()),
            function: OnceLock::new(),
            call_timeout_ms,
        }
    }

    /// A capture with no databases and no function, for tests of the buffer:
    /// its calls go nowhere.
    pub(crate) fn detached(iii: Arc<IIIClient>, call_timeout_ms: u64) -> Self {
        Self::new(
            iii,
            Arc::new(tokio::sync::RwLock::new(WorkerConfig {
                databases: HashMap::new(),
                ..WorkerConfig::default()
            })),
            Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            call_timeout_ms,
        )
    }

    /// The registered function. Set once, right after `register_function`
    /// returns.
    pub fn attach(&self, function: FunctionRef) {
        let _ = self.function.set(function);
    }

    /// Whether the function has any active observer: one atomic load.
    #[inline]
    pub fn is_observed(&self) -> bool {
        self.function.get().is_some_and(FunctionRef::is_observed)
    }

    /// How many observers this worker has accepted and not yet removed.
    pub fn observer_count(&self) -> usize {
        self.observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Calls `database::on-row-changed` with `event` when the call would
    /// reach an observer: one atomic load when nothing observes the function,
    /// one hash lookup per key when its observers watch other tables, and
    /// the event serialized only when one of them watches this one.
    ///
    /// Called without an action and awaited, so a transaction's statements
    /// reach observers in commit order. The call runs in-process (the
    /// handler returns the event) and iii's delivery does not wait for the
    /// observers' targets; `call_timeout_ms` bounds the round trip when the
    /// engine has not yet confirmed in-process calls.
    pub async fn call(&self, event: &RowChangedEvent) {
        let Some(function) = self.function.get() else {
            return;
        };
        if !function.is_observed() {
            return;
        }
        let probe = Call {
            input: json!({ "db": event.db, "table": event.table, "op": event.op }),
            ..Call::default()
        };
        if !function.is_observed_for(&probe) {
            return;
        }
        let payload = match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(error = %e, "row-changed event did not serialize");
                return;
            }
        };
        let request = CallRequest {
            function_id: ROW_CHANGED_FN.to_string(),
            payload,
            action: None,
            timeout_ms: Some(self.call_timeout_ms),
        };
        if let Err(e) = self.iii.call(request).await {
            tracing::warn!(error = %e, "row-changed call failed");
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

    /// Validates `args` against the live config and, on a native database,
    /// installs capture for the table (once per `(db, table)`). Returns the
    /// `(db, table)` whose capture the observer holds, to release on removal.
    pub async fn admit(
        &self,
        args: &RowChangedArgs,
    ) -> Result<Option<Installed>, ObserveRejection> {
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
        let Some(table) = args.table.as_deref() else {
            return Err(ObserveRejection::new(
                codes::NATIVE_CAPTURE_REQUIRES_TABLE,
                format!(
                    "db `{}` uses `capture: native`, which requires args.table — per-table \
                     database triggers are what make external writes visible",
                    args.db
                ),
            ));
        };
        let key = (args.db.clone(), table.to_string());
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
            self.install(&args.db, table).await?;
        }
        *self
            .installed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_default() += 1;
        Ok(Some(key))
    }

    /// Records an observer admitted with [`admit`](Self::admit). An id
    /// accepted again releases what it held before.
    pub fn accept(&self, observer_id: &str, installed: Option<Installed>) {
        let replaced = self
            .observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(observer_id.to_string(), installed);
        if let Some(Some(old)) = replaced {
            self.release(&old);
        }
    }

    /// Forgets an observer and releases its hold on native capture.
    pub fn forget(&self, observer_id: &str) {
        let removed = self
            .observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(observer_id);
        if let Some(Some(key)) = removed {
            self.release(&key);
        }
    }

    fn release(&self, key: &Installed) {
        let mut installed = self.installed.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = installed.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                // ponytail: native-capture database triggers stay installed —
                // an orphan pg_notify per write statement is near-free and
                // idempotent to reinstall; add DDL teardown when someone
                // actually needs it.
                installed.remove(key);
            }
        }
    }

    /// The function's added hook: the args are canonical and name a
    /// configured database, and a native database's table has capture
    /// installed, before the observer becomes active. Its first call after
    /// that reaches it.
    pub async fn added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        let args = obs.args::<RowChangedArgs>()?; // db required: checked by the engine
        Self::check_canonical(&args)?;
        let installed = self.admit(&args).await?;
        tracing::info!(
            observer = %obs.id(),
            target = %obs.target(),
            db = %args.db,
            table = ?args.table,
            "row-changed observer added"
        );
        self.accept(obs.id(), installed);
        Ok(())
    }

    /// The function's removed hook.
    pub fn removed(&self, obs: &Observer) {
        self.forget(obs.id());
        tracing::info!(observer = %obs.id(), "row-changed observer removed");
    }

    /// After a config reload: late-rejects every observer the reloaded config
    /// cannot serve (its database is gone, or is native while the observer
    /// names no table). The rejection removes it, which runs the removed
    /// hook.
    pub async fn revalidate(&self) {
        let Some(function) = self.function.get() else {
            return;
        };
        let live = self.config.read().await;
        for obs in function.observers() {
            let Ok(args) = obs.args::<RowChangedArgs>() else {
                continue;
            };
            match live.databases.get(&args.db) {
                None => obs.reject(
                    codes::UNKNOWN_DB,
                    format!("db `{}` was removed from the configuration", args.db),
                ),
                Some(db) if db.capture == CaptureMode::Native && args.table.is_none() => obs
                    .reject(
                        codes::NATIVE_CAPTURE_REQUIRES_TABLE,
                        format!(
                            "db `{}` uses `capture: native`, which requires args.table",
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
    use crate::row_changes::sql::Op;

    /// Whether an observer with `args` wants a call with `input`: they share
    /// a key, as the SDK's index decides it.
    fn wants(args: Value, input: Value) -> bool {
        let filed = observer_keys(&args);
        call_keys(&input).iter().any(|key| filed.contains(key))
    }

    fn change(db: &str, table: Option<&str>, op: Op) -> Value {
        serde_json::to_value(RowChangedEvent::from_raw(db, table, op, 1, None, false)).unwrap()
    }

    #[test]
    fn keys_match_db_table_and_ops() {
        let orders = json!({ "db": "primary", "table": "orders" });
        let everything = json!({ "db": "primary" });
        let deletes = json!({ "db": "primary", "table": "orders", "ops": ["delete"] });
        let other_db = json!({ "db": "analytics" });

        let insert = change("primary", Some("orders"), Op::Insert);
        assert!(wants(orders.clone(), insert.clone()));
        assert!(wants(everything.clone(), insert.clone()));
        assert!(!wants(deletes.clone(), insert.clone()));
        assert!(!wants(other_db.clone(), insert));

        let delete = change("primary", Some("orders"), Op::Delete);
        assert!(wants(deletes.clone(), delete.clone()));
        assert!(wants(orders.clone(), delete));

        // An observer on `orders` never receives `payments` events.
        let payments = change("primary", Some("payments"), Op::Update);
        assert!(!wants(orders.clone(), payments.clone()));
        assert!(wants(everything.clone(), payments));

        // An unclassifiable table reaches only the observers of every table.
        let unknown = change("primary", None, Op::Other);
        assert!(!wants(orders, unknown.clone()));
        assert!(wants(everything, unknown));

        assert!(wants(
            other_db,
            change("analytics", Some("orders"), Op::Delete)
        ));
    }

    #[test]
    fn an_empty_ops_list_hears_nothing() {
        assert!(observer_keys(&json!({ "db": "primary", "ops": [] })).is_empty());
        assert!(!wants(
            json!({ "db": "primary", "ops": [] }),
            change("primary", Some("orders"), Op::Insert)
        ));
    }

    #[test]
    fn keys_do_not_collide_across_names_or_with_wildcards() {
        // A table named like a separator or like the wildcard is its own key.
        assert!(!wants(
            json!({ "db": "a", "table": "b|c" }),
            change("a|b", Some("c"), Op::Insert)
        ));
        assert!(!wants(
            json!({ "db": "primary", "table": "null" }),
            change("primary", None, Op::Insert)
        ));
        assert_eq!(
            call_keys(&change("primary", Some("orders"), Op::Insert)),
            [
                r#"["primary","orders","insert"]"#,
                r#"["primary","orders",null]"#,
                r#"["primary",null,"insert"]"#,
                r#"["primary",null,null]"#,
            ]
        );
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

    #[test]
    fn accepting_an_id_again_releases_its_previous_hold() {
        let capture = capture_with("databases:\n  p:\n    url: \"sqlite::memory:\"\n");
        let key = ("p".to_string(), "orders".to_string());
        *capture
            .installed
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default() += 1;
        capture.accept("o1", Some(key.clone()));
        capture.accept("o1", None);
        assert_eq!(capture.observer_count(), 1);
        assert!(capture.installed.lock().unwrap().get(&key).is_none());
        capture.forget("o1");
        assert_eq!(capture.observer_count(), 0);
        capture.forget("nope");
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
            capture.admit(&unknown).await.unwrap_err().code,
            codes::UNKNOWN_DB
        );
        let tableless = RowChangedArgs {
            db: "p".into(),
            table: None,
            ops: None,
        };
        assert_eq!(
            capture.admit(&tableless).await.unwrap_err().code,
            codes::NATIVE_CAPTURE_REQUIRES_TABLE
        );
        // With a table but no live pool the DDL cannot land: refused, not
        // silently listening to nothing.
        let with_table = RowChangedArgs {
            db: "p".into(),
            table: Some("orders".into()),
            ops: None,
        };
        let err = capture.admit(&with_table).await.unwrap_err();
        assert_eq!(err.code, codes::CAPTURE_INSTALL_FAILED);
        assert!(err.message.contains("no pool"), "{}", err.message);
    }

    /// The direct refcount test: two observers on one native table share one
    /// install; removing one leaves the capture in place for the other.
    #[tokio::test]
    async fn native_capture_is_installed_once_and_outlives_one_holder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("watched.db");
        rusqlite::Connection::open(&file)
            .unwrap()
            .execute_batch("CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)")
            .unwrap();
        let cfg = WorkerConfig::from_yaml(&format!(
            "databases:\n  watched:\n    url: \"sqlite:{}\"\n    capture: native\n",
            file.display()
        ))
        .unwrap();
        let pools = crate::configuration::build_pools(&cfg).await.unwrap();
        let capture = NativeCapture::new(
            Arc::new(IIIClient::new("ws://127.0.0.1:9")),
            Arc::new(tokio::sync::RwLock::new(cfg)),
            Arc::new(tokio::sync::RwLock::new(pools)),
            100,
        );
        let capture_triggers = || -> i64 {
            rusqlite::Connection::open(&file)
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'orders'",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        };
        let holders = |capture: &NativeCapture| {
            capture
                .installed
                .lock()
                .unwrap()
                .get(&("watched".to_string(), "orders".to_string()))
                .copied()
        };
        let args = RowChangedArgs {
            db: "watched".into(),
            table: Some("orders".into()),
            ops: None,
        };
        for id in ["first", "second"] {
            let installed = capture.admit(&args).await.unwrap();
            assert_eq!(
                installed,
                Some(("watched".to_string(), "orders".to_string()))
            );
            capture.accept(id, installed);
        }
        assert_eq!(holders(&capture), Some(2));
        assert_eq!(capture.observer_count(), 2);
        assert_eq!(capture_triggers(), 3, "one insert/update/delete set");

        capture.forget("first");
        assert_eq!(holders(&capture), Some(1));
        assert_eq!(capture_triggers(), 3);

        capture.forget("second");
        assert_eq!(holders(&capture), None);
        // The DDL stays when the last holder leaves; installing again is a no-op.
        assert_eq!(capture_triggers(), 3);
        capture.admit(&args).await.unwrap();
        assert_eq!(holders(&capture), Some(1));
        assert_eq!(capture_triggers(), 3);
    }

    #[test]
    fn pg_install_hint_matches_the_failure_shape() {
        let hint = pg_install_hint(
            r#"relation "III_CAPTURE_TEST" does not exist"#,
            "III_CAPTURE_TEST",
        );
        assert!(hint.contains("case-sensitive"), "{hint}");
        assert!(!hint.contains("TRIGGER privilege"), "{hint}");
        let hint = pg_install_hint("permission denied for table orders", "orders");
        assert!(hint.contains("TRIGGER privilege"), "{hint}");
        assert_eq!(pg_install_hint("connection reset by peer", "orders"), "");
    }
}
