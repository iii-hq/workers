//! `run::start` — durable session entrypoint.

use std::sync::Arc;

use harness_types::{AgentEvent, AgentMessage};
use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::awaiting::AwaitingApproval;
use crate::config::TurnOrchestratorConfig;
use crate::events;
use crate::persistence;
use crate::state::TurnStateRecord;

pub const FUNCTION_ID: &str = "run::start";
pub const SYNC_FUNCTION_ID: &str = "run::start_and_wait";
pub const RESUME_FUNCTION_ID: &str = "run::resume";
pub const STEP_TOPIC: &str = "turn::step_requested";

pub async fn execute(iii: III, payload: Value) -> Result<Value, IIIError> {
    let session_id = required_str(&payload, "session_id")?;
    let max_turns = payload
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    let request = build_run_request(&payload);
    let initial_messages = decode_initial_messages(&payload)?;

    persistence::save_run_request(&iii, &session_id, request.clone()).await;
    persistence::save_messages(&iii, &session_id, &initial_messages).await;

    let record = TurnStateRecord::new(&session_id, max_turns);
    persistence::save_record(&iii, &record).await;

    if let Some(cwd) = request.get("cwd").and_then(Value::as_str) {
        persistence::save_cwd(&iii, &session_id, cwd).await;
        if let Some(cwd_hash) = request.get("cwd_hash").and_then(Value::as_str) {
            persistence::save_cwd_index(&iii, cwd_hash, &session_id).await;
        }
    }

    // Emit AgentStart and initial-message events BEFORE publishing the
    // first step. This ordering matches the legacy `run_loop` in
    // `provider-router/src/loop_state.rs:73-80` so consumers see the same
    // prefix on the stream regardless of which entrypoint they triggered.
    for evt in build_initial_event_plan(&initial_messages) {
        events::emit(&iii, &session_id, &evt).await;
    }

    publish_step(&iii, &session_id).await;

    Ok(json!({ "session_id": session_id }))
}

fn decode_initial_messages(payload: &Value) -> Result<Vec<AgentMessage>, IIIError> {
    serde_json::from_value(
        payload
            .get("messages")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )
    .map_err(|e| IIIError::Handler(format!("decode messages: {e}")))
}

/// Pure helper: produce the request envelope persisted for later resume.
///
/// `tools` is intentionally NOT read from the payload: the catalog is
/// rebuilt from `engine::functions::list` in the provisioning state, so
/// any `tools` the caller supplies is silently ignored.
fn build_run_request(payload: &Value) -> Value {
    json!({
        "provider": payload.get("provider").cloned().unwrap_or_else(|| json!("")),
        "model": payload.get("model").cloned().unwrap_or_else(|| json!("")),
        "system_prompt": payload.get("system_prompt").cloned().unwrap_or_else(|| json!("")),
        "approval_required": payload
            .get("approval_required")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "image": payload.get("image").cloned().unwrap_or_else(|| json!("python")),
        "idle_timeout_secs": payload.get("idle_timeout_secs").cloned().unwrap_or_else(|| json!(300)),
        "cwd": payload.get("cwd").cloned().unwrap_or(Value::Null),
        "cwd_hash": payload.get("cwd_hash").cloned().unwrap_or(Value::Null),
    })
}

/// Pure helper: produce the ordered list of [`AgentEvent`]s that
/// `execute` emits before the first state transition. Decoupled so it
/// can be unit-tested without a live engine.
fn build_initial_event_plan(initial_messages: &[AgentMessage]) -> Vec<AgentEvent> {
    let mut plan = Vec::with_capacity(1 + initial_messages.len() * 2);
    plan.push(AgentEvent::AgentStart);
    for m in initial_messages {
        plan.push(AgentEvent::MessageStart { message: m.clone() });
        plan.push(AgentEvent::MessageEnd { message: m.clone() });
    }
    plan
}

