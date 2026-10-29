//! The orchestration events the harness emits, each a private function whose
//! handler returns the event it is called with: `harness::on-ready` after
//! boot, `harness::on-turn-started` / `harness::on-turn-completed` at turn
//! boundaries, `harness::on-message-queued` when a message parks in the
//! mid-turn queue, `harness::on-observers-changed` when a session's observer
//! set or fire count changes, and `harness::on-compaction` when an explicit
//! compaction starts and finishes (harness.md § Events emitted). Consumers
//! observe them to react without polling.
//!
//! The harness emits an event by calling its function with `CallAction::Void`,
//! after one `is_observed` check; iii delivers it to every observer whose
//! filters match. None of these functions takes args: consumers select a
//! session with filters on `input.session_id`, and a sub-agent's parent with
//! `input.parent.session_id` or `input.parent_session_id`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, ObserveRejection, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::surface::schema_value;
use crate::types::turn::ParentLink;

pub const ON_TURN_STARTED_FN: &str = "harness::on-turn-started";
pub const ON_TURN_COMPLETED_FN: &str = "harness::on-turn-completed";
pub const ON_MESSAGE_QUEUED_FN: &str = "harness::on-message-queued";
pub const ON_OBSERVERS_CHANGED_FN: &str = "harness::on-observers-changed";
pub const ON_READY_FN: &str = "harness::on-ready";
pub const ON_COMPACTION_FN: &str = "harness::on-compaction";

/// `harness::on-ready`: the harness completed boot and accepts turns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadyEvent {
    /// Always `ready`.
    pub status: String,
    pub timestamp: i64,
}

/// `harness::on-turn-started`: a turn began executing (first loop step).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TurnStartedEvent {
    pub session_id: String,
    pub turn_id: String,
    pub timestamp: i64,
    /// 0 for a top-level turn.
    pub depth: u32,
    /// The first characters of the message that started the turn (a
    /// sub-agent's task text for a child).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
    /// The spawning call, for a sub-agent turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
    /// The session this one is displayed under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
}

/// `harness::on-turn-completed`: a turn reached a terminal status.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TurnCompletedEvent {
    pub session_id: String,
    pub turn_id: String,
    /// `completed`, `cancelled` or `failed`.
    pub status: String,
    /// `false` while the session still owns an armed wake: the run continues
    /// and a later turn carries the real outcome.
    pub terminal: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// The latest generation's context accounting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::context_snapshot::ContextSnapshotV1>,
}

/// `harness::on-message-queued`: a pointer to a message parked in the
/// session's queue; consumers refetch `harness::status` → `queued`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageQueuedEvent {
    pub session_id: String,
    pub entry_id: String,
    pub queued_at: i64,
    pub timestamp: i64,
}

/// `harness::on-observers-changed`: a doorbell; consumers refetch
/// `harness::observers::list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObserversChangedEvent {
    pub session_id: String,
    pub timestamp: i64,
}

/// `harness::on-compaction`: an explicit compaction's progress.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactionEvent {
    pub session_id: String,
    pub compaction_id: String,
    /// `started` or `completed`.
    pub phase: String,
    pub timestamp: i64,
    /// `completed` only: `ok`, `busy`, `empty` or `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// `completed` only: the user-facing message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
}

/// Registers one event function: private, and its handler returns the event.
fn register_event(iii: &IIIClient, id: &str, description: &str, format: Value) {
    iii.register_function(
        id,
        RegisterFunction::new(|event: Value| Ok::<Value, Error>(event))
            .description(description)
            .request_format(format.clone())
            .response_format(format)
            .private(),
    );
}

/// An observer of a turn event never targets `harness::spawn`: delivery
/// never creates an agent, so the observer is refused when it is added
/// rather than no-op (or worse) on every turn.
async fn refuse_spawn_target(obs: Observer) -> Result<(), ObserveRejection> {
    if obs.target() == crate::functions::SPAWN_ID {
        return Err(ObserveRejection::new(
            "FORBIDDEN",
            "harness::spawn is not an observer target: delivery never creates an agent. Spawn \
             children directly and watch what they write.",
        ));
    }
    Ok(())
}

/// The harness's event emitter. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct TurnEvents {
    iii: Arc<IIIClient>,
    ready: Arc<AtomicBool>,
}

impl TurnEvents {
    /// Set the event functions' hooks, then register them, and return the
    /// emitter.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let ready = Arc::new(AtomicBool::new(false));

