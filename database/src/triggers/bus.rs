//! The `database::row-changed` binding table and fan-out.
//!
//! The worker owns this trigger type, so it owns the bindings: the engine hands
//! each `register_trigger` to the handler, which files it here, and every
//! successful mutation looks up the matching bindings and calls their functions
//! directly (`harness/src/events.rs` does the same for turn events).
//!
//! Two rules shape it:
//!
//! * **Emit after the write is durable, never before.** A statement inside a
//!   transaction has not happened until the commit does, so interactive
//!   transactions buffer here and flush on commit; a rollback drops the buffer.
//!   Announcing a row that then rolls back is worse than announcing nothing.
//! * **Only what this worker wrote.** This is not change data capture. A
//!   mutation applied by psql, another worker, or a trigger inside the database
//!   is invisible here, by construction.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sql::{self, Op};

/// The trigger type this worker registers.
pub const ROW_CHANGED_TYPE: &str = "database::row-changed";

/// Per-binding config: which database, and optionally which table.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowChangedConfig {
    /// Database handle, as named in the worker's config. Required — a binding
    /// that watched every database would fire for traffic its owner never
    /// asked about.
    pub db: String,
    /// Table filter. Matched case-insensitively and ignoring a schema
    /// qualifier. Omit to hear every table in the database.
    #[serde(default)]
    pub table: Option<String>,
    /// Operation filter. Omit to hear every operation.
    #[serde(default)]
    pub ops: Option<Vec<Op>>,
}

/// What a subscriber receives.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RowChangedEvent {
    pub db: String,
    /// `null` when the statement was recognisably a write but its table could
    /// not be read out of the SQL (a CTE-wrapped write, for example).
    pub table: Option<String>,
    pub op: Op,
    pub affected_rows: u64,
    /// The `RETURNING` rows, when the caller asked for them. Absent otherwise —
    /// this trigger reports that a change happened, not the new row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returning: Option<Vec<serde_json::Map<String, Value>>>,
    /// Epoch millis at emit time.
    pub at: i64,
}

#[derive(Clone, Debug)]
struct Subscriber {
    instance_id: String,
    function_id: String,
    /// The metadata the engine stored on this binding, handed back verbatim at
    /// fire time. Load-bearing, not decoration: a harness binding carries its
    /// `__binding` pointer here, and a dispatch without it reaches the harness
    /// with nothing to resolve — the event is simply dropped, silently.
    metadata: Option<Value>,
    config: SubscriberFilter,
}

#[derive(Clone, Debug)]
struct SubscriberFilter {
    db: String,
    table: Option<String>,
    ops: Option<Vec<Op>>,
}

