//! The `database::on-row-changed` event, the transaction buffer, and the emit
//! path.
//!
//! The worker announces its own writes through `database::on-row-changed`,
//! an owner-delivered function (`capture.rs`): each event is routed to the
//! observers whose args name its db, table and op. This module decides WHEN
//! an event exists:
//!
//! * **Emit after the write is durable, never before.** A statement inside a
//!   transaction has not happened until the commit does, so interactive
//!   transactions buffer here and flush on commit; a rollback drops the buffer.
//!   Announcing a row that then rolls back is worse than announcing nothing.
//! * **Only what this worker wrote — on a `capture: statements` database.** The
//!   classification path is not change data capture: a mutation applied by
//!   psql, another worker, or a trigger inside the database is invisible to it.
//!   `capture: native` databases hear every client instead (`native.rs`,
//!   `sqlite_watch.rs`, `mysql_binlog.rs`) and hand their events to
//!   [`RowChangeBus::emit_event`] already shaped.
//! * **Nothing is serialized for nobody.** With no active observer (and no
//!   binding through the pre-0.25 fallback) an event costs one atomic load.
//!
//! The event never invents row data. On the statements path `returning` is the
//! writer's own `RETURNING` projection, forwarded verbatim; on the native path
//! it is the primary-key columns of the changed rows, capped at [`KEY_CAP`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::capture::NativeCapture;
use super::sql::{self, Op};

/// The trigger type this worker registered before 0.25. Kept as the
/// function's `legacy_trigger_type`, and registered as a trigger type only
/// against an engine without observe (see `legacy.rs`).
pub const ROW_CHANGED_TYPE: &str = "database::row-changed";

/// How many changed-row keys one `capture: native` event carries at most.
/// Shared by all three native producers so a listener sees one contract.
/// Sized for the postgres NOTIFY payload limit (8000 bytes): a hundred
/// single-column integer keys is ~2 KB, a hundred two-column uuid keys ~6 KB;
/// the postgres trigger additionally halves the count until the payload fits.
pub(crate) const KEY_CAP: usize = 100;

/// An observer's args: which database, and optionally which table and
/// operations. Also the shape of the old trigger config.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowChangedArgs {
    /// Database handle, as named in the worker's config.
    // Required: an observer that watched every database would fire for
    // traffic its owner never asked about.
    pub db: String,
    /// Table, canonical: the bare name, unquoted and lowercased (`orders`,
    /// not `public.Orders`). Optional on a `capture: statements` database
    /// (omit to hear every table); required on a `capture: native` one,
    /// where per-table database triggers are what make other clients'
    /// writes visible.
    #[serde(default)]
    pub table: Option<String>,
    /// Operations. Omit to hear every operation.
    #[serde(default)]
    pub ops: Option<Vec<Op>>,
}

/// One committed write statement: the input of `database::on-row-changed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RowChangedEvent {
    pub db: String,
    /// The table's schema, lowercased, when the SQL or the driver named one;
    /// `null` otherwise.
    pub schema: Option<String>,
    /// Canonical table name: bare, unquoted, lowercased. `null` when the
    /// statement was recognisably a write but its table could not be read out
    /// of the SQL (a CTE-wrapped write, for example): such an event reaches
    /// only observers whose args name no table. Always set on a
    /// `capture: native` database.
    pub table: Option<String>,
    pub op: Op,
    /// Rows the statement changed. Exact even when `returning` is capped.
    pub affected_rows: u64,
    /// Which rows changed, when that is known. On a `capture: statements`
    /// database this is the writer's own `RETURNING` projection — whatever
    /// columns the writer's SQL named, nothing more — so it is absent when
    /// the writer's SQL had no `RETURNING` clause. On a `capture: native`
    /// database it is the primary-key columns of each changed row (UPDATE
    /// reports the new key), capped at 100 rows per event; absent for a
    /// table without a primary key. Values are encoded like `database::query`
    /// returns them for the same column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returning: Option<Vec<serde_json::Map<String, Value>>>,
    /// True when `returning` holds fewer keys than `affected_rows` because the
    /// per-event cap dropped the rest (native capture only). Omitted when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// Epoch millis at emit time.
    pub at: i64,
}

