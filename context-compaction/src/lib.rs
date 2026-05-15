//! `context-compaction` worker.
//!
//! Subscribes to `agent::events` on the iii bus. On every `TurnEnd`, checks
//! whether the running transcript has crossed the configured token
//! threshold; if so, kicks off an out-of-band summarisation via
//! `router::stream_assistant` and appends a `Compaction` entry to the
//! session-tree. The orchestrator's next turn picks up the compacted
//! transcript via `session-tree::load_messages`.
//!
//! No LLM-facing tools. Invisible to the model. The only observable
//! artefact is an extra `Compaction` entry in the session-tree.

pub mod config;
pub mod manifest;
pub mod summarize;
pub mod threshold;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, Value,
    III,
};
use serde_json::json;

const AGENT_EVENTS_STREAM: &str = "agent::events";
const SUBSCRIBER_FN: &str = "context-compaction::on_agent_event";
const STATE_SCOPE: &str = "agent";
/// Must exceed `summarize::SUMMARIZER_TIMEOUT_MS / 1000` (currently 120s) so
/// a slow LLM call can't have its own lease expire underneath it and let a
/// peer start a duplicate compaction. 5 min also bounds how long a crashed
/// worker can block compaction: the next TurnEnd after the TTL elapses
/// re-acquires.
const LEASE_TTL_SECS: i64 = 300;

/// Register the agent::events subscriber and return a handle the binary
/// can hold for the worker's lifetime.
pub fn register_with_iii(iii: &Arc<III>) -> anyhow::Result<Refs> {
    let subscriber_fn = register_subscriber_fn(iii);
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "stream".into(),
        function_id: SUBSCRIBER_FN.into(),
        config: json!({ "stream_name": AGENT_EVENTS_STREAM }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(Refs { subscriber_fn })
}

/// Handles returned by [`register_with_iii`]. Hold for the lifetime of the
/// worker; drop / unregister on shutdown.
pub struct Refs {
    pub subscriber_fn: FunctionRef,
}

fn register_subscriber_fn(iii: &Arc<III>) -> FunctionRef {
    let iii_for_handler = Arc::clone(iii);
    iii.register_function((
        RegisterFunctionMessage::with_id(SUBSCRIBER_FN.to_string()).with_description(
            "Internal: subscribes to agent::events; triggers session compaction on TurnEnd when running tokens exceed the configured threshold."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = Arc::clone(&iii_for_handler);
            async move {
                handle_event(&iii, payload).await;
                // Stream subscribers don't return meaningful data; the
                // engine ignores anything that isn't an Err.
                Ok::<Value, IIIError>(Value::Null)
            }
        },
    ))
}

/// Handle a single `agent::events` frame. Returns silently on every
/// non-actionable shape (non-TurnEnd events, missing fields, races) so a
/// malformed frame never aborts the subscriber.
async fn handle_event(iii: &III, payload: Value) {
    let Some((session_id, event)) = extract_event_payload(&payload) else {
        return;
    };
    let Some(usage) = turn_end_usage(&event) else {
        return;
    };
    let total_tokens = usage_total(&usage);
    if !threshold::should_compact(total_tokens) {
        return;
    }
    let Some(nonce) = acquire_lease(iii, &session_id).await else {
        tracing::debug!(%session_id, "compaction lease held; skipping");
        return;
    };
    if let Err(e) = run_compaction(iii, &session_id).await {
        tracing::warn!(%session_id, error = %e, "compaction failed");
    }
    release_lease(iii, &session_id, &nonce).await;
}

/// Decode the stream frame into `(session_id, event_data)`. Mirrors the
/// extraction logic in `harness/src/fanout.rs::extract_event_payload` so
/// both camelCase (`groupId`) and snake_case (`group_id`) envelopes work.
fn extract_event_payload(payload: &Value) -> Option<(String, Value)> {
    let session_id = payload
        .get("groupId")
        .or_else(|| payload.get("group_id"))
        .and_then(Value::as_str)?
        .to_string();
    let data = payload
        .get("event")
        .and_then(|e| e.get("data"))
        .cloned()
        .or_else(|| payload.get("data").cloned())
        .unwrap_or(Value::Null);
    Some((session_id, data))
}

/// Return `Some(usage)` if the event is a `TurnEnd` carrying a `message`
/// with `usage`. None otherwise — we only decide on TurnEnd to mirror
/// the orchestrator's commit point.
fn turn_end_usage(event: &Value) -> Option<Value> {
    let kind = event.get("type").and_then(Value::as_str)?;
    if kind != "TurnEnd" && kind != "turn_end" {
        return None;
    }
    event
        .get("message")
        .and_then(|m| m.get("usage"))
        .cloned()
        .filter(|u| !u.is_null())
}

/// Anthropic and OpenAI both report only the *uncached* portion of the
/// prompt as `input` once caching is on. Adding `cache_read` recovers the
/// total transcript size we'd pay for without caching — which is the
/// right number to threshold on, because compaction should fire based on
/// *true* context-window pressure, not on what happens to be cache-hot.
fn usage_total(usage: &Value) -> u64 {
    let pick = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    pick("input") + pick("output") + pick("cache_read")
}

async fn run_compaction(iii: &III, session_id: &str) -> anyhow::Result<()> {
    summarize::summarize_and_append(iii, session_id).await
}

fn lease_key(session_id: &str) -> String {
    format!("session/{session_id}/compaction_lease")
}

/// Monotonic per-process counter mixed into [`mint_lease_nonce`] so two
/// nonces minted in the same nanosecond within one process still diverge.
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint a per-attempt nonce for the lease's "nonce-and-readback" protocol.
/// pid covers cross-process collisions; nanos covers cross-attempt within
/// one process; the counter covers two concurrent attempts in the same
/// nanosecond.
fn mint_lease_nonce() -> String {
    let pid = std::process::id();
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid}-{nanos}-{seq}")
}

