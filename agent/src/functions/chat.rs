use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

use crate::config::AgentConfig;
use crate::discovery;
use crate::llm::{ContentBlock, LlmClient, LlmRequest, Message, MessageContent};
use crate::state;

pub fn build_handler(
    iii: III,
    config: Arc<AgentConfig>,
    llm: Arc<LlmClient>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let config = config.clone();
        let llm = llm.clone();

        Box::pin(async move { handle_chat(iii, config, llm, payload).await })
    }
}

async fn handle_chat(
    iii: III,
    config: Arc<AgentConfig>,
    llm: Arc<LlmClient>,
    payload: Value,
) -> Result<Value, IIIError> {
    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let user_message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing 'message' field".to_string()))?
        .to_string();

    let tools = discovery::discover_tools(&iii).await;
    let system_prompt = discovery::build_system_prompt(&tools);

    let mut messages = load_history(&iii, &session_id).await;

    messages.push(Message {
        role: "user".to_string(),
        content: MessageContent::Text(user_message),
    });

    let mut iterations = 0u32;
    let max_iterations = config.max_iterations;

    loop {
        if iterations >= max_iterations {
            break;
        }
        iterations += 1;

        let request = LlmRequest {
            model: config.anthropic_model.clone(),
            max_tokens: config.max_tokens,
            system: system_prompt.clone(),
            messages: messages.clone(),
            tools: if tools.is_empty() {
                None
            } else {
                Some(tools.clone())
            },
        };

        let response = llm
            .send(&request)
            .await
            .map_err(|e| IIIError::Handler(format!("LLM request failed: {}", e)))?;

        let tool_uses = LlmClient::extract_tool_uses(&response);

        if tool_uses.is_empty() {
            let text = LlmClient::extract_text(&response);

            messages.push(Message {
                role: "assistant".to_string(),
                content: MessageContent::Text(text.clone()),
            });

            save_history(&iii, &session_id, &messages).await;

            return Ok(build_response(&text, &response));
        }

        let mut assistant_blocks: Vec<ContentBlock> = Vec::new();
        for block in &response.content {
            assistant_blocks.push(block.clone());
        }

        messages.push(Message {
            role: "assistant".to_string(),
            content: MessageContent::Blocks(assistant_blocks),
        });

        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

        for tool_use in &tool_uses {
            let function_id = discovery::tool_name_to_function_id(&tool_use.name);

            let result = execute_tool(&iii, &function_id, &tool_use.input).await;

            let (content, is_error) = match result {
                Ok(val) => (serde_json::to_string(&val).unwrap_or_default(), None),
                Err(e) => (format!("Error: {}", e), Some(true)),
            };

            tool_result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: tool_use.id.clone(),
                content,
                is_error,
            });
        }

        messages.push(Message {
            role: "user".to_string(),
            content: MessageContent::Blocks(tool_result_blocks),
        });
    }

    let text = "Reached maximum iterations without a final response.".to_string();
    save_history(&iii, &session_id, &messages).await;

    Ok(json!({
        "elements": [{"type": "text", "content": text}],
        "session_id": session_id,
        "iterations": iterations
    }))
}

async fn execute_tool(iii: &III, function_id: &str, input: &Value) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload: input.clone(),
        action: None,
        timeout_ms: Some(30000),
    })
    .await
}

fn build_response(text: &str, response: &crate::llm::LlmResponse) -> Value {
    let elements = parse_ui_elements(text);

    let mut result = json!({
        "elements": elements,
    });

    if let Some(usage) = &response.usage {
        result["usage"] = json!({
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens
        });
    }

    result
}

fn parse_ui_elements(text: &str) -> Vec<Value> {
    if let Ok(parsed) = serde_json::from_str::<Value>(text) {
        if parsed.is_array() {
            if let Some(arr) = parsed.as_array() {
                return arr.clone();
            }
        }
        if parsed.get("type").is_some() {
            return vec![parsed];
        }
    }

    vec![json!({"type": "text", "content": text})]
}

// Session records are stored as { "created_at": <rfc3339>, "messages": [...] }.
// Read the messages array out of that envelope rather than treating the
// whole record as Vec<Message> — preserves `created_at` so cleanup's
// age-based expiry and history tools keep working across turns.
async fn load_history(iii: &III, session_id: &str) -> Vec<Message> {
    if session_id.is_empty() {
        return Vec::new();
    }

    match state::state_get(iii, "agent:sessions", session_id).await {
        Ok(val) => {
            let inner = val.get("value").unwrap_or(&val);
            if let Some(arr) = inner.get("messages") {
                serde_json::from_value::<Vec<Message>>(arr.clone()).unwrap_or_default()
            } else if inner.is_array() {
                // Legacy shape: the value was saved as a bare messages array
                // by an earlier version. Load it so old sessions still open.
                serde_json::from_value::<Vec<Message>>(inner.clone()).unwrap_or_default()
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

// Preserve (or mint) created_at so session_cleanup's TTL still fires and
// session_history's response shape stays stable after every turn.
async fn save_history(iii: &III, session_id: &str, messages: &[Message]) {
    if session_id.is_empty() {
        return;
    }

    let created_at = match state::state_get(iii, "agent:sessions", session_id).await {
        Ok(val) => val
            .get("value")
            .and_then(|v| v.get("created_at"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        Err(_) => chrono::Utc::now().to_rfc3339(),
    };

    let value = json!({
        "created_at": created_at,
        "messages": serde_json::to_value(messages).unwrap_or(json!([])),
    });
    let _ = state::state_set(iii, "agent:sessions", session_id, &value).await;
}
