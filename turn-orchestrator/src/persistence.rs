//! State load/save helpers. Each helper runs one or two `state::*` triggers
//! and never panics; missing keys deserialise to defaults so callers can
//! treat first-time and retry paths the same way.

use harness_types::{AgentMessage, ToolCall, ToolResult};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::{json, Value as JsonValue};

use crate::state::{
    cwd_index_key, cwd_key, messages_key, run_request_key, sandbox_id_key, tool_schemas_key,
    turn_state_key, TurnStateRecord,
};

const STATE_SCOPE: &str = "agent";

pub async fn load_record(iii: &III, session_id: &str) -> Option<TurnStateRecord> {
    let key = turn_state_key(session_id);
    let value = state_get(iii, &key).await?;
    serde_json::from_value(value).ok()
}

pub async fn save_record(iii: &III, record: &TurnStateRecord) {
    let key = turn_state_key(&record.session_id);
    if let Ok(value) = serde_json::to_value(record) {
        state_set(iii, &key, value).await;
    }
}

pub async fn load_messages(iii: &III, session_id: &str) -> Vec<AgentMessage> {
    let key = messages_key(session_id);
    let Some(value) = state_get(iii, &key).await else {
        return Vec::new();
    };
    serde_json::from_value(value).unwrap_or_default()
}

pub async fn save_messages(iii: &III, session_id: &str, messages: &[AgentMessage]) {
    let key = messages_key(session_id);
    if let Ok(value) = serde_json::to_value(messages) {
        state_set(iii, &key, value).await;
    }
}

pub async fn save_run_request(iii: &III, session_id: &str, request: JsonValue) {
    let key = run_request_key(session_id);
    state_set(iii, &key, request).await;
}

pub async fn load_run_request(iii: &III, session_id: &str) -> JsonValue {
    state_get(iii, &run_request_key(session_id))
        .await
        .unwrap_or_else(|| json!({}))
}

pub async fn save_cwd(iii: &III, session_id: &str, cwd: &str) {
    state_set(
        iii,
        &cwd_key(session_id),
        JsonValue::String(cwd.to_string()),
    )
    .await;
}

pub async fn load_cwd(iii: &III, session_id: &str) -> Option<String> {
    state_get(iii, &cwd_key(session_id))
        .await
        .and_then(|v| v.as_str().map(str::to_string))
}

pub async fn save_cwd_index(iii: &III, cwd_hash: &str, session_id: &str) {
    state_set(
        iii,
        &cwd_index_key(cwd_hash),
        JsonValue::String(session_id.to_string()),
    )
    .await;
}

pub async fn save_sandbox_id(iii: &III, session_id: &str, sandbox_id: Option<&str>) {
    let key = sandbox_id_key(session_id);
    let value = sandbox_id.map_or(JsonValue::Null, |s| JsonValue::String(s.to_string()));
    state_set(iii, &key, value).await;
}

pub async fn load_sandbox_id(iii: &III, session_id: &str) -> Option<String> {
    state_get(iii, &sandbox_id_key(session_id))
        .await
        .and_then(|v| v.as_str().map(str::to_string))
}

pub async fn save_tool_schemas(iii: &III, session_id: &str, schemas: JsonValue) {
    state_set(iii, &tool_schemas_key(session_id), schemas).await;
}

pub async fn load_tool_schemas(iii: &III, session_id: &str) -> JsonValue {
    state_get(iii, &tool_schemas_key(session_id))
        .await
        .unwrap_or_else(|| json!([]))
}

async fn state_get(iii: &III, key: &str) -> Option<Value> {
    match iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(v) if v.is_null() => None,
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, %key, "turn-orchestrator: state::get failed");
            None
        }
    }
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
        tracing::warn!(error = %e, %key, "turn-orchestrator: state::set failed");
    }
}

const PREPARED_KEY: &str = "tool_prepared";
const EXECUTED_KEY: &str = "tool_executed";

fn staging_key(session_id: &str, suffix: &str) -> String {
    format!("session/{session_id}/{suffix}")
}

