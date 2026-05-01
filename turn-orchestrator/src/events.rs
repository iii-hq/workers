//! Event emission for `turn-orchestrator`.
//!
//! Every state transition that produces a user-visible lifecycle event
//! writes one [`AgentEvent`] frame to the `agent::events/<session_id>`
//! stream via `stream::set`. The shape is byte-compatible with the
//! `IiiSink` impl in `harness-runtime/src/register.rs:425-472`, so any
//! consumer that worked against `agent::run_loop` works against
//! `run::start_and_wait` without changes.
//!
//! Item-ids are minted from a per-session counter held in iii state under
//! `session/<sid>/event_counter`. Storing the counter in state (rather
//! than a process-local atomic) lets the orchestrator emit a coherent
//! id sequence across restarts — the durability test exercises this
//! path by interrupting mid-turn and republishing `turn::step_requested`.

use harness_types::AgentEvent;
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

/// Stream name for agent events. Matches `harness_runtime::EVENTS_STREAM`.
pub const EVENTS_STREAM: &str = "agent::events";

/// State scope for the per-session event counter.
const STATE_SCOPE: &str = "agent";

fn counter_key(session_id: &str) -> String {
    format!("session/{session_id}/event_counter")
}

pub fn format_item_id(session_id: &str, seq: u64) -> String {
    format!("{session_id}-{seq:08}")
}

/// Emit a single `AgentEvent` frame.
///
/// Best-effort: bus errors are logged at warn-level and swallowed so a
/// transient stream failure cannot stop the state machine. The orchestrator
/// always persists state before emitting; a dropped event is recoverable
/// (callers can re-read the transcript), a wedged state machine is not.
pub async fn emit(iii: &III, session_id: &str, event: &AgentEvent) {
    let payload = match serde_json::to_value(event) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(?err, %session_id, "failed to serialise AgentEvent");
            return;
        }
    };
    let seq = next_seq(iii, session_id).await;
    let item_id = format_item_id(session_id, seq);
    if let Err(err) = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".to_string(),
            payload: json!({
                "stream_name": EVENTS_STREAM,
                "group_id": session_id,
                "item_id": item_id,
                "data": payload,
            }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %err, %session_id, %item_id, "stream::set failed");
    }
}

/// Atomically increment the per-session event counter and return its
/// previous value. Falls back to a synthetic 0 on bus error so emission
/// never panics; ids may collide briefly under that fallback, but the
/// CLI/TUI printers tolerate duplicate item-ids (they dedup by sequence).
async fn next_seq(iii: &III, session_id: &str) -> u64 {
    let key = counter_key(session_id);
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "state::update".to_string(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": key,
                "ops": [{ "type": "increment", "path": "", "by": 1 }],
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
    match resp {
        Ok(v) => v.get("old_value").and_then(Value::as_u64).unwrap_or(0),
        Err(err) => {
            tracing::warn!(error = %err, %session_id, "state::update for event_counter failed");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_id_is_zero_padded_and_session_prefixed() {
        let id_zero = format_item_id("sess-abc", 0);
        let id_one = format_item_id("sess-abc", 1);
        let id_huge = format_item_id("sess-abc", 12_345_678);
        assert_eq!(id_zero, "sess-abc-00000000");
        assert_eq!(id_one, "sess-abc-00000001");
        assert_eq!(id_huge, "sess-abc-12345678");
    }

    #[test]
    fn agent_start_serialises_to_typed_object() {
        let evt = harness_types::AgentEvent::AgentStart;
        let v = serde_json::to_value(&evt).expect("serialise");
        assert_eq!(v, serde_json::json!({"type": "agent_start"}));
    }

    #[test]
    fn turn_start_serialises_to_typed_object() {
        let evt = harness_types::AgentEvent::TurnStart;
        let v = serde_json::to_value(&evt).expect("serialise");
        assert_eq!(v, serde_json::json!({"type": "turn_start"}));
    }
}