        // An observer added after boot receives the current ready state
        // through itself once it is active.
        iii.on_observer_added_for(ON_READY_FN, {
            let iii = iii.clone();
            let ready = ready.clone();
            move |obs: Observer| {
                let iii = iii.clone();
                let ready = ready.load(Ordering::Acquire);
                async move {
                    if ready {
                        tokio::spawn(send_ready(iii, obs));
                    }
                    Ok::<(), ObserveRejection>(())
                }
            }
        });
        iii.on_observer_added_for(ON_TURN_STARTED_FN, refuse_spawn_target);
        iii.on_observer_added_for(ON_TURN_COMPLETED_FN, refuse_spawn_target);

        register_event(
            iii,
            ON_READY_FN,
            "The harness completed boot and can accept turns. An observer added later receives \
             the current ready event when it becomes active.",
            schema_value::<ReadyEvent>(),
        );
        register_event(
            iii,
            ON_TURN_STARTED_FN,
            "A harness turn began executing (first loop step). Filter on input.session_id, or \
             on input.parent.session_id / input.parent_session_id for a sub-agent's parent.",
            schema_value::<TurnStartedEvent>(),
        );
        register_event(
            iii,
            ON_TURN_COMPLETED_FN,
            "A harness turn reached a terminal status (completed/cancelled/failed). Filter on \
             input.session_id, or on input.parent.session_id / input.parent_session_id.",
            schema_value::<TurnCompletedEvent>(),
        );
        register_event(
            iii,
            ON_MESSAGE_QUEUED_FN,
            "A message parked in a session's server-side queue while its turn streams. Filter on \
             input.session_id.",
            schema_value::<MessageQueuedEvent>(),
        );
        register_event(
            iii,
            ON_OBSERVERS_CHANGED_FN,
            "A session's observer set or fire count changed: refetch harness::observers::list. \
             Filter on input.session_id.",
            schema_value::<ObserversChangedEvent>(),
        );
        crate::deletion_events::register(iii);
        register_event(
            iii,
            ON_COMPACTION_FN,
            "An explicit compaction of a session started (`phase: started`) or finished \
             (`phase: completed`, with `status` ok/busy/empty/failed and the user-facing \
             `message`). Filter on input.session_id or input.compaction_id.",
            schema_value::<CompactionEvent>(),
        );
        tracing::info!(
            "registered harness::on-ready / on-turn-started / on-turn-completed / \
             on-message-queued / on-observers-changed / on-session-tree-deletion / on-compaction"
        );

        Self {
            iii: iii.clone(),
            ready,
        }
    }

    /// Announce readiness after all turn dependencies and lifecycle observers
    /// are in place. Observers added later receive it from the added hook.
    pub async fn emit_ready(&self) {
        self.ready.store(true, Ordering::Release);
        self.emit(ON_READY_FN, ready_payload()).await;
    }

    /// A message parked in the session's `harness_queue` (send's queue path).
    /// The payload is a pointer, not the message — consumers refetch
    /// `harness::status` → `queued` (idempotent under at-least-once delivery).
    pub async fn emit_queued(&self, session_id: &str, entry_id: &str, queued_at: i64) {
        tracing::info!(session_id, entry_id, "message queued");
        if !self.iii.is_observed(ON_MESSAGE_QUEUED_FN) {
            return;
        }
        let payload = serde_json::json!({
            "session_id": session_id,
            "entry_id": entry_id,
            "queued_at": queued_at,
            "timestamp": now_ms(),
        });
        self.emit(ON_MESSAGE_QUEUED_FN, payload).await;
    }

    /// Doorbell only — a session's observer set or fires count changed;
    /// consumers refetch `harness::observers::list`. No per-event logging:
    /// a single fire rings twice (claim + retirement).
    pub async fn emit_observers_changed(&self, session_id: &str) {
        if !self.iii.is_observed(ON_OBSERVERS_CHANGED_FN) {
            return;
        }
        let payload = serde_json::json!({
            "session_id": session_id,
            "timestamp": now_ms(),
        });
        self.emit(ON_OBSERVERS_CHANGED_FN, payload).await;
    }

    /// An explicit compaction's progress (`crate::compaction::event_payload`).
    /// Every client also gets it durably: the start notice and the outcome
    /// row land in the transcript; this is the live signal for consumers that
    /// track the operation itself.
    pub async fn emit_compaction(&self, _session_id: &str, payload: Value) {
        self.emit(ON_COMPACTION_FN, payload).await;
    }

    /// `message_preview` is the first characters of the user message that
    /// started the turn (a sub-agent's task text for a child), so a consumer
    /// can name the turn without reading the transcript; `depth` is 0 for a
    /// top-level turn.
    pub async fn emit_started(
        &self,
        session_id: &str,
        turn_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        message_preview: Option<&str>,
        depth: u32,
    ) {
        tracing::info!(session_id, turn_id, "turn started");
        if !self.iii.is_observed(ON_TURN_STARTED_FN) {
            return;
        }
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "timestamp": now_ms(),
            "depth": depth,
        });
        if let Some(preview) = message_preview.filter(|p| !p.trim().is_empty()) {
            payload["message_preview"] = Value::String(preview.to_string());
        }
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        if let Some(dp) = display_parent {
            payload["parent_session_id"] = Value::String(dp.to_string());
        }
        self.emit(ON_TURN_STARTED_FN, payload).await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn emit_completed(
        &self,
        session_id: &str,
        turn_id: &str,
        status: &str,
        result: Option<&Value>,
        result_error: Option<&str>,
        reason: Option<&str>,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        terminal: bool,
        context: Option<&crate::context_snapshot::ContextSnapshotV1>,
    ) {
        tracing::info!(
            session_id,
            turn_id,
            status,
            result_error,
            terminal,
            "turn completed"
        );
        if !self.iii.is_observed(ON_TURN_COMPLETED_FN) {
            return;
        }
        // `terminal: false` marks a turn whose session still owns an armed
        // wake (a one-shot notify) — the run continues and a later turn in the
        // same session carries the real outcome. Consumers should treat
        // non-terminal completions as progress and finalize only on
        // `terminal: true`.
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "status": status,
            "terminal": terminal,
            "timestamp": now_ms(),
        });
        if let Some(r) = result {
            payload["result"] = r.clone();
        }
        if let Some(e) = result_error {
            payload["result_error"] = Value::String(e.to_string());
        }
        if let Some(r) = reason {
            payload["reason"] = Value::String(r.to_string());
        }
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        if let Some(dp) = display_parent {
            payload["parent_session_id"] = Value::String(dp.to_string());
        }
        // The latest generation's context accounting (categories, budget,
        // usage) so live consumers never re-walk the transcript for it.
        if let Some(snapshot) = context {
            payload["context"] = serde_json::to_value(snapshot).unwrap_or(Value::Null);
        }
        self.emit(ON_TURN_COMPLETED_FN, payload).await;
    }

    /// Call the event function, fire-and-forget: iii delivers the call to
    /// each observer whose filters match it. Nothing is built or sent when
    /// nobody observes the function.
    async fn emit(&self, function_id: &str, payload: Value) {
        if !self.iii.is_observed(function_id) {
            return;
        }
        let request = CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = self.iii.call(request).await {
            tracing::warn!(function_id, %error, "harness event call failed");
        }
    }
}