pub async fn execute_sync(
    iii: III,
    cfg: Arc<TurnOrchestratorConfig>,
    payload: Value,
) -> Result<Value, IIIError> {
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(cfg.sync_default_timeout_ms);

    let started = execute(iii.clone(), payload).await?;
    let session_id = started
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| IIIError::Handler("session_id missing".into()))?
        .to_string();

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if let Some(record) = persistence::load_record(&iii, &session_id).await {
            if record.is_terminal() {
                let messages = persistence::load_messages(&iii, &session_id).await;
                return Ok(json!({
                    "session_id": session_id,
                    "messages": messages,
                    "turn_count": record.turn_count,
                }));
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(IIIError::Handler(format!(
                "run::start_and_wait timed out after {timeout_ms} ms"
            )));
        }
        tokio::time::sleep(std::time::Duration::from_millis(cfg.sync_poll_interval_ms)).await;
    }
}

pub(crate) fn build_resume_record(existing: &TurnStateRecord) -> TurnStateRecord {
    let mut resumed = TurnStateRecord::new(existing.session_id.clone(), existing.max_turns);
    resumed.turn_count = existing.turn_count;
    resumed
}

pub(crate) fn build_resume_plan(existing: &TurnStateRecord) -> Option<TurnStateRecord> {
    existing.is_terminal().then(|| build_resume_record(existing))
}

/// Max time `execute_resume` will wait for the executor to park the
/// session. The happy path completes within milliseconds — this is the
/// failure ceiling, not the expected latency.
const RESUME_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn execute_resume(
    iii: III,
    awaiting: AwaitingApproval,
    payload: Value,
) -> Result<Value, IIIError> {
    let session_id = required_str(&payload, "session_id")?;

    if let Some(plan) = try_resume_now(&iii, &session_id).await? {
        return finish_resume(&iii, &awaiting, &session_id, plan).await;
    }

    // Arm a Notified future *before* the second load so a signal that
    // fires between the two checks is captured. Same lost-wake guard
    // pattern as tokio::sync::Notify's documented usage.
    let slot = awaiting.slot(&session_id);
    let notified = slot.notified();
    tokio::pin!(notified);

    if let Some(plan) = try_resume_now(&iii, &session_id).await? {
        return finish_resume(&iii, &awaiting, &session_id, plan).await;
    }

    match tokio::time::timeout(RESUME_WAIT_TIMEOUT, notified).await {
        Ok(()) => {
            if let Some(plan) = try_resume_now(&iii, &session_id).await? {
                finish_resume(&iii, &awaiting, &session_id, plan).await
            } else {
                Ok(json!({
                    "ok": true,
                    "session_id": session_id,
                    "resumed": false,
                }))
            }
        }
        Err(_) => Err(IIIError::Handler(format!(
            "run::resume timed out waiting for session {session_id} to park"
        ))),
    }
}

async fn try_resume_now(iii: &III, session_id: &str) -> Result<Option<TurnStateRecord>, IIIError> {
    let existing = persistence::load_record(iii, session_id)
        .await
        .ok_or_else(|| IIIError::Handler(format!("unknown session: {session_id}")))?;
    Ok(build_resume_plan(&existing))
}

async fn finish_resume(
    iii: &III,
    awaiting: &AwaitingApproval,
    session_id: &str,
    plan: TurnStateRecord,
) -> Result<Value, IIIError> {
    persistence::save_record(iii, &plan).await;
    publish_step(iii, session_id).await;
    awaiting.clear(session_id);
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "resumed": true,
    }))
}

pub async fn publish_step(iii: &III, session_id: &str) {
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "iii::durable::publish".into(),
            payload: json!({
                "topic": STEP_TOPIC,
                "data": { "session_id": session_id },
            }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %session_id, "turn::step_requested publish failed");
    }
}