/// What a `database::row-changed` trigger received before 0.25, and what a
/// legacy binding (shim observer or fallback trigger) still receives: no
/// `schema`, and the table schema-qualified when a schema is known
/// (`public.orders` from postgres native capture, as before).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct LegacyRowChangedEvent {
    pub db: String,
    pub table: Option<String>,
    pub op: Op,
    pub affected_rows: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returning: Option<Vec<serde_json::Map<String, Value>>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    pub at: i64,
}

impl RowChangedEvent {
    /// An event for a table reference as the SQL or the driver spelled it
    /// (`orders`, `"Orders"`, `public.orders`): split and canonicalized here.
    pub(crate) fn from_raw(
        db: &str,
        raw_table: Option<&str>,
        op: Op,
        affected_rows: u64,
        returning: Option<Vec<serde_json::Map<String, Value>>>,
        truncated: bool,
    ) -> Self {
        let (schema, table) = match raw_table.map(sql::split_table) {
            Some((schema, table)) => (schema, Some(table)),
            None => (None, None),
        };
        Self {
            db: db.to_string(),
            schema,
            table,
            op,
            affected_rows,
            returning: returning.filter(|r| !r.is_empty()),
            truncated,
            at: now_ms(),
        }
    }

    /// The same event canonicalized: native producers fill `table` with what
    /// the database reported (postgres: `schema.table`) and no `schema`.
    pub(crate) fn canonical(self) -> Self {
        if self.schema.is_some() {
            return self;
        }
        let Some(raw) = self.table.as_deref() else {
            return self;
        };
        let (schema, table) = sql::split_table(raw);
        Self {
            schema,
            table: Some(table),
            ..self
        }
    }

    /// The pre-0.25 shape of this event (see [`LegacyRowChangedEvent`]).
    pub fn legacy_view(&self) -> LegacyRowChangedEvent {
        LegacyRowChangedEvent {
            db: self.db.clone(),
            table: match (&self.schema, &self.table) {
                (Some(schema), Some(table)) => Some(format!("{schema}.{table}")),
                (_, table) => table.clone(),
            },
            op: self.op,
            affected_rows: self.affected_rows,
            returning: self.returning.clone(),
            truncated: self.truncated,
            at: self.at,
        }
    }
}

/// One buffered change, waiting for its transaction to commit.
#[derive(Clone, Debug)]
struct Pending {
    db: String,
    table: Option<String>,
    op: Op,
    affected_rows: u64,
    returning: Option<Vec<serde_json::Map<String, Value>>>,
}

/// The transaction buffer. Kept separate from the client so the buffer
/// semantics — the part with real consequences if wrong — are testable
/// without a live engine.
#[derive(Default)]
struct Buffer {
    /// transaction id → changes not yet committed.
    pending: HashMap<String, Vec<Pending>>,
}

impl Buffer {
    fn stage(
        &mut self,
        transaction_id: &str,
        db: &str,
        sql: &str,
        affected_rows: u64,
        returning: Option<&[serde_json::Map<String, Value>]>,
    ) {
        if affected_rows == 0 {
            return;
        }
        let Some(mutation) = sql::classify(sql) else {
            return;
        };
        self.pending
            .entry(transaction_id.to_string())
            .or_default()
            .push(Pending {
                db: db.to_string(),
                table: mutation.table,
                op: mutation.op,
                affected_rows,
                returning: returning.map(<[_]>::to_vec).filter(|r| !r.is_empty()),
            });
    }

    fn take_pending(&mut self, transaction_id: &str) -> Vec<Pending> {
        self.pending.remove(transaction_id).unwrap_or_default()
    }
}

pub struct RowChangeBus {
    iii: Arc<IIIClient>,
    capture: Arc<NativeCapture>,
    inner: Mutex<Buffer>,
}

impl RowChangeBus {
    /// The engine client this bus dispatches through. Handlers reuse it to
    /// call sibling workers (history goes to `state::*`) without threading a
    /// second client through `AppState`.
    pub fn client(&self) -> &Arc<IIIClient> {
        &self.iii
    }