pub async fn save_prepared_calls(
    iii: &III,
    session_id: &str,
    prepared: &[(ToolCall, Option<ToolResult>)],
) {
    let payload = serde_json::to_value(
        prepared
            .iter()
            .map(|(tc, pre)| json!({ "tool_call": tc, "blocked": pre }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| json!([]));
    state_set(iii, &staging_key(session_id, PREPARED_KEY), payload).await;
}

pub async fn load_prepared_calls(
    iii: &III,
    session_id: &str,
) -> Vec<(ToolCall, Option<ToolResult>)> {
    let value = state_get(iii, &staging_key(session_id, PREPARED_KEY))
        .await
        .unwrap_or_else(|| json!([]));
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let tc = serde_json::from_value::<ToolCall>(entry.get("tool_call")?.clone()).ok()?;
            let pre = entry
                .get("blocked")
                .and_then(|v| serde_json::from_value::<Option<ToolResult>>(v.clone()).ok())
                .unwrap_or(None);
            Some((tc, pre))
        })
        .collect()
}

pub async fn save_executed_calls(
    iii: &III,
    session_id: &str,
    executed: &[(ToolCall, ToolResult, bool)],
) {
    let payload = serde_json::to_value(
        executed
            .iter()
            .map(|(tc, r, e)| json!({ "tool_call": tc, "result": r, "is_error": e }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| json!([]));
    state_set(iii, &staging_key(session_id, EXECUTED_KEY), payload).await;
}

pub async fn load_executed_calls(iii: &III, session_id: &str) -> Vec<(ToolCall, ToolResult, bool)> {
    let value = state_get(iii, &staging_key(session_id, EXECUTED_KEY))
        .await
        .unwrap_or_else(|| json!([]));
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let tc = serde_json::from_value::<ToolCall>(entry.get("tool_call")?.clone()).ok()?;
            let r = serde_json::from_value::<ToolResult>(entry.get("result")?.clone()).ok()?;
            let e = entry
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some((tc, r, e))
        })
        .collect()
}

pub fn find_executed_call<'a>(
    executed: &'a [(ToolCall, ToolResult, bool)],
    tool_call_id: &str,
) -> Option<&'a (ToolCall, ToolResult, bool)> {
    executed.iter().find(|(tc, _, _)| tc.id == tool_call_id)
}

pub fn upsert_executed_call(
    executed: &mut Vec<(ToolCall, ToolResult, bool)>,
    entry: (ToolCall, ToolResult, bool),
) {
    if let Some(existing) = executed.iter_mut().find(|(tc, _, _)| tc.id == entry.0.id) {
        *existing = entry;
    } else {
        executed.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TurnState;
    use harness_types::{ContentBlock, TextContent};

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: json!({ "id": id }),
        }
    }

    fn tool_result(text: &str) -> ToolResult {
        ToolResult {
            content: vec![ContentBlock::Text(TextContent { text: text.into() })],
            details: json!({ "text": text }),
            terminate: false,
        }
    }

    #[test]
    fn record_round_trips_through_json() {
        let mut r = TurnStateRecord::new("s1", Some(8));
        r.transition_to(TurnState::AwaitingAssistant);
        let v = serde_json::to_value(&r).unwrap();
        let back: TurnStateRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back.state, TurnState::AwaitingAssistant);
        assert_eq!(back.session_id, "s1");
        assert_eq!(back.max_turns, Some(8));
    }

    #[test]
    fn find_executed_call_matches_tool_call_id() {
        let executed = vec![
            (tool_call("tc-1", "read"), tool_result("one"), false),
            (tool_call("tc-2", "write"), tool_result("two"), true),
        ];

        let found = find_executed_call(&executed, "tc-2").expect("expected tc-2");

        assert_eq!(found.0.id, "tc-2");
        assert_eq!(found.0.name, "write");
        assert!(found.2);
        assert!(find_executed_call(&executed, "missing").is_none());
    }

    #[test]
    fn upsert_executed_call_preserves_order_and_replaces_existing() {
        let mut executed = vec![
            (tool_call("tc-1", "read"), tool_result("one"), false),
            (tool_call("tc-2", "write"), tool_result("two"), true),
        ];

        upsert_executed_call(
            &mut executed,
            (
                tool_call("tc-2", "write"),
                tool_result("replacement"),
                false,
            ),
        );
        upsert_executed_call(
            &mut executed,
            (tool_call("tc-3", "list"), tool_result("three"), false),
        );

        assert_eq!(executed.len(), 3);
        assert_eq!(executed[0].0.id, "tc-1");
        assert_eq!(executed[1].0.id, "tc-2");
        assert_eq!(executed[2].0.id, "tc-3");
        assert!(!executed[1].2);
        assert!(matches!(
            executed[1].1.content.first(),
            Some(ContentBlock::Text(text)) if text.text == "replacement"
        ));
    }
}