/// Acquire a single-writer lease for compaction on this session via
/// nonce-and-readback. Returns the winning nonce when granted, `None` when
/// another writer holds an unexpired lease OR raced us and won.
///
/// Why a nonce: the engine's `state::*` ops have no CAS primitive. A naive
/// check-then-write is a race — two concurrent writers both see an empty
/// lease, both write their timestamp, and both proceed. By stamping a
/// unique nonce and reading it back, exactly one writer sees its own
/// nonce surviving in the store: state::set is last-write-wins, so
/// whichever writer landed last "owns" the readback. Loser sees a
/// different nonce and bails.
async fn acquire_lease(iii: &III, session_id: &str) -> Option<String> {
    let key = lease_key(session_id);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let now_secs = now_ms / 1000;

    // Bail early if a non-expired lease is already held. Saves the
    // readback round-trip in the common no-contention case.
    if let Some(existing) = state_get(iii, &key).await {
        let ts_secs = read_lease_timestamp_secs(&existing);
        if ts_secs > 0 && now_secs - ts_secs < LEASE_TTL_SECS {
            return None;
        }
    }

    let nonce = mint_lease_nonce();
    state_set(iii, &key, json!({ "nonce": nonce, "ts": now_ms })).await;

    let stored = state_get(iii, &key).await;
    let stored_nonce = stored
        .as_ref()
        .and_then(|v| v.get("nonce"))
        .and_then(Value::as_str);
    if stored_nonce == Some(nonce.as_str()) {
        Some(nonce)
    } else {
        None
    }
}

/// Read the lease's millisecond timestamp from either the current
/// `{nonce, ts}` shape or the legacy bare-i64 shape, and return it in
/// seconds. Returns 0 when the value is missing or unrecognised, which
/// makes any caller treat the lease as expired.
fn read_lease_timestamp_secs(v: &Value) -> i64 {
    if let Some(ms) = v.get("ts").and_then(Value::as_i64) {
        return ms / 1000;
    }
    if let Some(secs) = v.as_i64() {
        return secs;
    }
    0
}

/// Release the lease only if we still hold it. Without this check, a
/// long-running compaction that exceeded the TTL could clear the lease
/// belonging to a successor worker.
async fn release_lease(iii: &III, session_id: &str, our_nonce: &str) {
    let key = lease_key(session_id);
    let stored = state_get(iii, &key).await;
    let stored_nonce = stored
        .as_ref()
        .and_then(|v| v.get("nonce"))
        .and_then(Value::as_str);
    if stored_nonce == Some(our_nonce) {
        state_set(iii, &key, Value::Null).await;
    }
}

/// Stamp the orchestrator-watched key so `handle_streaming` knows it must
/// rebuild its hot `session/<id>/messages` view from session-tree before
/// the next assistant turn. Called from
/// [`summarize::summarize_and_append`] after the Compaction entry has
/// been committed.
pub async fn stamp_last_compaction(iii: &III, session_id: &str) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    state_set(
        iii,
        &format!("session/{session_id}/last_compaction_at"),
        json!(now_ms),
    )
    .await;
}

async fn state_get(iii: &III, key: &str) -> Option<Value> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
    .ok()
    .filter(|v| !v.is_null())
}