pub fn register(iii: &III, cfg: &Arc<TurnOrchestratorConfig>, awaiting: AwaitingApproval) {
    let iii_async = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string())
            .with_description("Start a durable agent session and return immediately.".to_string()),
        move |payload: Value| {
            let iii = iii_async.clone();
            async move { execute(iii, payload).await }
        },
    ));
    let iii_sync = iii.clone();
    let cfg_sync = cfg.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(SYNC_FUNCTION_ID.to_string()).with_description(
            "Start a durable agent session and block until terminal (test/dev convenience)."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_sync.clone();
            let cfg = cfg_sync.clone();
            async move { execute_sync(iii, cfg, payload).await }
        },
    ));
    let iii_resume = iii.clone();
    let awaiting_resume = awaiting.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(RESUME_FUNCTION_ID.to_string()).with_description(
            "Resume an idled approval session and publish a turn step.".to_string(),
        ),
        move |payload: Value| {
            let iii = iii_resume.clone();
            let awaiting = awaiting_resume.clone();
            async move { execute_resume(iii, awaiting, payload).await }
        },
    ));
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{AgentEvent, AgentMessage, ContentBlock, TextContent, UserMessage};

    fn user_msg(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: text.to_string(),
            })],
            timestamp: 0,
        })
    }

    #[test]
    fn build_initial_event_plan_starts_with_agent_start() {
        let plan = build_initial_event_plan(&[user_msg("hi"), user_msg("again")]);
        assert!(matches!(plan.first(), Some(AgentEvent::AgentStart)));
    }

    #[test]
    fn build_initial_event_plan_has_pair_per_message() {
        let plan = build_initial_event_plan(&[user_msg("hi"), user_msg("again")]);
        // 1 AgentStart + 2 messages * (MessageStart + MessageEnd) = 5
        assert_eq!(plan.len(), 5);
    }

    #[test]
    fn build_initial_event_plan_handles_empty_messages() {
        let plan = build_initial_event_plan(&[]);
        assert_eq!(plan.len(), 1);
        assert!(matches!(plan[0], AgentEvent::AgentStart));
    }

    #[test]
    fn build_run_request_preserves_optional_cwd_and_cwd_hash() {
        let request = build_run_request(&json!({
            "provider": "openai",
            "model": "gpt-test",
            "cwd": "/tmp/project",
            "cwd_hash": "abc123",
        }));

        assert_eq!(request["cwd"], json!("/tmp/project"));
        assert_eq!(request["cwd_hash"], json!("abc123"));
    }

    #[test]
    fn build_run_request_defaults_absent_cwd_metadata_to_null() {
        let request = build_run_request(&json!({}));

        assert_eq!(request["cwd"], Value::Null);
        assert_eq!(request["cwd_hash"], Value::Null);
    }

    #[test]
    fn build_resume_record_reopens_terminal_record_without_resetting_budget() {
        let mut stopped = TurnStateRecord::new("sess-pending", Some(4));
        stopped.turn_count = 3;
        stopped.transition_to(crate::state::TurnState::Stopped);

        let resumed = build_resume_record(&stopped);

        assert_eq!(resumed.session_id, "sess-pending");
        assert_eq!(resumed.state, crate::state::TurnState::Provisioning);
        assert_eq!(resumed.turn_count, 3);
        assert_eq!(resumed.max_turns, Some(4));
        assert!(!resumed.is_terminal());
        assert!(resumed.last_assistant.is_none());
        assert!(resumed.pending_function_calls.is_empty());
    }

    #[test]
    fn build_resume_plan_reopens_only_terminal_records() {
        let mut stopped = TurnStateRecord::new("sess-stopped", None);
        stopped.transition_to(crate::state::TurnState::Stopped);
        assert!(build_resume_plan(&stopped).is_some());

        let mut active = TurnStateRecord::new("sess-active", None);
        active.transition_to(crate::state::TurnState::FunctionExecute);
        assert!(
            build_resume_plan(&active).is_none(),
            "run::resume must not publish another step while a turn is already active"
        );
    }

    #[test]
    fn build_run_request_propagates_approval_required() {
        let request = build_run_request(&json!({
            "approval_required": ["shell::fs::write"],
        }));
        assert_eq!(request["approval_required"], json!(["shell::fs::write"]),);
    }

    #[test]
    fn build_run_request_defaults_approval_required_to_empty() {
        let request = build_run_request(&json!({}));
        assert_eq!(request["approval_required"], json!([]));
    }

    #[test]
    fn build_run_request_drops_caller_supplied_tools() {
        // The catalog is rebuilt server-side from engine::functions::list,
        // so any `tools` the caller puts in the run::start payload must
        // not survive into the persisted request envelope.
        let request = build_run_request(&json!({
            "provider": "openai",
            "tools": [{ "name": "stale_tool", "description": "x" }],
        }));
        assert!(
            request.get("tools").is_none(),
            "expected `tools` to be dropped from the persisted run request envelope"
        );
    }

    #[test]
    fn decode_initial_messages_rejects_malformed_messages() {
        let result = decode_initial_messages(&json!({
            "messages": "not an array",
        }));

        assert!(result.is_err());
    }
}