impl SubscriberFilter {
    fn matches(&self, db: &str, table: Option<&str>, op: Op) -> bool {
        if self.db != db {
            return false;
        }
        let table_matches = match (&self.table, table) {
            (None, _) => true,
            // A binding that named a table never matches an event whose table
            // could not be determined: silence beats a wrong match.
            (Some(_), None) => false,
            (Some(want), Some(got)) => sql::same_table(want, got),
        };
        table_matches && self.ops.as_ref().is_none_or(|ops| ops.contains(&op))
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

#[derive(Default)]
struct Inner {
    subscribers: Vec<Subscriber>,
    /// transaction id → changes not yet committed.
    pending: HashMap<String, Vec<Pending>>,
}

/// Everything the bus does that does not need the engine: bookkeeping and
/// matching. Kept separate so the buffer semantics — the part with real
/// consequences if wrong — are testable without a live client.
impl Inner {
    fn register(
        &mut self,
        instance_id: String,
        function_id: String,
        metadata: Option<Value>,
        cfg: RowChangedConfig,
    ) {
        // Idempotent on the engine's instance id: a re-registration replaces
        // rather than doubles.
        self.subscribers.retain(|s| s.instance_id != instance_id);
        self.subscribers.push(Subscriber {
            instance_id,
            function_id,
            metadata,
            config: SubscriberFilter {
                db: cfg.db,
                table: cfg.table,
                ops: cfg.ops,
            },
        });
    }

    fn unregister(&mut self, instance_id: &str) {
        self.subscribers.retain(|s| s.instance_id != instance_id);
    }

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

    /// Matching subscribers as (function id, stored metadata) pairs.
    fn targets_for(&self, db: &str, table: Option<&str>, op: Op) -> Vec<(String, Option<Value>)> {
        self.subscribers
            .iter()
            .filter(|s| s.config.matches(db, table, op))
            .map(|s| (s.function_id.clone(), s.metadata.clone()))
            .collect()
    }
}

pub struct RowChangeBus {
    iii: Arc<IIIClient>,
    dispatch_timeout_ms: u64,
    inner: Mutex<Inner>,
}

impl RowChangeBus {
    pub fn new(iii: Arc<IIIClient>, dispatch_timeout_ms: u64) -> Self {
        Self {
            iii,
            dispatch_timeout_ms,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn register(
        &self,
        instance_id: String,
        function_id: String,
        metadata: Option<Value>,
        cfg: RowChangedConfig,
    ) {
        self.lock()
            .register(instance_id, function_id, metadata, cfg);
    }

    pub fn unregister(&self, instance_id: &str) {
        self.lock().unregister(instance_id);
    }

    pub fn subscriber_count(&self) -> usize {
        self.lock().subscribers.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self, transaction_id: &str) -> usize {
        self.lock().pending.get(transaction_id).map_or(0, Vec::len)
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
        if affected_rows == 0 {
            return;
        }
        let Some(mutation) = sql::classify(sql) else {
            return;
        };
        let event = RowChangedEvent {
            db: db.to_string(),
            table: mutation.table.clone(),
            op: mutation.op,
            affected_rows,
            returning: returning.map(|r| r.to_vec()).filter(|r| !r.is_empty()),
            at: now_ms(),
        };
        self.fan_out(event).await;
    }

    /// Buffer a change made inside an interactive transaction. Nothing is
    /// announced until [`commit`] — the row does not exist for anyone else yet.
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
            self.fan_out(RowChangedEvent {
                db: p.db,
                table: p.table,
                op: p.op,
                affected_rows: p.affected_rows,
                returning: p.returning,
                at: now_ms(),
            })
            .await;
        }
    }

    /// Drop a rolled-back transaction's buffer. Also the timeout path: a
    /// transaction the watcher auto-rolls back must not announce anything.
    pub fn rollback(&self, transaction_id: &str) {
        self.lock().pending.remove(transaction_id);
    }

    async fn fan_out(&self, event: RowChangedEvent) {
        let targets = self
            .lock()
            .targets_for(&event.db, event.table.as_deref(), event.op);
        if targets.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "row-changed event serialize failed");
                return;
            }
        };
        for (function_id, metadata) in targets {
            let request = TriggerRequest {
                function_id: function_id.clone(),
                payload: payload.clone(),
                action: Some(iii_sdk::TriggerAction::Void),
                timeout_ms: Some(self.dispatch_timeout_ms),
            };
            // `Void` enqueues and returns without awaiting subscriber
            // execution. The stored metadata rides along — the target may
            // need it to know which binding this is.
            let res = match metadata {
                Some(m) => self.iii.trigger(request.metadata(m)).await,
                None => self.iii.trigger(request).await,
            };
            if let Err(e) = res {
                tracing::warn!(
                    function_id = %function_id,
                    error = %e,
                    "row-changed dispatch failed"
                );
            }
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(db: &str, table: Option<&str>) -> SubscriberFilter {
        SubscriberFilter {
            db: db.into(),
            table: table.map(str::to_string),
            ops: None,
        }
    }

    #[test]
    fn a_table_filter_matches_case_and_schema_insensitively() {
        let f = filter("primary", Some("orders"));
        assert!(f.matches("primary", Some("orders"), Op::Insert));
        assert!(f.matches("primary", Some("ORDERS"), Op::Insert));
        assert!(f.matches("primary", Some("public.orders"), Op::Insert));
        assert!(!f.matches("primary", Some("order_items"), Op::Insert));
        assert!(!f.matches("analytics", Some("orders"), Op::Insert));
    }

    #[test]
    fn a_db_only_filter_hears_every_table() {
        let f = filter("primary", None);
        assert!(f.matches("primary", Some("orders"), Op::Insert));
        assert!(f.matches("primary", Some("payments"), Op::Update));
        // Including the ones whose table could not be classified.
        assert!(f.matches("primary", None, Op::Other));
        assert!(!f.matches("other", Some("orders"), Op::Delete));
    }

    #[test]
    fn a_named_table_never_matches_an_unknown_one() {
        // Silence beats a wrong match: a CTE-wrapped write reports no table,
        // and a binding watching `orders` must not claim it.
        let f = filter("primary", Some("orders"));
        assert!(!f.matches("primary", None, Op::Other));
    }

    #[test]
    fn an_op_filter_matches_only_selected_operations() {
        let f = SubscriberFilter {
            db: "primary".into(),
            table: Some("orders".into()),
            ops: Some(vec![Op::Insert, Op::Delete]),
        };
        assert!(f.matches("primary", Some("orders"), Op::Insert));
        assert!(f.matches("primary", Some("orders"), Op::Delete));
        assert!(!f.matches("primary", Some("orders"), Op::Update));
        assert!(!f.matches("primary", Some("orders"), Op::Other));
    }

    #[test]
    fn staged_changes_survive_until_commit_and_die_on_rollback() {
        let mut inner = Inner::default();
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
        let mut inner = Inner::default();
        inner.stage("tx1", "primary", "SELECT * FROM a", 0, None);
        inner.stage("tx1", "primary", "CREATE TABLE b (id INT)", 0, None);
        inner.stage("tx1", "primary", "UPDATE a SET n = 1", 0, None);
        assert!(!inner.pending.contains_key("tx1"));
    }

    #[test]
    fn staged_changes_keep_returning_rows() {
        let mut inner = Inner::default();
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
    fn config_rejects_unknown_filter_keys() {
        let err = serde_json::from_value::<RowChangedConfig>(serde_json::json!({
            "db": "primary",
            "tabl": "orders"
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn unknown_table_serializes_as_null() {
        let value = serde_json::to_value(RowChangedEvent {
            db: "primary".into(),
            table: None,
            op: Op::Other,
            affected_rows: 1,
            returning: None,
            at: 0,
        })
        .unwrap();
        assert_eq!(value["table"], serde_json::Value::Null);
    }

    #[test]
    fn registration_is_idempotent_on_the_instance_id() {
        let mut inner = Inner::default();
        let cfg = || RowChangedConfig {
            db: "primary".into(),
            table: None,
            ops: None,
        };
        inner.register("i1".into(), "app::on-change".into(), None, cfg());
        inner.register("i1".into(), "app::on-change".into(), None, cfg());
        assert_eq!(inner.subscribers.len(), 1);
        inner.register("i2".into(), "app::other".into(), None, cfg());
        assert_eq!(inner.subscribers.len(), 2);
        inner.unregister("i1");
        assert_eq!(inner.subscribers.len(), 1);
        // Unregistering something unknown is a no-op, not a panic.
        inner.unregister("nope");
        assert_eq!(inner.subscribers.len(), 1);
    }

    #[test]
    fn fan_out_targets_only_matching_subscribers() {
        let mut inner = Inner::default();
        inner.register(
            "i1".into(),
            "app::orders".into(),
            None,
            RowChangedConfig {
                db: "primary".into(),
                table: Some("orders".into()),
                ops: None,
            },
        );
        inner.register(
            "i2".into(),
            "app::everything".into(),
            None,
            RowChangedConfig {
                db: "primary".into(),
                table: None,
                ops: None,
            },
        );
        inner.register(
            "i3".into(),
            "app::other-db".into(),
            None,
            RowChangedConfig {
                db: "analytics".into(),
                table: None,
                ops: None,
            },
        );

        let names = |db: &str, t: Option<&str>, op: Op| -> Vec<String> {
            inner
                .targets_for(db, t, op)
                .into_iter()
                .map(|(f, _)| f)
                .collect()
        };
        assert_eq!(
            names("primary", Some("orders"), Op::Insert),
            vec!["app::orders".to_string(), "app::everything".to_string()]
        );
        assert_eq!(
            names("primary", Some("payments"), Op::Update),
            vec!["app::everything".to_string()]
        );
        // An unclassifiable table reaches only the db-wide watcher.
        assert_eq!(
            names("primary", None, Op::Other),
            vec!["app::everything".to_string()]
        );
        assert_eq!(
            names("analytics", Some("orders"), Op::Delete),
            vec!["app::other-db".to_string()]
        );
    }

    #[test]
    fn stored_metadata_rides_along_to_the_target() {
        // The bug this pins: dispatching without the binding's metadata
        // reaches the harness with no `__binding` to resolve, and the event is
        // dropped silently — the write happened, nobody heard it.
        let mut inner = Inner::default();
        inner.register(
            "i1".into(),
            "harness::trigger::deliver".into(),
            Some(serde_json::json!({ "__binding": "sub_abc" })),
            RowChangedConfig {
                db: "primary".into(),
                table: None,
                ops: None,
            },
        );
        let targets = inner.targets_for("primary", Some("orders"), Op::Insert);
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].1.as_ref().and_then(|m| m.get("__binding")),
            Some(&serde_json::json!("sub_abc"))
        );
    }
}