fn ready_payload() -> Value {
    serde_json::json!({
        "status": "ready",
        "timestamp": now_ms(),
    })
}

/// Send the ready event through `obs` once the observer is active.
async fn send_ready(iii: Arc<IIIClient>, obs: Observer) {
    if obs.accepted().await.is_err() {
        return;
    }
    let request = CallRequest::observer(&obs, ready_payload()).action(CallAction::Void);
    if let Err(error) = iii.call(request).await {
        tracing::warn!(
            function_id = ON_READY_FN,
            observer = %obs.id(),
            %error,
            "ready event for a new observer failed"
        );
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The published formats describe exactly what the emitters send, so a
    /// consumer's filters on `input.<field>` resolve against a real schema.
    #[test]
    fn event_formats_round_trip_the_emitted_payloads() {
        let started: TurnStartedEvent = serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "timestamp": 1,
            "depth": 1,
            "message_preview": "go",
            "parent": { "session_id": "p_1", "turn_id": "t", "function_call_id": "fc" },
            "parent_session_id": "root_1",
        }))
        .unwrap();
        assert_eq!(started.parent.unwrap().session_id, "p_1");

        let completed: TurnCompletedEvent = serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "status": "completed",
            "terminal": false,
            "timestamp": 2,
        }))
        .unwrap();
        assert!(!completed.terminal);

        let compaction: CompactionEvent = serde_json::from_value(json!({
            "session_id": "s_1",
            "compaction_id": "c_1",
            "phase": "completed",
            "timestamp": 3,
            "status": "ok",
            "message": "compacted",
            "entry_ids": ["e_1"],
        }))
        .unwrap();
        assert_eq!(compaction.status.as_deref(), Some("ok"));
    }

    #[test]
    fn every_event_function_follows_the_on_naming() {
        for id in [
            ON_READY_FN,
            ON_TURN_STARTED_FN,
            ON_TURN_COMPLETED_FN,
            ON_MESSAGE_QUEUED_FN,
            ON_OBSERVERS_CHANGED_FN,
            ON_COMPACTION_FN,
        ] {
            assert!(id.starts_with("harness::on-"), "{id}");
        }
    }

    #[test]
    fn ready_payload_says_ready() {
        assert_eq!(ready_payload()["status"], json!("ready"));
    }
}
