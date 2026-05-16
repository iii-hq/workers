//! Delivery-tracking handlers.
//!
//! Three RPCs make up the gate's read/drain surface:
//!
//! - [`handle_list_pending`] — UI-facing list of in-flight prompts.
//!   Applies lazy timeout flip on read: a Pending row past `expires_at`
//!   flips to `Done(TimedOut)` and disappears from the list.
//! - [`handle_consume`] — atomic drain: returns Done rows and deletes
//!   them in the same call. Defensive `session_id` filter; cap +
//!   `omitted` counter; sort by `resolved_at` for deterministic LLM
//!   replay across multi-row consumes (cascade case).
//! - [`handle_sweep_session`] — force-cancellation for `run::stop`:
//!   flips every Pending and InFlight row to `Done(TimedOut)`.

use serde_json::{json, Value};

use crate::record::{Outcome, Record, Status};
use crate::state::StateBus;
use crate::wire::pending_key;

/// Default per-call cap on `handle_consume`. Bounds the response size —
/// `Outcome::Executed.result` can carry MB-sized stdout/stderr payloads,
/// and we don't want one consume to blow the trigger wire or the next
/// LLM turn.
pub const CONSUME_DEFAULT_LIMIT: usize = 50;

/// List Pending rows for a session. Applies lazy timeout flip on read —
/// expired Pending rows are persisted as `Done(TimedOut)` and dropped
/// from the response.
pub async fn handle_list_pending(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload.get("session_id").and_then(Value::as_str).unwrap_or("");
    if session_id.is_empty() {
        return json!({ "pending": [] });
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let prefix = format!("{session_id}/");
    let rows = bus.list_prefix(state_scope, &prefix).await;

    let mut pending = Vec::new();
    for raw in rows {
        let Some(record) = Record::from_value(raw) else { continue };
        if record.session_id != session_id { continue; }   // defensive
        // Lazy flip + persist; expired rows leave the Pending list.
        if let Some(flipped) = record.flipped_to_timed_out_if_expired(now_ms) {
            let key = pending_key(session_id, &flipped.function_call_id);
            let _ = bus.set(state_scope, &key, flipped.to_value()).await;
            continue;
        }
        if record.status == Status::Pending {
            pending.push(record.to_value());
        }
    }
    json!({ "pending": pending })
}

/// Atomic drain: returns Done rows for a session and deletes them in the
/// same call. Pending and InFlight rows stay in state. Pending rows past
/// `expires_at` are lazy-flipped to `Done(TimedOut)` and returned.
///
/// Three phases:
///   1. gather Done candidates (no state mutation);
///   2. sort by `resolved_at`, apply cap, report `omitted` count;
///   3. delete-and-return — only rows whose delete succeeded are returned,
///      so a partial failure leaves the row to be retried next consume.
///
/// Sort order matters when cascade auto-resolves multiple rows that all
/// surface to the same consume — `resolved_at` produces deterministic
/// LLM message order.
pub async fn handle_consume(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let session_id = payload.get("session_id").and_then(Value::as_str).unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id" });
    }
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(CONSUME_DEFAULT_LIMIT);

    let prefix = format!("{session_id}/");
    let rows = bus.list_prefix(state_scope, &prefix).await;

    // Phase 1: gather Done candidates without mutating state.
    let mut candidates: Vec<Record> = Vec::new();
    for raw in rows {
        let Some(record) = Record::from_value(raw) else { continue };
        // Defensive session_id filter: some state-bus backends ignore the
        // prefix arg and return every row in the scope. Drop anything not
        // stamped with the session_id we're consuming for — otherwise a
        // faulty backend could cross-session delete.
        if record.session_id != session_id { continue; }
        // Lazy flip (Pending → Done(TimedOut)). No persist needed — we're
        // about to delete this row.
        let record = record.flipped_to_timed_out_if_expired(now_ms).unwrap_or(record);
        // Only drain Done. Pending (awaiting operator) and InFlight
        // (invoke in progress) stay in state.
        if record.status != Status::Done { continue; }
        candidates.push(record);
    }

    // Phase 2: sort + cap.
    candidates.sort_by_key(|r| r.resolved_at.unwrap_or(u64::MAX));
    let total = candidates.len();
    let omitted = total.saturating_sub(limit) as u64;
    candidates.truncate(limit);

    // Phase 3: delete-and-return.
    let mut entries: Vec<Value> = Vec::with_capacity(candidates.len());
    for record in candidates {
        let key = pending_key(session_id, &record.function_call_id);
        if bus.delete(state_scope, &key).await.is_ok() {
            entries.push(record.to_value());
        }
    }
    json!({ "ok": true, "entries": entries, "omitted": omitted })
}

