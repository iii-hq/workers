use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt;
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::AgentConfig;
use crate::discovery;
use crate::llm::{ContentBlock, LlmClient, LlmRequest, Message, MessageContent, StreamEvent};
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

        Box::pin(async move { handle_chat_stream(iii, config, llm, payload).await })
    }
}

async fn handle_chat_stream(
    iii: III,
    config: Arc<AgentConfig>,
    llm: Arc<LlmClient>,
    payload: Value,
) -> Result<Value, IIIError> {
    // Mint a fresh session_id when the caller omits one. Without this,
    // every session-less request writes events into the shared
    // `agent:events:` group, so concurrent callers interleave each
    // other's streamed output.
    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let user_message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing 'message' field".to_string()))?
        .to_string();

    let stream_group = format!("agent:events:{}", session_id);

    let tools = discovery::discover_tools(&iii).await;
    let system_prompt = discovery::build_system_prompt(&tools);

    let mut messages = load_history(&iii, &session_id).await;

    messages.push(Message {
        role: "user".to_string(),
        content: MessageContent::Text(user_message),
    });

    let mut iterations = 0u32;
    let max_iterations = config.max_iterations;
    let mut full_text = String::new();

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

        let stream_result = llm.send_stream(&request).await;

        let mut event_stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                emit_event(
                    &iii,
                    &stream_group,
                    &json!({
                        "type": "error",
                        "message": format!("LLM stream failed: {}", e)
                    }),
                )
                .await;
                return Err(IIIError::Handler(format!("LLM stream failed: {}", e)));
            }
        };

        let mut current_tool_name = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_input_json = String::new();
        let mut collected_tool_uses: Vec<(String, String, Value)> = Vec::new();
        let mut has_tool_use = false;

        while let Some(event_result) = event_stream.next().await {
            match event_result {
                Ok(event) => {
                    process_stream_event(
                        &iii,
                        &stream_group,
                        &event,
                        &mut full_text,
                        &mut current_tool_name,
                        &mut current_tool_id,
                        &mut current_tool_input_json,
                        &mut collected_tool_uses,
                        &mut has_tool_use,
                    )
                    .await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "stream event error");
                }
            }
        }

        if !current_tool_id.is_empty() {
            let input: Value = serde_json::from_str(&current_tool_input_json).unwrap_or(json!({}));
            collected_tool_uses.push((current_tool_id.clone(), current_tool_name.clone(), input));
        }

        if !has_tool_use {
            messages.push(Message {
                role: "assistant".to_string(),
                content: MessageContent::Text(full_text.clone()),
            });

            save_history(&iii, &session_id, &messages).await;

            emit_event(&iii, &stream_group, &json!({"type": "done"})).await;

            return Ok(json!({
                "stream_group": stream_group,
                "session_id": session_id,
                "iterations": iterations
            }));
        }

        let mut assistant_blocks: Vec<ContentBlock> = Vec::new();
        if !full_text.is_empty() {
            assistant_blocks.push(ContentBlock::Text {
                text: full_text.clone(),
            });
        }
        for (tool_id, tool_name, tool_input) in &collected_tool_uses {
            assistant_blocks.push(ContentBlock::ToolUse {
                id: tool_id.clone(),
                name: tool_name.clone(),
                input: tool_input.clone(),
            });
        }

        messages.push(Message {
            role: "assistant".to_string(),
            content: MessageContent::Blocks(assistant_blocks),
        });

        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

        for (tool_id, tool_name, tool_input) in &collected_tool_uses {
            let function_id = discovery::tool_name_to_function_id(tool_name);

            emit_event(
                &iii,
                &stream_group,
                &json!({
                    "type": "tool_use",
                    "name": function_id,
                    "input": tool_input
                }),
            )
            .await;

            let result = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: tool_input.clone(),
                    action: None,
                    timeout_ms: Some(30000),
                })
                .await;

            let (content, is_error) = match &result {
                Ok(val) => (serde_json::to_string(val).unwrap_or_default(), None),
                Err(e) => (format!("Error: {}", e), Some(true)),
            };

            emit_event(
                &iii,
                &stream_group,
                &json!({
                    "type": "tool_result",
                    "name": function_id,
                    "result": match &result {
                        Ok(v) => v.clone(),
                        Err(e) => json!({"error": e.to_string()})
                    }
                }),
            )
            .await;

            tool_result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: tool_id.clone(),
                content,
                is_error,
            });
        }

        messages.push(Message {
            role: "user".to_string(),
            content: MessageContent::Blocks(tool_result_blocks),
        });

        full_text.clear();
    }

    emit_event(&iii, &stream_group, &json!({"type": "done"})).await;
    save_history(&iii, &session_id, &messages).await;

    Ok(json!({
        "stream_group": stream_group,
        "session_id": session_id,
        "iterations": iterations
    }))
}

#[allow(clippy::too_many_arguments)]
async fn process_stream_event(
    iii: &III,
    stream_group: &str,
    event: &StreamEvent,
    full_text: &mut String,
    current_tool_name: &mut String,
    current_tool_id: &mut String,
    current_tool_input_json: &mut String,
    collected_tool_uses: &mut Vec<(String, String, Value)>,
    has_tool_use: &mut bool,
) {
    match event.event_type.as_str() {
        "content_block_start" => {
            if let Some(ContentBlock::ToolUse { id, name, .. }) = &event.content_block {
                *current_tool_id = id.clone();
                *current_tool_name = name.clone();
                current_tool_input_json.clear();
                *has_tool_use = true;
            }
        }
        "content_block_delta" => {
            if let Some(delta) = &event.delta {
                if let Some(text) = &delta.text {
                    full_text.push_str(text);
                    emit_event(
                        iii,
                        stream_group,
                        &json!({"type": "text_delta", "text": text}),
                    )
                    .await;
                }
                if let Some(partial_json) = &delta.partial_json {
                    current_tool_input_json.push_str(partial_json);
                }
            }
        }
        "content_block_stop" if !current_tool_id.is_empty() => {
            let input: Value = serde_json::from_str(current_tool_input_json).unwrap_or(json!({}));
            collected_tool_uses.push((current_tool_id.clone(), current_tool_name.clone(), input));
            current_tool_id.clear();
            current_tool_name.clear();
            current_tool_input_json.clear();
        }
        _ => {}
    }
}

async fn emit_event(iii: &III, stream_group: &str, event: &Value) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".to_string(),
            payload: json!({
                "scope": stream_group,
                "key": uuid::Uuid::new_v4().to_string(),
                "value": event
            }),
            action: None,
            timeout_ms: Some(5000),
        })
        .await;
}

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
                serde_json::from_value::<Vec<Message>>(inner.clone()).unwrap_or_default()
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

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
