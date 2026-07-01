//! AgentMessage[] → xAI Chat Completions wire shape. Port of the TS
//! provider's wire-messages.ts with the orphan/dedup boundary sanitization
//! from provider-anthropic (each rule traces to a production incident).
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Body of the synthetic `role: "tool"` row injected for an orphan tool call
/// (xAI rejects assistant `tool_calls` without a tool message per id).
const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

/// Flat text body for a tool message; `details.status == "denied"` gets the
/// `[PERMISSION_DENIED]` marker + single-line JSON envelope so the LLM can
/// parse the structured denial (port of harness/src/types/wire.ts; same body
/// as provider-anthropic/src/wire/messages.rs).
fn format_function_result_content(m: &FunctionResultMessage) -> String {
    let body = m
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let denied = m.details.get("status").and_then(Value::as_str) == Some("denied");
    if denied {
        let envelope = serde_json::to_string(&m.details).unwrap_or_else(|_| "{}".into());
        format!("[PERMISSION_DENIED]\n{envelope}\n\n{body}")
    } else {
        body
    }
}

/// User content: flat string when text-only; the content-part array form
/// when images are present (`image_url` data URIs).
fn user_content_to_wire(content: &[ContentBlock]) -> Value {
    let has_images = content
        .iter()
        .any(|c| matches!(c, ContentBlock::Image { .. }));
    let text = content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !has_images {
        return Value::String(text);
    }
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({ "type": "text", "text": text }));
    }
    for c in content {
        if let ContentBlock::Image { mime, data } = c {
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime};base64,{data}") }
            }));
        }
    }
    Value::Array(parts)
}

fn tool_row(tool_call_id: &str, content: String) -> Value {
    json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content })
}

/// Latest-wins dedup: replace an existing `role:"tool"` row with the same id
/// (strict gateways reject duplicates; lenient ones silently overwrite).
fn upsert_tool_row(out: &mut Vec<Value>, row: Value) {
    let id = row
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let existing = out.iter().position(|e| {
        e.get("role").and_then(Value::as_str) == Some("tool")
            && e.get("tool_call_id").and_then(Value::as_str) == Some(id)
    });
    match existing {
        Some(i) => out[i] = row,
        None => out.push(row),
    }
}

