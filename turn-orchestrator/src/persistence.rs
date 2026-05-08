//! State load/save helpers. Each helper runs one or two `state::*` triggers
//! and never panics; missing keys deserialise to defaults so callers can
//! treat first-time and retry paths the same way.

use harness_types::{AgentMessage, FunctionCall, FunctionResult};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::{json, Value as JsonValue};

use crate::state::{
    cwd_index_key, cwd_key, function_schemas_key, messages_key, run_request_key, sandbox_id_key,
    tool_schemas_key, turn_state_key, TurnStateRecord,
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
    // Best-effort mirror to session-tree. Failure does not abort the turn.
    mirror_messages_to_session_tree(iii, session_id, messages).await;
}

async fn mirror_messages_to_session_tree(iii: &III, session_id: &str, messages: &[AgentMessage]) {
    let last_key = crate::state::last_session_tree_len_key(session_id);
    let already_mirrored = state_get(iii, &last_key)
        .await
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    if messages.len() <= already_mirrored {
        return;
    }

    if already_mirrored == 0 {
        if let Err(e) = iii
            .trigger(TriggerRequest {
                function_id: "session-tree::ensure".into(),
                payload: json!({ "session_id": session_id }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            tracing::warn!(
                error = %e,
                %session_id,
                "turn-orchestrator: session-tree::ensure failed; mirror skipped"
            );
            return;
        }
    }

    // Find the current leaf entry_id by reading session-tree state — needed
    // so subsequent appends thread parent_id correctly. For a fresh session
    // (already_mirrored == 0), parent_id starts as None.
    let mut last_appended: Option<String> = None;
    if already_mirrored > 0 {
        match iii
            .trigger(TriggerRequest {
                function_id: "session-tree::messages".into(),
                payload: json!({ "session_id": session_id }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            Ok(resp) => {
                last_appended = resp
                    .get("messages")
                    .and_then(|m| m.as_array())
                    .and_then(|arr| arr.last())
                    .and_then(|last| last.get("entry_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %session_id,
                    "turn-orchestrator: session-tree::messages read failed mid-mirror; skipping append batch to avoid orphaning"
                );
                return;
            }
        }
    }

    for msg in &messages[already_mirrored..] {
        let payload = json!({
            "session_id": session_id,
            "parent_id": last_appended,
            "message": msg,
        });
        match iii
            .trigger(TriggerRequest {
                function_id: "session-tree::append".into(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
        {
            Ok(resp) => {
                last_appended = resp
                    .get("entry_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %session_id,
                    "turn-orchestrator: session-tree::append mirror failed"
                );
                return;
            }
        }
    }

    state_set(iii, &last_key, json!(messages.len())).await;
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

pub async fn save_function_schemas(iii: &III, session_id: &str, schemas: JsonValue) {
    state_set(iii, &function_schemas_key(session_id), schemas).await;
}

/// Load function catalog JSON; falls back to legacy `tool_schemas` key when the new key is absent.
pub async fn load_function_schemas(iii: &III, session_id: &str) -> JsonValue {
    let new_key = function_schemas_key(session_id);
    match state_get(iii, &new_key).await {
        Some(v) => v,
        None => state_get(iii, &tool_schemas_key(session_id))
            .await
            .unwrap_or_else(|| json!([])),
    }
}

/// Back-compat name for callers being migrated — prefer [`save_function_schemas`].
#[inline]
pub async fn save_tool_schemas(iii: &III, session_id: &str, schemas: JsonValue) {
    save_function_schemas(iii, session_id, schemas).await;
}

/// Back-compat — prefer [`load_function_schemas`].
#[inline]
pub async fn load_tool_schemas(iii: &III, session_id: &str) -> JsonValue {
    load_function_schemas(iii, session_id).await
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

const PREPARED_KEY: &str = "function_prepared";
const EXECUTED_KEY: &str = "function_executed";
const LEGACY_PREPARED_KEY: &str = "tool_prepared";
const LEGACY_EXECUTED_KEY: &str = "tool_executed";

fn staging_key(session_id: &str, suffix: &str) -> String {
    format!("session/{session_id}/{suffix}")
}

async fn staging_get_with_legacy(
    iii: &III,
    session_id: &str,
    new_suffix: &str,
    legacy_suffix: &str,
) -> JsonValue {
    let new_k = staging_key(session_id, new_suffix);
    match state_get(iii, &new_k).await {
        Some(v) => v,
        None => state_get(iii, &staging_key(session_id, legacy_suffix))
            .await
            .unwrap_or_else(|| json!([])),
    }
}

pub async fn save_prepared_calls(
    iii: &III,
    session_id: &str,
    prepared: &[(FunctionCall, Option<FunctionResult>)],
) {
    let payload = serde_json::to_value(
        prepared
            .iter()
            .map(|(tc, pre)| json!({ "function_call": tc, "blocked": pre }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| json!([]));
    state_set(iii, &staging_key(session_id, PREPARED_KEY), payload).await;
}

pub async fn load_prepared_calls(
    iii: &III,
    session_id: &str,
) -> Vec<(FunctionCall, Option<FunctionResult>)> {
    let value = staging_get_with_legacy(iii, session_id, PREPARED_KEY, LEGACY_PREPARED_KEY).await;
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let fc = entry
                .get("function_call")
                .or_else(|| entry.get("tool_call"))
                .and_then(|v| serde_json::from_value::<FunctionCall>(v.clone()).ok())?;
            let pre = entry
                .get("blocked")
                .and_then(|v| serde_json::from_value::<Option<FunctionResult>>(v.clone()).ok())
                .unwrap_or(None);
            Some((fc, pre))
        })
        .collect()
}

pub async fn save_executed_calls(
    iii: &III,
    session_id: &str,
    executed: &[(FunctionCall, FunctionResult, bool)],
) {
    let payload = serde_json::to_value(
        executed
            .iter()
            .map(|(tc, r, e)| json!({ "function_call": tc, "result": r, "is_error": e }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| json!([]));
    state_set(iii, &staging_key(session_id, EXECUTED_KEY), payload).await;
}

pub async fn load_executed_calls(
    iii: &III,
    session_id: &str,
) -> Vec<(FunctionCall, FunctionResult, bool)> {
    let value = staging_get_with_legacy(iii, session_id, EXECUTED_KEY, LEGACY_EXECUTED_KEY).await;
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| {
            let fc = entry
                .get("function_call")
                .or_else(|| entry.get("tool_call"))
                .and_then(|v| serde_json::from_value::<FunctionCall>(v.clone()).ok())?;
            let r = serde_json::from_value::<FunctionResult>(entry.get("result")?.clone()).ok()?;
            let e = entry
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some((fc, r, e))
        })
        .collect()
}

pub fn find_executed_call<'a>(
    executed: &'a [(FunctionCall, FunctionResult, bool)],
    function_call_id: &str,
) -> Option<&'a (FunctionCall, FunctionResult, bool)> {
    executed.iter().find(|(fc, _, _)| fc.id == function_call_id)
}

pub fn upsert_executed_call(
    executed: &mut Vec<(FunctionCall, FunctionResult, bool)>,
    entry: (FunctionCall, FunctionResult, bool),
) {
    if let Some(existing) = executed.iter_mut().find(|(fc, _, _)| fc.id == entry.0.id) {
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

    fn fc(id: &str, function_id: &str) -> FunctionCall {
        FunctionCall {
            id: id.into(),
            function_id: function_id.into(),
            arguments: json!({ "id": id }),
        }
    }

    fn func_result(text: &str) -> FunctionResult {
        FunctionResult {
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
    fn find_executed_call_matches_function_call_id() {
        let executed = vec![
            (fc("tc-1", "read"), func_result("one"), false),
            (fc("tc-2", "write"), func_result("two"), true),
        ];

        let found = find_executed_call(&executed, "tc-2").expect("expected tc-2");

        assert_eq!(found.0.id, "tc-2");
        assert_eq!(found.0.function_id, "write");
        assert!(found.2);
        assert!(find_executed_call(&executed, "missing").is_none());
    }

    #[test]
    fn upsert_executed_call_preserves_order_and_replaces_existing() {
        let mut executed = vec![
            (fc("tc-1", "read"), func_result("one"), false),
            (fc("tc-2", "write"), func_result("two"), true),
        ];

        upsert_executed_call(
            &mut executed,
            (fc("tc-2", "write"), func_result("replacement"), false),
        );
        upsert_executed_call(
            &mut executed,
            (fc("tc-3", "list"), func_result("three"), false),
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
