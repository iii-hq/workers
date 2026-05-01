//! `run::start` — durable session entrypoint.

use harness_types::{AgentEvent, AgentMessage};
use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::TurnStateRecord;

pub const FUNCTION_ID: &str = "run::start";
pub const SYNC_FUNCTION_ID: &str = "run::start_and_wait";
pub const STEP_TOPIC: &str = "turn::step_requested";

const POLL_INTERVAL_MS: u64 = 50;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 120_000;

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
    // `harness-runtime/src/loop_state.rs:73-80` so consumers see the same
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
fn build_run_request(payload: &Value) -> Value {
    json!({
        "provider": payload.get("provider").cloned().unwrap_or_else(|| json!("")),
        "model": payload.get("model").cloned().unwrap_or_else(|| json!("")),
        "system_prompt": payload.get("system_prompt").cloned().unwrap_or_else(|| json!("")),
        "tools": payload.get("tools").cloned().unwrap_or_else(|| json!([])),
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

pub async fn execute_sync(iii: III, payload: Value) -> Result<Value, IIIError> {
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);

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
        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

pub async fn publish_step(iii: &III, session_id: &str) {
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "publish".into(),
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

pub fn register(iii: &III) {
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
    iii.register_function((
        RegisterFunctionMessage::with_id(SYNC_FUNCTION_ID.to_string()).with_description(
            "Start a durable agent session and block until terminal (test/dev convenience)."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_sync.clone();
            async move { execute_sync(iii, payload).await }
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
    fn decode_initial_messages_rejects_malformed_messages() {
        let result = decode_initial_messages(&json!({
            "messages": "not an array",
        }));

        assert!(result.is_err());
    }
}