pub fn to_wire_messages(messages: &[AgentMessage], system_prompt: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if !system_prompt.is_empty() {
        out.push(json!({ "role": "system", "content": system_prompt }));
    }

    // Pre-pass: every function_call_id that has a matching function_result
    // anywhere in the conversation. tool_calls NOT in this set get a synthetic
    // placeholder so xAI never sees an unanswered tool_call_id.
    let mut resolved_ids: HashSet<String> = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::FunctionResult(r) => Some(r.function_call_id.clone()),
            _ => None,
        })
        .collect();

    for m in messages {
        match m {
            AgentMessage::User(u) => {
                out.push(json!({ "role": "user", "content": user_content_to_wire(&u.content) }));
            }
            AgentMessage::Assistant(a) => {
                let text = a
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let tool_calls: Vec<Value> = a
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::FunctionCall {
                            id,
                            function_id,
                            arguments,
                        } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": encode_tool_name(function_id),
                                "arguments": arguments.to_string(),
                            }
                        })),
                        // Thinking/RedactedThinking: no reasoning replay on
                        // Chat Completions; images never appear in assistant
                        // turns from this provider.
                        _ => None,
                    })
                    .collect();
                let mut entry = json!({ "role": "assistant" });
                if !text.is_empty() {
                    entry["content"] = Value::String(text);
                }
                if !tool_calls.is_empty() {
                    entry["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(entry);
                // Placeholders for orphans go directly after the assistant
                // row — exactly where xAI expects the tool messages.
                for block in &a.content {
                    if let ContentBlock::FunctionCall { id, .. } = block {
                        if !resolved_ids.contains(id) {
                            out.push(tool_row(id, ORPHAN_TOOL_PLACEHOLDER.to_string()));
                            resolved_ids.insert(id.clone());
                        }
                    }
                }
            }
            AgentMessage::FunctionResult(r) => {
                // Images inside results are dropped: Chat Completions tool
                // messages accept text content only.
                upsert_tool_row(
                    &mut out,
                    tool_row(&r.function_call_id, format_function_result_content(r)),
                );
            }
            // Never reach the provider per spec (stripped upstream); defensive.
            AgentMessage::Custom(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, CustomMessage, CustomRoleTag, FunctionResultMessage,
        FunctionResultRoleTag, UserMessage, UserRoleTag,
    };

    fn user(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content,
            timestamp: 1,
        })
    }
    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "xai".into(),
            timestamp: 2,
        })
    }
    fn result(id: &str, text: &str, details: Value) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "shell::exec".into(),
            content: vec![ContentBlock::Text { text: text.into() }],
            details,
            is_error: false,
            timestamp: 3,
        })
    }
    fn call(id: &str) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: "shell::exec".into(),
            arguments: json!({ "cmd": "ls" }),
        }
    }

    #[test]
    fn system_prompt_is_the_first_row_when_present() {
        let wire = to_wire_messages(
            &[user(vec![ContentBlock::Text { text: "hi".into() }])],
            "be brief",
        );
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[0]["content"], "be brief");
        assert_eq!(wire[1]["role"], "user");
        // empty prompt: omitted entirely
        let wire = to_wire_messages(&[user(vec![ContentBlock::Text { text: "hi".into() }])], "");
        assert_eq!(wire.len(), 1);
    }

    #[test]
    fn assistant_function_calls_become_tool_calls_with_encoded_names() {
        let wire = to_wire_messages(
            &[
                assistant(vec![
                    ContentBlock::Text {
                        text: "running".into(),
                    },
                    call("t1"),
                ]),
                result("t1", "ok", json!({})),
            ],
            "",
        );
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[0]["content"], "running");
        assert_eq!(wire[0]["tool_calls"][0]["id"], "t1");
        assert_eq!(wire[0]["tool_calls"][0]["type"], "function");
        assert_eq!(wire[0]["tool_calls"][0]["function"]["name"], "shell__exec");
        assert_eq!(
            wire[0]["tool_calls"][0]["function"]["arguments"],
            r#"{"cmd":"ls"}"#
        );
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["tool_call_id"], "t1");
        assert_eq!(wire[1]["content"], "ok");
        assert!(
            wire[1].get("is_error").is_none(),
            "nonstandard field never shipped"
        );
    }

    #[test]
    fn orphan_tool_call_gets_synthetic_placeholder_directly_after_assistant() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("orphan")]),
                user(vec![ContentBlock::Text { text: "hi".into() }]),
            ],
            "",
        );
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["tool_call_id"], "orphan");
        assert!(wire[1]["content"].as_str().unwrap().contains("interrupted"));
        assert_eq!(wire[2]["role"], "user");
    }

    #[test]
    fn duplicate_tool_results_dedup_latest_wins() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result("t1", "first", json!({})),
                result("t1", "second", json!({})),
            ],
            "",
        );
        let tool_rows: Vec<&Value> = wire.iter().filter(|r| r["role"] == "tool").collect();
        assert_eq!(tool_rows.len(), 1);
        assert_eq!(tool_rows[0]["content"], "second");
    }

    #[test]
    fn denied_result_carries_permission_envelope() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result(
                    "t1",
                    "nope",
                    json!({ "status": "denied", "reason": "operator" }),
                ),
            ],
            "",
        );
        let body = wire[1]["content"].as_str().unwrap();
        assert!(body.starts_with("[PERMISSION_DENIED]\n"));
        assert!(body.contains("\"status\":\"denied\""));
        assert!(body.ends_with("\n\nnope"));
    }

    #[test]
    fn user_images_use_the_content_part_array_with_data_uri() {
        let wire = to_wire_messages(
            &[user(vec![
                ContentBlock::Text {
                    text: "what is this".into(),
                },
                ContentBlock::Image {
                    mime: "image/png".into(),
                    data: "QUJD".into(),
                },
            ])],
            "",
        );
        let parts = wire[0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,QUJD");
        // text-only stays a flat string
        let wire = to_wire_messages(&[user(vec![ContentBlock::Text { text: "hi".into() }])], "");
        assert!(wire[0]["content"].is_string());
    }

    #[test]
    fn thinking_blocks_and_result_images_are_dropped() {
        let wire = to_wire_messages(
            &[assistant(vec![
                ContentBlock::Thinking {
                    text: "hmm".into(),
                    signature: Some("sig".into()),
                },
                ContentBlock::Text {
                    text: "answer".into(),
                },
            ])],
            "",
        );
        assert_eq!(wire[0]["content"], "answer");
        assert!(wire[0].get("tool_calls").is_none());

        let msg = AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: "t1".into(),
            function_id: "web::fetch".into(),
            content: vec![
                ContentBlock::Text {
                    text: "page".into(),
                },
                ContentBlock::Image {
                    mime: "image/png".into(),
                    data: "QUJD".into(),
                },
            ],
            details: json!({}),
            is_error: false,
            timestamp: 3,
        });
        let wire = to_wire_messages(&[assistant(vec![call("t1")]), msg], "");
        assert_eq!(wire[1]["content"], "page", "tool content is text-only");
    }

    #[test]
    fn custom_messages_are_skipped() {
        let wire = to_wire_messages(
            &[
                AgentMessage::Custom(CustomMessage {
                    role: CustomRoleTag::Custom,
                    custom_type: "note".into(),
                    content: vec![],
                    display: None,
                    details: None,
                    timestamp: 1,
                }),
                user(vec![ContentBlock::Text { text: "hi".into() }]),
            ],
            "",
        );
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
    }
}
