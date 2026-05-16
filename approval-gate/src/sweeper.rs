//! Periodic timeout sweeper + stream-event helpers.
//!
//! The sweeper runs as a background task: every `interval_ms` it scans
//! the configured state scope, promotes any pending record past its
//! `expires_at` to `timed_out`, and emits the resulting
//! `approval_resolved` event on `agent::events/<session>` so the
//! orchestrator sees the timeout without having to poll.
//!
//! [`write_event`] and [`write_hook_reply`] are the two iii stream
//! writes the gate makes; they live here because the sweeper is their
//! primary caller (the resolve flow also uses them, but their shape is
//! tied to the events-stream contract that the sweeper owns).

use std::sync::Arc;

use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

use crate::lifecycle::collect_timed_out_for_sweep;
use crate::state::StateBus;

/// Lightweight unique-ish id without pulling uuid in: ns timestamp + counter.
/// Used as the `item_id` for stream writes so two appends from the same
/// process don't collide.
pub(crate) fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{n:x}")
}

/// Append `event` to the `agent::events` stream for `session_id`. Used by
/// the sweeper (timeout flips) and by the resolve closure (post-resolve
/// `approval_resolved` frame). Fire-and-forget: errors are swallowed
/// because the persisted record is the source of truth — orchestrators
/// re-derive state from `approval::list_undelivered` if a frame is lost.
pub(crate) async fn write_event(iii: &III, session_id: &str, event: &Value) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
                "item_id": format!("approval-{}", uuid_like()),
                "data": event,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

/// Build the `approval_resolved` event a sweeper emits when it auto-flips an
/// expired pending record. Pure — caller pumps the result onto the stream.
pub(crate) fn timeout_resolved_event(function_call_id: &str) -> Value {
    // Timed-out approvals carry no Denial — the `status: "timed_out"` is
    // self-describing per the Denial refactor. Consumers (turn-orchestrator
    // stitching, UIs) render the timeout from the status alone.
    json!({
        "type": "approval_resolved",
        "function_call_id": function_call_id,
        "tool_call_id": function_call_id,
        "decision": "deny",
        "status": "timed_out",
    })
}

/// Spawn the periodic timeout sweeper. The task ticks every `interval_ms`,
/// scans the configured state scope, and for any pending record whose
/// `expires_at` is in the past: writes the flipped record back and emits an
/// `approval_resolved` (status=timed_out) frame on `agent::events/<session>`.
///
/// Active sweeping closes the gap left by lazy flips: operators who never
/// open the UI for a session would otherwise leave its pending rows in
/// `pending` forever and the paused orchestrator would never see a
/// decision.
pub(crate) fn spawn_timeout_sweeper(
    iii: III,
    bus: Arc<dyn StateBus>,
    state_scope: String,
    interval_ms: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_millis(interval_ms.max(50)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Drop the immediate first tick so we don't sweep before any
        // pending row could possibly exist.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let all = bus.list_prefix(&state_scope, "").await;
            for (key, flipped, session_id, call_id) in collect_timed_out_for_sweep(&all, now_ms) {
                if let Err(err) = bus.set(&state_scope, &key, flipped).await {
                    tracing::warn!(
                        "approval-gate sweeper: failed to flip {key} → timed_out: {err}"
                    );
                    continue;
                }
                write_event(&iii, &session_id, &timeout_resolved_event(&call_id)).await;
            }
        }
    })
}

/// Append a hook reply onto `stream_name` keyed by `event_id`. No-op when
/// either id is empty so a malformed envelope can't crash the gate.
pub(crate) async fn write_hook_reply(iii: &III, stream_name: &str, event_id: &str, reply: &Value) {
    if stream_name.is_empty() || event_id.is_empty() {
        return;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": stream_name,
                "group_id": event_id,
                "item_id": uuid_like(),
                "data": reply,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_resolved_event_shape() {
        let evt = timeout_resolved_event("tc-1");
        assert_eq!(evt["type"], "approval_resolved");
        assert_eq!(evt["function_call_id"], "tc-1");
        assert_eq!(evt["tool_call_id"], "tc-1");
        assert_eq!(evt["decision"], "deny");
        assert_eq!(evt["status"], "timed_out");
        // timed_out is self-describing — no Denial / no legacy reason.
        assert!(evt.get("decision_reason").is_none());
        assert!(evt.get("denial").is_none());
    }
}
