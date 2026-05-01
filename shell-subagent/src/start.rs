//! `shell::subagent::start` — spawn a child durable session via `run::start`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

pub const ID: &str = "shell::subagent::start";
pub const DESCRIPTION: &str =
    "Spawn a sub-agent for a focused subtask. Args: prompt, provider, model, system_prompt?, max_turns?, parent_session_id?, max_subagent_depth?.";

pub const DEFAULT_MAX_DEPTH: usize = 3;

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let prompt = required(args, "prompt")?;
    let provider = required(args, "provider")?;
    let model = required(args, "model")?;
    let system_prompt = args
        .get("system_prompt")
        .and_then(Value::as_str)
        .unwrap_or("You are a focused sub-agent. Answer the parent's subtask concisely and stop.")
        .to_string();
    let parent_session = args
        .get("parent_session_id")
        .and_then(Value::as_str)
        .unwrap_or("root")
        .to_string();
    let max_depth = args
        .get("max_subagent_depth")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_MAX_DEPTH, |v| v as usize);
    let current_depth = parent_session.matches("::sub-").count();
    if current_depth >= max_depth {
        let msg = format!(
            "sub-agent depth limit reached ({current_depth}/{max_depth}); refusing spawn from {parent_session}"
        );
        return Ok(json!({
            "content": [{ "type": "text", "text": msg.clone() }],
            "details": {
                "depth_limit_reached": true,
                "depth": current_depth,
                "max_depth": max_depth,
                "parent_session_id": parent_session,
                "error": msg,
            },
            "terminate": false,
        }));
    }
    let child_session_id = format!(
        "{parent_session}::sub-{}",
        chrono::Utc::now().timestamp_millis()
    );
    let payload = json!({
        "session_id": child_session_id,
        "parent_session_id": parent_session,
        "provider": provider,
        "model": model,
        "system_prompt": system_prompt,
        "messages": [{
            "role": "user",
            "content": [{"type": "text", "text": prompt}],
            "timestamp": chrono::Utc::now().timestamp_millis(),
        }],
        "tools": [],
    });

    let response = iii
        .trigger(TriggerRequest {
            function_id: "run::start_and_wait".into(),
            payload,
            action: None,
            timeout_ms: Some(600_000),
        })
        .await;

    Ok(match response {
        Ok(value) => {
            let messages = value
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let final_text = extract_final_assistant(&messages)
                .unwrap_or_else(|| "<sub-agent returned no text>".to_string());
            json!({
                "content": [{ "type": "text", "text": final_text }],
                "details": {
                    "child_session_id": child_session_id,
                    "turn_count": messages.len(),
                    "via": "run::start_and_wait",
                },
                "terminate": false,
            })
        }
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sub-agent failed: {e}") }],
            "details": {
                "child_session_id": child_session_id,
                "via": "run::start_and_wait",
                "error": e.to_string(),
            },
            "terminate": false,
        }),
    })
}

fn required(args: &Value, key: &str) -> Result<String, IIIError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required arg: {key}")))
}

fn extract_final_assistant(messages: &[Value]) -> Option<String> {
    for m in messages.iter().rev() {
        let role = m.get("role").and_then(Value::as_str)?;
        if role != "assistant" {
            continue;
        }
        let content = m.get("content").and_then(Value::as_array)?;
        let text: String = content
            .iter()
            .filter_map(|c| {
                if c.get("type").and_then(Value::as_str) == Some("text") {
                    c.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_final_assistant_picks_last_assistant_text() {
        let msgs = vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "ok" }] }),
            json!({ "role": "tool_result", "content": [{ "type": "text", "text": "x" }] }),
        ];
        assert_eq!(extract_final_assistant(&msgs).as_deref(), Some("ok"));
    }

    #[test]
    fn id_is_namespaced() {
        assert_eq!(ID, "shell::subagent::start");
    }

    #[test]
    fn default_max_depth_matches_legacy() {
        assert_eq!(DEFAULT_MAX_DEPTH, 3);
    }
}