    /// A bus over its own, empty capture: staging and commit work, events go
    /// nowhere. For tests that exercise the buffer.
    pub fn new(iii: Arc<IIIClient>, dispatch_timeout_ms: u64) -> Self {
        let capture = Arc::new(NativeCapture::detached(iii.clone(), dispatch_timeout_ms));
        Self::with_capture(iii, capture)
    }

    /// A bus that hands its events to `capture`, which routes them.
    pub fn with_capture(iii: Arc<IIIClient>, capture: Arc<NativeCapture>) -> Self {
        Self {
            iii,
            capture,
            inner: Mutex::new(Buffer::default()),
        }
    }

    pub fn capture(&self) -> &Arc<NativeCapture> {
        &self.capture
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self, transaction_id: &str) -> usize {
        self.lock().pending.get(transaction_id).map_or(0, Vec::len)
    }

    /// Announce an already-shaped event — the native capture path, where the
    /// database told us table/op/count and there is no SQL to classify.
    pub async fn emit_event(&self, event: RowChangedEvent) {
        self.fan_out(event.canonical()).await;
    }

    /// Announce a committed change. `sql` is classified here so callers stay
    /// one line; a statement that changes no rows fires nothing.
    pub async fn emit(
        &self,
        db: &str,
        sql: &str,
        affected_rows: u64,
        returning: Option<&[serde_json::Map<String, Value>]>,
    ) {
        if affected_rows == 0 || !self.capture.is_listened() {
            return;
        }
        let Some(mutation) = sql::classify(sql) else {
            return;
        };
        let event = RowChangedEvent::from_raw(
            db,
            mutation.table.as_deref(),
            mutation.op,
            affected_rows,
            returning.map(<[_]>::to_vec),
            false,
        );
        self.fan_out(event).await;
    }

    /// Buffer a change made inside an interactive transaction. Nothing is
    /// announced until [`commit`] — the row does not exist for anyone else yet.
    /// Staging stays unconditional: an observer that becomes active between a
    /// statement and its commit is owed the event.
    pub fn stage(
        &self,
        transaction_id: &str,
        db: &str,
        sql: &str,
        affected_rows: u64,
        returning: Option<&[serde_json::Map<String, Value>]>,
    ) {
        self.lock()
            .stage(transaction_id, db, sql, affected_rows, returning);
    }

    /// Flush a committed transaction's buffered changes, in statement order.
    pub async fn commit(&self, transaction_id: &str) {
        let staged = self.lock().take_pending(transaction_id);
        for p in staged {
            self.fan_out(RowChangedEvent::from_raw(
                &p.db,
                p.table.as_deref(),
                p.op,
                p.affected_rows,
                p.returning,
                false,
            ))
            .await;
        }
    }

    /// Drop a rolled-back transaction's buffer. Also the timeout path: a
    /// transaction the watcher auto-rolls back must not announce anything.
    pub fn rollback(&self, transaction_id: &str) {
        self.lock().pending.remove(transaction_id);
    }

