//! State for the `approval::grant-watch` ladder's memory rungs (steps 6-7
//! of contracts.md § post_trigger / spec-pr3): the per-session "denied this
//! session" directory list and the per-call re-ask attempt counter.
//!
//! Both scopes follow the same read-modify-write discipline as
//! `settings.rs` (materialize_and) — the crate has no atomic `state::update`
//! wrapper today (`state.rs` exposes only get/set/delete/list) and these
//! counters are a soft cap, not a security boundary, so a tolerant RMW is
//! the same tradeoff the codebase already makes elsewhere. See
//! `pure` helpers below for the mutation logic in isolation.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state;

pub const DENIED_SCOPE: &str = "approval_grant_denied";
pub const ATTEMPTS_SCOPE: &str = "approval_grant_attempts";

// ---------------------------------------------------------------------------
// Denied memory: approval_grant_denied/<session_id> -> string[]
// ---------------------------------------------------------------------------

/// Idempotent append — mirrors `settings::with_grant`.
pub fn with_denied(entries: &[String], dir: &str) -> Vec<String> {
    if entries.iter().any(|d| d == dir) {
        return entries.to_vec();
    }
    let mut next = entries.to_vec();
    next.push(dir.to_string());
    next
}

fn parse_denied(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Tolerant read: any failure (state outage, garbage) degrades to an empty
/// list — never blocks the ladder on a denied-memory outage.
pub async fn read_denied(iii: &IIIClient, session_id: &str) -> Vec<String> {
    match state::get(iii, DENIED_SCOPE, session_id).await {
        Ok(value) => parse_denied(&value),
        Err(e) => {
            tracing::warn!(session_id, error = %e, "denied-memory read failed; treating as empty");
            Vec::new()
        }
    }
}

/// Record a denial. Read-modify-write; races only widen the list by an
/// idempotent no-op (the entry is deduped either way).
pub async fn add_denied(iii: &IIIClient, session_id: &str, dir: &str) -> Result<(), Error> {
    let value = state::get(iii, DENIED_SCOPE, session_id).await?;
    let next = with_denied(&parse_denied(&value), dir);
    state::set(
        iii,
        DENIED_SCOPE,
        session_id,
        serde_json::to_value(next).unwrap_or(Value::Array(Vec::new())),
    )
    .await?;
    Ok(())
}

/// Session-delete purge: drop the whole list.
pub async fn clear_denied(iii: &IIIClient, session_id: &str) -> Result<(), Error> {
    state::delete(iii, DENIED_SCOPE, session_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Attempt cap: approval_grant_attempts/<session_id>/<function_call_id>
// ---------------------------------------------------------------------------

/// Stored value — carries `session_id` / `function_call_id` / `turn_id` so
/// turn-scoped purge can find and delete matching entries the same way
/// `pending::list_all` + `purge_matching` does for the inbox (the engine's
/// `state::list` returns values only, not keys).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub session_id: String,
    pub function_call_id: String,
    pub turn_id: String,
    pub count: u32,
}

fn attempt_key(session_id: &str, function_call_id: &str) -> String {
    format!("{session_id}/{function_call_id}")
}

fn parse_attempt(value: &Value) -> Option<AttemptRecord> {
    if value.is_null() {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

/// Outcome of the check-and-increment step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// Below the cap; the counter was incremented to this value.
    Allowed(u32),
    /// At or above the cap; the counter was left untouched.
    Capped,
}

/// Pure decision: does `current` (the count BEFORE this attempt) allow one
/// more hold under `limit`?
pub fn check_cap(current: u32, limit: usize) -> bool {
    (current as usize) < limit
}

/// Read the current count (0 when no record exists), and — if under
/// `limit` — write it back incremented by one. Read-modify-write (see
/// module docs); the ladder treats any I/O failure here as fail-open
/// (Continue), so a lost race merely re-asks a little more than `limit`
/// times, never fewer.
pub async fn check_and_bump(
    iii: &IIIClient,
    session_id: &str,
    function_call_id: &str,
    turn_id: &str,
    limit: usize,
) -> Result<AttemptOutcome, Error> {
    let key = attempt_key(session_id, function_call_id);
    let value = state::get(iii, ATTEMPTS_SCOPE, &key).await?;
    let current = parse_attempt(&value).map(|r| r.count).unwrap_or(0);
    if !check_cap(current, limit) {
        return Ok(AttemptOutcome::Capped);
    }
    let next = current + 1;
    let record = AttemptRecord {
        session_id: session_id.to_string(),
        function_call_id: function_call_id.to_string(),
        turn_id: turn_id.to_string(),
        count: next,
    };
    state::set(
        iii,
        ATTEMPTS_SCOPE,
        &key,
        serde_json::to_value(&record).unwrap_or(Value::Null),
    )
    .await?;
    Ok(AttemptOutcome::Allowed(next))
}

async fn list_attempts(iii: &IIIClient) -> Result<Vec<AttemptRecord>, Error> {
    let reply = state::list(iii, ATTEMPTS_SCOPE).await?;
    let values = match reply {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        other => {
            tracing::warn!(
                ?other,
                "unexpected state::list reply shape for attempts scope"
            );
            Vec::new()
        }
    };
    Ok(values.iter().filter_map(parse_attempt).collect())
}

/// Purge every attempt counter matching `predicate`. Errors degrade to "0
/// purged, try again next event" — mirrors `functions::purge::purge_matching`.
pub async fn purge_matching<F>(iii: &IIIClient, predicate: F) -> usize
where
    F: Fn(&AttemptRecord) -> bool,
{
    let records = match list_attempts(iii).await {
        Ok(records) => records,
        Err(e) => {
            tracing::warn!(error = %e, "grant-attempts purge: list failed; retry on next event");
            return 0;
        }
    };
    let mut purged = 0usize;
    for record in records.into_iter().filter(|r| predicate(r)) {
        let key = attempt_key(&record.session_id, &record.function_call_id);
        match state::delete(iii, ATTEMPTS_SCOPE, &key).await {
            Ok(_) => purged += 1,
            Err(e) => {
                tracing::warn!(key, error = %e, "grant-attempts purge: delete failed");
            }
        }
    }
    purged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_denied_is_idempotent() {
        let one = with_denied(&[], "/a/b");
        let two = with_denied(&one, "/a/b");
        assert_eq!(one, two);
        assert_eq!(two, vec!["/a/b".to_string()]);
    }

    #[test]
    fn with_denied_appends_distinct_dirs() {
        let one = with_denied(&[], "/a/b");
        let two = with_denied(&one, "/c/d");
        assert_eq!(two, vec!["/a/b".to_string(), "/c/d".to_string()]);
    }

    #[test]
    fn check_cap_is_strictly_less_than_limit() {
        assert!(check_cap(0, 3));
        assert!(check_cap(2, 3));
        assert!(!check_cap(3, 3));
        assert!(!check_cap(4, 3));
    }

    #[test]
    fn parse_denied_tolerates_garbage() {
        assert!(parse_denied(&Value::Null).is_empty());
        assert!(parse_denied(&serde_json::json!("garbage")).is_empty());
        assert_eq!(
            parse_denied(&serde_json::json!(["/a", "/b"])),
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn parse_attempt_tolerates_garbage() {
        assert!(parse_attempt(&Value::Null).is_none());
        assert!(parse_attempt(&serde_json::json!("garbage")).is_none());
        let record = AttemptRecord {
            session_id: "s_1".into(),
            function_call_id: "c_1".into(),
            turn_id: "t_1".into(),
            count: 2,
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(parse_attempt(&value).unwrap(), record);
    }
}