async fn state_set(iii: &III, key: &str, value: Value) {
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %key, "context-compaction: state::set failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_event_payload_handles_camelcase() {
        let env = json!({
            "groupId": "sess-1",
            "event": { "data": { "type": "TurnEnd", "message": {} } },
        });
        let (sid, data) = extract_event_payload(&env).expect("extracts");
        assert_eq!(sid, "sess-1");
        assert_eq!(data["type"], "TurnEnd");
    }

    #[test]
    fn extract_event_payload_handles_snake_case() {
        let env = json!({
            "group_id": "sess-2",
            "data": { "type": "TurnEnd" },
        });
        let (sid, data) = extract_event_payload(&env).expect("extracts");
        assert_eq!(sid, "sess-2");
        assert_eq!(data["type"], "TurnEnd");
    }

    #[test]
    fn turn_end_usage_extracts_when_present() {
        let event = json!({
            "type": "TurnEnd",
            "message": { "usage": { "input": 100, "output": 50, "cache_read": 800 } },
        });
        let u = turn_end_usage(&event).expect("has usage");
        assert_eq!(u["input"], 100);
    }

    #[test]
    fn turn_end_usage_skips_non_turn_end() {
        for kind in ["TurnStart", "MessageStart", "FunctionExecutionStart", "AgentStart"] {
            let event = json!({ "type": kind, "message": { "usage": { "input": 9999 } }});
            assert!(turn_end_usage(&event).is_none(), "{kind} must be ignored");
        }
    }

    #[test]
    fn turn_end_usage_skips_missing_usage() {
        let event = json!({ "type": "TurnEnd", "message": {} });
        assert!(turn_end_usage(&event).is_none());
    }

    #[test]
    fn usage_total_sums_all_three_buckets() {
        let u = json!({ "input": 100, "output": 50, "cache_read": 800, "cache_write": 200 });
        // cache_write is intentionally excluded — it counts toward cost
        // but not toward transcript size.
        assert_eq!(usage_total(&u), 950);
    }

    #[test]
    fn usage_total_handles_missing_fields() {
        let u = json!({ "input": 100 });
        assert_eq!(usage_total(&u), 100);
    }

    #[test]
    fn lease_key_namespaces_by_session() {
        assert!(lease_key("sess-9").contains("sess-9"));
        assert!(lease_key("sess-9").contains("compaction_lease"));
    }

    #[test]
    fn mint_lease_nonce_returns_unique_values_across_calls() {
        let n1 = mint_lease_nonce();
        let n2 = mint_lease_nonce();
        let n3 = mint_lease_nonce();
        assert_ne!(n1, n2);
        assert_ne!(n2, n3);
        assert_ne!(n1, n3);
    }

    #[test]
    fn mint_lease_nonce_diverges_for_same_nanosecond_calls() {
        // The pid+nanos+counter recipe must survive the tightest possible
        // back-to-back call. The counter alone guarantees this even when
        // nanos repeats.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(mint_lease_nonce()), "nonces must not collide");
        }
    }

    #[test]
    fn read_lease_timestamp_secs_accepts_new_object_shape() {
        let v = json!({ "nonce": "abc", "ts": 1_700_000_000_000_i64 });
        assert_eq!(read_lease_timestamp_secs(&v), 1_700_000_000);
    }

    #[test]
    fn read_lease_timestamp_secs_accepts_legacy_bare_int_shape() {
        // Pre-nonce releases wrote a bare i64 (seconds). Restart compat: a
        // worker upgraded mid-flight may read a legacy lease and must still
        // interpret its TTL.
        let v = json!(1_700_000_000_i64);
        assert_eq!(read_lease_timestamp_secs(&v), 1_700_000_000);
    }

    #[test]
    fn read_lease_timestamp_secs_returns_zero_for_garbage() {
        // Anything we can't parse must read as "expired" so a writer
        // attempts re-acquisition instead of being permanently blocked.
        assert_eq!(read_lease_timestamp_secs(&json!({"unrelated": true})), 0);
        assert_eq!(read_lease_timestamp_secs(&json!("not an int")), 0);
        assert_eq!(read_lease_timestamp_secs(&Value::Null), 0);
    }

    #[test]
    fn lease_ttl_exceeds_summarizer_timeout() {
        // The acquire_lease comment says LEASE_TTL_SECS must exceed the
        // summariser LLM call timeout so a long call can't lose its lease
        // mid-flight. This test pins that invariant.
        let summarizer_timeout_secs =
            (crate::summarize::SUMMARIZER_TIMEOUT_MS_FOR_TEST / 1000) as i64;
        assert!(
            LEASE_TTL_SECS > summarizer_timeout_secs,
            "lease TTL ({LEASE_TTL_SECS}s) must exceed summarizer timeout ({summarizer_timeout_secs}s)"
        );
    }
}
