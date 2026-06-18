//! The pending inbox records: `approval_pending/<session_id>/<function_call_id>`.
//! Deliberately ephemeral — a record exists only while a call is held.
//! Every record has an explicit deletion path (resolve, turn/session
//! purge), which is what keeps `state::list` O(live holds).

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::state;
use crate::types::PendingApprovalRecord;

pub const PENDING_SCOPE: &str = "approval_pending";

/// Callers validate ids first (no `/`); see types::validate_id.
pub fn pending_key(session_id: &str, function_call_id: &str) -> String {
    format!("{session_id}/{function_call_id}")
}

/// Tolerant parse: null/garbage → None (a corrupt record must not wedge
/// the inbox; turn/session purge collects orphaned records).
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
    iii: &III,
    session_id: &str,
    function_call_id: &str,
) -> Result<Option<PendingApprovalRecord>, IIIError> {
    let reply = state::get(
        iii,
        PENDING_SCOPE,
        &pending_key(session_id, function_call_id),
    )
    .await?;
    Ok(parse_record(&reply))
}

/// Write the record. Returns the previous value when one existed (a
/// concurrent duplicate hold lost the race — the caller must not emit a
/// second `pending_created`).
pub async fn put(iii: &III, record: &PendingApprovalRecord) -> Result<Option<Value>, IIIError> {
    let reply = state::set(
        iii,
        PENDING_SCOPE,
        &pending_key(&record.session_id, &record.function_call_id),
        serde_json::to_value(record).unwrap_or(Value::Null),
    )
    .await?;
    let old = reply.get("old_value").cloned().unwrap_or(Value::Null);
    Ok(if old.is_null() { None } else { Some(old) })
}

/// The single deletion helper every lifecycle path funnels through —
/// deletion is the emit gate (approval-gate.md § Deletion is the emit
/// gate): only the caller that observed the live record emits
/// `pending_resolved`, so concurrent paths (a resolve racing a turn
/// abort) produce exactly one event per record.
///
/// Mechanics: `state::set null` is the atomic gate — the engine swaps the
/// value under its write lock and returns the prior one — but it stores a
/// literal null rather than deleting (engine kv semantics), so a
/// follow-up `state::delete` removes the tombstone and keeps the scope
/// list O(live). The delete is benign if it races: the gate already
/// decided emission.
pub async fn delete_with_gate(
    iii: &III,
    session_id: &str,
    function_call_id: &str,
) -> Result<Option<PendingApprovalRecord>, IIIError> {
    let key = pending_key(session_id, function_call_id);
    let reply = state::set(iii, PENDING_SCOPE, &key, Value::Null).await?;
    let old = reply.get("old_value").cloned().unwrap_or(Value::Null);
    if let Err(e) = state::delete(iii, PENDING_SCOPE, &key).await {
        // The null tombstone survives until the next delete attempt; it
        // is invisible to readers (parse_record skips nulls).
        tracing::warn!(key, error = %e, "tombstone cleanup failed");
    }
    Ok(parse_record(&old))
}

/// Full-scope scan, values-only (the engine's `state::list` contract).
/// Malformed/null values are skipped.
pub async fn list_all(iii: &III) -> Result<Vec<PendingApprovalRecord>, IIIError> {
    let reply = state::list(iii, PENDING_SCOPE).await?;
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
    use serde_json::json;

    fn record(session_id: &str, call_id: &str) -> PendingApprovalRecord {
        PendingApprovalRecord {
            session_id: session_id.into(),
            turn_id: "t_1".into(),
            function_call_id: call_id.into(),
            function_id: "shell::run".into(),
            arguments_excerpt: json!({ "cmd": "ls" }),
            pending_at: 100,
            session_title: None,
            session_description: None,
            session_metadata: None,
            depth: 0,
            assistant_excerpt: None,
        }
    }

    #[test]
    fn parse_record_skips_null_and_garbage() {
        assert!(parse_record(&Value::Null).is_none());
        assert!(parse_record(&json!("garbage")).is_none());
        assert_eq!(
            parse_record(&serde_json::to_value(record("s_1", "c_1")).unwrap())
                .unwrap()
                .session_id,
            "s_1"
        );
    }
}