/// Force-cancel every non-terminal row in a session by flipping it to
/// `Done(TimedOut)`. Called from `run::stop` so a stale UI modal cannot
/// still execute its function after the operator clicks Stop. Lazy
/// timeout is not a substitute — default `expires_at` is 5 min and we
/// cannot leave a 5-min stale-modal window after Stop.
pub async fn handle_sweep_session(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
) -> Value {
    let session_id = payload.get("session_id").and_then(Value::as_str).unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id", "swept": 0 });
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let prefix = format!("{session_id}/");
    let rows = bus.list_prefix(state_scope, &prefix).await;
    let mut swept = 0u64;

    for raw in rows {
        let Some(record) = Record::from_value(raw) else { continue };
        if record.session_id != session_id { continue; }       // defensive
        if record.status == Status::Done { continue; }         // already terminal

        let key = pending_key(session_id, &record.function_call_id);
        let timed_out = record.done_at(now_ms, Outcome::TimedOut);
        if bus.set(state_scope, &key, timed_out.to_value()).await.is_ok() {
            swept += 1;
        }
    }
    json!({ "ok": true, "swept": swept })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Outcome, Record};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct InMemBus {
        rows: Mutex<std::collections::BTreeMap<(String, String), Value>>,
    }
    #[async_trait::async_trait]
    impl StateBus for InMemBus {
        async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
            self.rows.lock().unwrap().insert((scope.into(), key.into()), value);
            Ok(())
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.rows.lock().unwrap().get(&(scope.into(), key.into())).cloned()
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            self.rows.lock().unwrap()
                .iter()
                .filter(|((s, k), _)| s == scope && k.starts_with(prefix))
                .map(|(_, v)| v.clone())
                .collect()
        }
        async fn delete(&self, scope: &str, key: &str) -> Result<(), iii_sdk::IIIError> {
            self.rows.lock().unwrap().remove(&(scope.into(), key.into()));
            Ok(())
        }
    }

    async fn seed_done(bus: &InMemBus, session: &str, cid: &str, resolved_at: u64) {
        let r = Record::pending(
            cid.into(), "shell::exec".into(),
            json!({"command": "ls"}), session.into(), 0, 60_000,
        ).in_flight(resolved_at).done(Outcome::Executed { result: json!({"cid": cid}) });
        bus.set("approvals", &format!("{session}/{cid}"), r.to_value()).await.unwrap();
    }

    async fn seed_pending(bus: &InMemBus, session: &str, cid: &str, expires_at: u64) {
        let mut r = Record::pending(
            cid.into(), "shell::exec".into(),
            json!({}), session.into(), 0, 60_000);
        r.expires_at = expires_at;
        bus.set("approvals", &format!("{session}/{cid}"), r.to_value()).await.unwrap();
    }

    #[tokio::test]
    async fn consume_returns_done_rows_and_deletes_them() {
        let bus = InMemBus::default();
        seed_done(&bus, "sess_a", "tc-1", 100).await;
        seed_done(&bus, "sess_a", "tc-2", 200).await;
        let reply = handle_consume(&bus, "approvals",
            json!({"session_id": "sess_a"}), 1_000).await;
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["omitted"], 0);
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(bus.get("approvals", "sess_a/tc-1").await.is_none());
        assert!(bus.get("approvals", "sess_a/tc-2").await.is_none());
    }

    #[tokio::test]
    async fn consume_skips_pending_rows() {
        let bus = InMemBus::default();
        seed_done(&bus, "sess_a", "tc-1", 100).await;
        seed_pending(&bus, "sess_a", "tc-2", 999_999).await;
        let reply = handle_consume(&bus, "approvals",
            json!({"session_id": "sess_a"}), 1_000).await;
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(bus.get("approvals", "sess_a/tc-2").await.is_some());
    }

    #[tokio::test]
    async fn consume_lazy_flips_expired_pending_then_returns_and_deletes() {
        let bus = InMemBus::default();
        seed_pending(&bus, "sess_a", "tc-1", 500).await;
        let reply = handle_consume(&bus, "approvals",
            json!({"session_id": "sess_a"}), 1_000).await;
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["status"], "done");
        assert_eq!(entries[0]["outcome"]["kind"], "timed_out");
        assert!(bus.get("approvals", "sess_a/tc-1").await.is_none());
    }

    #[tokio::test]
    async fn consume_sorts_by_resolved_at_ascending() {
        let bus = InMemBus::default();
        seed_done(&bus, "sess_a", "tc-z-late", 300).await;
        seed_done(&bus, "sess_a", "tc-a-early", 100).await;
        seed_done(&bus, "sess_a", "tc-m-mid", 200).await;
        let reply = handle_consume(&bus, "approvals",
            json!({"session_id": "sess_a"}), 1_000).await;
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries[0]["function_call_id"], "tc-a-early");
        assert_eq!(entries[1]["function_call_id"], "tc-m-mid");
        assert_eq!(entries[2]["function_call_id"], "tc-z-late");
    }

    #[tokio::test]
    async fn consume_cap_with_omitted_counter() {
        let bus = InMemBus::default();
        for i in 0..60 {
            seed_done(&bus, "sess_a", &format!("tc-{i:02}"), i as u64).await;
        }
        let reply = handle_consume(&bus, "approvals",
            json!({"session_id": "sess_a", "limit": 50}), 1_000).await;
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 50);
        assert_eq!(reply["omitted"], 10);
        let still_there = bus.list_prefix("approvals", "sess_a/").await;
        assert_eq!(still_there.len(), 10);
    }

    #[tokio::test]
    async fn consume_missing_session_id_returns_error() {
        let bus = InMemBus::default();
        let reply = handle_consume(&bus, "approvals", json!({}), 1_000).await;
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], "missing_session_id");
    }

    #[tokio::test]
    async fn consume_defensive_session_id_filter_drops_foreign_rows() {
        let bus = InMemBus::default();
        let r = Record::pending(
            "tc-x".into(), "shell::exec".into(), json!({}),
            "sess_b".into(),  // WRONG session in data
            0, 60_000,
        ).in_flight(100).done(Outcome::Executed { result: json!({}) });
        bus.set("approvals", "sess_a/tc-x", r.to_value()).await.unwrap();

        let reply = handle_consume(&bus, "approvals",
            json!({"session_id": "sess_a"}), 1_000).await;
        let entries = reply["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 0);
        assert!(bus.get("approvals", "sess_a/tc-x").await.is_some(),
            "defensive: row stays in state, NOT deleted");
    }

    #[tokio::test]
    async fn sweep_flips_pending_and_done_untouched() {
        let bus = InMemBus::default();
        let pending = Record::pending(
            "tc-1".into(), "shell::exec".into(), json!({}),
            "sess_a".into(), 0, 60_000);
        bus.set("approvals", "sess_a/tc-1", pending.to_value()).await.unwrap();

        let in_flight = Record::pending(
            "tc-2".into(), "shell::exec".into(), json!({}),
            "sess_a".into(), 0, 60_000).in_flight(500);
        bus.set("approvals", "sess_a/tc-2", in_flight.to_value()).await.unwrap();

        let done = Record::pending(
            "tc-3".into(), "shell::exec".into(), json!({}),
            "sess_a".into(), 0, 60_000)
            .in_flight(100).done(Outcome::Executed { result: json!({}) });
        bus.set("approvals", "sess_a/tc-3", done.to_value()).await.unwrap();

        let reply = handle_sweep_session(&bus, "approvals",
            json!({"session_id": "sess_a"})).await;
        assert_eq!(reply["swept"], 2);

        let r1 = Record::from_value(bus.get("approvals", "sess_a/tc-1").await.unwrap()).unwrap();
        assert!(matches!(r1.outcome, Some(Outcome::TimedOut)));
        let r2 = Record::from_value(bus.get("approvals", "sess_a/tc-2").await.unwrap()).unwrap();
        assert!(matches!(r2.outcome, Some(Outcome::TimedOut)));
        let r3 = Record::from_value(bus.get("approvals", "sess_a/tc-3").await.unwrap()).unwrap();
        assert!(matches!(r3.outcome, Some(Outcome::Executed { .. })),
            "already-Done rows must not be re-stamped");
    }

    #[tokio::test]
    async fn list_pending_lazy_flips_expired_rows_out_of_the_list() {
        let bus = InMemBus::default();
        // tc-live: expires far in the future (year ~5138). tc-expired:
        // expires near epoch — definitely past now.
        seed_pending(&bus, "sess_a", "tc-live",    u64::MAX).await;
        seed_pending(&bus, "sess_a", "tc-expired", 500).await;
        // Advance the system clock indirectly: just trust the inline now_ms
        // in handle_list_pending. expires_at=500 < now_ms, so it should flip.
        // Wait briefly to ensure SystemTime::now() > 500ms since UNIX_EPOCH
        // (it's well past 1970, so any current time satisfies this).
        let reply = handle_list_pending(&bus, "approvals",
            json!({"session_id": "sess_a"})).await;
        let pending = reply["pending"].as_array().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["function_call_id"], "tc-live");

        // Expired row is now persisted as Done(TimedOut).
        let r = Record::from_value(bus.get("approvals", "sess_a/tc-expired").await.unwrap()).unwrap();
        assert!(matches!(r.outcome, Some(Outcome::TimedOut)));
    }
}