    async fn fan_out(&self, event: RowChangedEvent) {
        // Nobody observing: no serialization, no call, no frames.
        if !self.capture.is_listened() {
            return;
        }
        // Routed in-process, from the invocation that made the write (the
        // `database::execute` / `commitTransaction` handler, or a native
        // listener task): the SDK checks each routed observer's filters and
        // reports only matches, with that invocation as the event's caller.
        // Awaited, so a transaction's statements are reported in commit order.
        self.capture.route(&event).await;
    }
}

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_changes_survive_until_commit_and_die_on_rollback() {
        let mut inner = Buffer::default();
        inner.stage("tx1", "primary", "INSERT INTO a (n) VALUES (1)", 1, None);
        inner.stage("tx1", "primary", "UPDATE a SET n = 2", 1, None);
        inner.stage("tx2", "primary", "DELETE FROM b", 3, None);
        assert_eq!(inner.pending.get("tx1").map(Vec::len), Some(2));

        // A rollback drops only its own buffer — announcing a row that rolled
        // back is the failure this buffering exists to prevent.
        inner.pending.remove("tx1");
        assert!(!inner.pending.contains_key("tx1"));
        assert_eq!(inner.pending.get("tx2").map(Vec::len), Some(1));

        // Taking a committed buffer drains it rather than leaking the entry.
        let drained = inner.take_pending("tx2");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].op, Op::Delete);
        assert!(!inner.pending.contains_key("tx2"));
        // Committing an unknown transaction yields nothing, not a panic.
        assert!(inner.take_pending("never-existed").is_empty());
    }

    #[test]
    fn staging_ignores_statements_that_change_no_rows() {
        let mut inner = Buffer::default();
        inner.stage("tx1", "primary", "SELECT * FROM a", 0, None);
        inner.stage("tx1", "primary", "CREATE TABLE b (id INT)", 0, None);
        inner.stage("tx1", "primary", "UPDATE a SET n = 1", 0, None);
        assert!(!inner.pending.contains_key("tx1"));
    }

    #[test]
    fn staged_changes_keep_returning_rows() {
        let mut inner = Buffer::default();
        let rows = vec![serde_json::json!({ "id": 7 }).as_object().unwrap().clone()];
        inner.stage(
            "tx1",
            "primary",
            "INSERT INTO a (id) VALUES (7) RETURNING id",
            1,
            Some(&rows),
        );
        assert_eq!(
            inner.pending["tx1"][0].returning.as_ref().unwrap()[0]["id"],
            7
        );
    }

    #[test]
    fn args_reject_unknown_keys() {
        let err = serde_json::from_value::<RowChangedArgs>(serde_json::json!({
            "db": "primary",
            "tabl": "orders"
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn events_carry_canonical_schema_and_table() {
        let event = RowChangedEvent::from_raw(
            "primary",
            Some("public.\"Orders\""),
            Op::Insert,
            1,
            None,
            false,
        );
        assert_eq!(event.schema.as_deref(), Some("public"));
        assert_eq!(event.table.as_deref(), Some("orders"));
        let bare = RowChangedEvent::from_raw("primary", Some("ORDERS"), Op::Insert, 1, None, false);
        assert_eq!(bare.schema, None);
        assert_eq!(bare.table.as_deref(), Some("orders"));
        // A native postgres event names `schema.table`; canonical() splits it.
        let native = RowChangedEvent {
            schema: None,
            table: Some("public.orders".into()),
            ..bare.clone()
        }
        .canonical();
        assert_eq!(native.schema.as_deref(), Some("public"));
        assert_eq!(native.table.as_deref(), Some("orders"));
        // Legacy bindings keep the schema-qualified name they got before.
        assert_eq!(native.legacy_view().table.as_deref(), Some("public.orders"));
        assert_eq!(bare.legacy_view().table.as_deref(), Some("orders"));
    }

    #[test]
    fn unknown_table_serializes_as_null() {
        let value = serde_json::to_value(RowChangedEvent::from_raw(
            "primary",
            None,
            Op::Other,
            1,
            None,
            false,
        ))
        .unwrap();
        assert_eq!(value["table"], serde_json::Value::Null);
        assert_eq!(value["schema"], serde_json::Value::Null);
    }

    /// The wire shape a listener sees must not grow keys for events that
    /// carry no row identity: no `returning` key, no `truncated` key. The
    /// flag appears only when it is true. The legacy view has no `schema`.
    #[test]
    fn events_without_keys_omit_returning_and_truncated() {
        let plain =
            RowChangedEvent::from_raw("primary", Some("orders"), Op::Update, 3, None, false);
        let value = serde_json::to_value(&plain).unwrap();
        assert!(value.get("returning").is_none(), "{value}");
        assert!(value.get("truncated").is_none(), "{value}");
        let legacy = serde_json::to_value(plain.legacy_view()).unwrap();
        assert!(legacy.get("schema").is_none(), "{legacy}");
        assert!(legacy.get("returning").is_none(), "{legacy}");

        let capped = RowChangedEvent::from_raw(
            "primary",
            Some("orders"),
            Op::Insert,
            150,
            Some(vec![serde_json::json!({ "id": 1 })
                .as_object()
                .unwrap()
                .clone()]),
            true,
        );
        let value = serde_json::to_value(&capped).unwrap();
        assert_eq!(value["truncated"], serde_json::json!(true));
        assert_eq!(value["returning"][0]["id"], 1);
    }
}
