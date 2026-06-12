//! The pending inbox records: `approval_pending/<session_id>/<function_call_id>`.
//! Deliberately ephemeral — a record exists only while a call is held.
//! Every record has an explicit deletion path and the sweep as GC
//! backstop (approval-gate.md § State lifecycle), which is what keeps
//! `state::list` O(live holds).

use serde_json::{json, Value};

use crate::bus::{Bus, BusError};
use crate::types::PendingApprovalRecord;

pub const PENDING_SCOPE: &str = "approval_pending";

/// Callers validate ids first (no `/`); see types::validate_id.
pub fn pending_key(session_id: &str, function_call_id: &str) -> String {
    format!("{session_id}/{function_call_id}")
}

/// Tolerant parse: null/garbage → None (a corrupt record must not wedge
/// the inbox; the sweep collects it once it has an `expires_at`, and a
/// record without one is skipped everywhere).
pub fn parse_record(value: &Value) -> Option<PendingApprovalRecord> {
    if value.is_null() {
        return None;
    }
    match serde_json::from_value::<PendingApprovalRecord>(value.clone()) {
        Ok(record) => Some(record),
        Err(e) => {
            tracing::warn!(error = %e, "unparseable approval_pending record; skipping");
            None
        }
    }
}

pub async fn get(
    bus: &dyn Bus,
    session_id: &str,
    function_call_id: &str,
    timeout_ms: u64,
) -> Result<Option<PendingApprovalRecord>, BusError> {
    let reply = bus
        .call(
            "state::get",
            json!({ "scope": PENDING_SCOPE, "key": pending_key(session_id, function_call_id) }),
            Some(timeout_ms),
        )
        .await?;
    Ok(parse_record(&reply))
}

/// Write the record. Returns the previous value when one existed (a
/// concurrent duplicate hold lost the race — the caller must not emit a
/// second `pending_created`).
pub async fn put(
    bus: &dyn Bus,
    record: &PendingApprovalRecord,
    timeout_ms: u64,
) -> Result<Option<Value>, BusError> {
    let reply = bus
        .call(
            "state::set",
            json!({
                "scope": PENDING_SCOPE,
                "key": pending_key(&record.session_id, &record.function_call_id),
                "value": record,
            }),
            Some(timeout_ms),
        )
        .await?;
    let old = reply.get("old_value").cloned().unwrap_or(Value::Null);
    Ok(if old.is_null() { None } else { Some(old) })
}

/// The single deletion helper every lifecycle path funnels through —
/// deletion is the emit gate (approval-gate.md § Deletion is the emit
/// gate): only the caller that observed the live record emits
/// `pending_resolved`, so concurrent paths (a resolve racing the sweep
/// racing a turn abort) produce exactly one event per record.
///
/// Mechanics: `state::set null` is the atomic gate — the engine swaps the
/// value under its write lock and returns the prior one — but it stores a
/// literal null rather than deleting (engine kv semantics), so a
/// follow-up `state::delete` removes the tombstone and keeps the scope
/// list O(live). The delete is benign if it races: the gate already
/// decided emission.
pub async fn delete_with_gate(
    bus: &dyn Bus,
    session_id: &str,
    function_call_id: &str,
    timeout_ms: u64,
) -> Result<Option<PendingApprovalRecord>, BusError> {
    let key = pending_key(session_id, function_call_id);
    let reply = bus
        .call(
            "state::set",
            json!({ "scope": PENDING_SCOPE, "key": key, "value": Value::Null }),
            Some(timeout_ms),
        )
        .await?;
    let old = reply.get("old_value").cloned().unwrap_or(Value::Null);
    if let Err(e) = bus
        .call(
            "state::delete",
            json!({ "scope": PENDING_SCOPE, "key": key }),
            Some(timeout_ms),
        )
        .await
    {
        // The null tombstone survives until the next delete attempt; it
        // is invisible to readers (parse_record skips nulls).
        tracing::warn!(key, error = %e, "tombstone cleanup failed");
    }
    Ok(parse_record(&old))
}

/// Full-scope scan, values-only (the engine's `state::list` contract).
/// Malformed/null values are skipped.
pub async fn list_all(
    bus: &dyn Bus,
    timeout_ms: u64,
) -> Result<Vec<PendingApprovalRecord>, BusError> {
    let reply = bus
        .call(
            "state::list",
            json!({ "scope": PENDING_SCOPE }),
            Some(timeout_ms),
        )
        .await?;
    let values = match reply {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        other => {
            tracing::warn!(?other, "unexpected state::list reply shape");
            Vec::new()
        }
    };
    Ok(values.iter().filter_map(parse_record).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::FakeBus;
    use serde_json::json;

    fn record(session_id: &str, call_id: &str) -> PendingApprovalRecord {
        PendingApprovalRecord {
            session_id: session_id.into(),
            turn_id: "t_1".into(),
            function_call_id: call_id.into(),
            function_id: "shell::run".into(),
            arguments_excerpt: json!({ "cmd": "ls" }),
            pending_at: 100,
            expires_at: 1_800_100,
            session_title: None,
            session_description: None,
            session_metadata: None,
            depth: 0,
            assistant_excerpt: None,
        }
    }

    #[tokio::test]
    async fn put_get_roundtrip() {
        let bus = FakeBus::new();
        let _state = bus.with_memory_state();
        let rec = record("s_1", "c_1");
        assert!(put(&bus, &rec, 100).await.unwrap().is_none());
        let read = get(&bus, "s_1", "c_1", 100).await.unwrap().unwrap();
        assert_eq!(read, rec);
    }

    #[tokio::test]
    async fn put_reports_prior_value_on_duplicate() {
        let bus = FakeBus::new();
        let _state = bus.with_memory_state();
        let rec = record("s_1", "c_1");
        assert!(put(&bus, &rec, 100).await.unwrap().is_none());
        assert!(put(&bus, &rec, 100).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_with_gate_returns_the_record_exactly_once() {
        let bus = FakeBus::new();
        let state = bus.with_memory_state();
        let rec = record("s_1", "c_1");
        put(&bus, &rec, 100).await.unwrap();

        let first = delete_with_gate(&bus, "s_1", "c_1", 100).await.unwrap();
        assert_eq!(first, Some(rec));
        // Second deletion path observes no live record: no emission.
        let second = delete_with_gate(&bus, "s_1", "c_1", 100).await.unwrap();
        assert!(second.is_none());
        // The tombstone left by `state::set null` was cleaned up.
        assert!(state.peek(PENDING_SCOPE, "s_1/c_1").is_none());
    }

    #[tokio::test]
    async fn list_all_skips_null_tombstones_and_garbage() {
        let bus = FakeBus::new();
        let state = bus.with_memory_state();
        put(&bus, &record("s_1", "c_1"), 100).await.unwrap();
        state.seed(PENDING_SCOPE, "s_2/c_9", Value::Null);
        state.seed(PENDING_SCOPE, "s_3/c_8", json!("garbage"));

        let records = list_all(&bus, 100).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "s_1");
    }

    #[tokio::test]
    async fn state_outage_surfaces_as_bus_error() {
        let bus = FakeBus::new();
        assert!(get(&bus, "s_1", "c_1", 100).await.is_err());
        assert!(list_all(&bus, 100).await.is_err());
        assert!(delete_with_gate(&bus, "s_1", "c_1", 100).await.is_err());
    }
}
