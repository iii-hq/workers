//! AgentMessage[] → OpenCode Go Chat Completions wire shape. Port of the TS
//! provider's wire-messages.ts with the orphan/dedup boundary sanitization
//! from provider-anthropic (each rule traces to a production incident).
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Body of the synthetic `role: "tool"` row injected for an orphan tool call
/// (OpenCode Go rejects assistant `tool_calls` without a tool message per id).
const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

/// Flat text body for a tool message; `details.status == "denied"` gets the
/// `[PERMISSION_DENIED]` marker + single-line JSON envelope.
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

/// Hoisted result images: tool rows are text-only on Chat Completions, so
/// images inside FunctionResults are buffered per call and emitted as ONE
/// synthetic user message when the contiguous run of tool rows ends.
fn flush_result_images(out: &mut Vec<Value>, buf: &mut Vec<(String, Vec<(String, String)>)>) {
    if buf.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    for (id, images) in buf.drain(..) {
        parts.push(json!({ "type": "text", "text": format!("[image result of tool call {id}]") }));
        for (mime, data) in images {
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime};base64,{data}") }
            }));
        }
    }
    out.push(json!({ "role": "user", "content": parts }));
}

/// Latest-wins dedup: replace an existing `role:"tool"` row with the same id.
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
    // Results displaced behind an interleaved user message must be pulled back.
    let messages = llm_router::types::messages::reorder_displaced_results(messages);
    let mut out: Vec<Value> = Vec::new();
    if !system_prompt.is_empty() {
        out.push(json!({ "role": "system", "content": system_prompt }));
    }

    let mut resolved_ids: HashSet<String> = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::FunctionResult(r) => Some(r.function_call_id.clone()),
            _ => None,
        })
        .collect();

    let mut pending_images: Vec<(String, Vec<(String, String)>)> = Vec::new();

    for m in messages {
        match m {
            AgentMessage::User(u) => {
                flush_result_images(&mut out, &mut pending_images);
                out.push(json!({ "role": "user", "content": user_content_to_wire(&u.content) }));
            }
            AgentMessage::Assistant(a) => {
                flush_result_images(&mut out, &mut pending_images);
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
                        _ => None,
                    })
                    .collect();
                // A content-less, call-less assistant serializes to a bare
                // {"role":"assistant"} that strict gateways reject — omit it.
                if text.is_empty() && tool_calls.is_empty() {
                    continue;
                }
                let mut entry = json!({ "role": "assistant" });
                if !text.is_empty() {
                    entry["content"] = Value::String(text);
                }
                if !tool_calls.is_empty() {
                    entry["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(entry);
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
                let images: Vec<(String, String)> = r
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Image { mime, data } => Some((mime.clone(), data.clone())),
                        _ => None,
                    })
                    .collect();
                let existing = pending_images
                    .iter()
                    .position(|(id, _)| id == &r.function_call_id);
                match existing {
                    Some(i) if images.is_empty() => {
                        pending_images.remove(i);
                    }
                    Some(i) => pending_images[i].1 = images,
                    None if !images.is_empty() => {
                        pending_images.push((r.function_call_id.clone(), images));
                    }
                    None => {}
                }
                upsert_tool_row(
                    &mut out,
                    tool_row(&r.function_call_id, format_function_result_content(r)),
                );
            }
            AgentMessage::Custom(_) => {}
        }
    }
    flush_result_images(&mut out, &mut pending_images);
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
            provider: "opencode_go".into(),
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
    fn image_result(id: &str, text: &str, data: &str) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "shell::exec".into(),
            content: vec![
                ContentBlock::Text { text: text.into() },
                ContentBlock::Image {
                    mime: "image/png".into(),
                    data: data.into(),
                },
            ],
            details: json!({}),
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
    fn user_message_between_call_and_result_keeps_tool_row_adjacent() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                user(vec![ContentBlock::Text {
                    text: "[notification] progress".into(),
                }]),
                result("t1", "ok", json!({})),
            ],
            "",
        );
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[1]["tool_call_id"], "t1");
        assert_eq!(wire[2]["role"], "user");
    }

    #[test]
    fn system_prompt_is_first_row() {
        let wire = to_wire_messages(
            &[user(vec![ContentBlock::Text { text: "hi".into() }])],
            "be helpful",
        );
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[0]["content"], "be helpful");
        assert_eq!(wire[1]["role"], "user");
    }

    #[test]
    fn orphan_tool_call_gets_placeholder() {
        let wire = to_wire_messages(&[assistant(vec![call("t1")])], "");
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["tool_call_id"], "t1");
        assert!(wire[1]["content"].as_str().unwrap().contains("interrupted"));
    }

    #[test]
    fn tool_call_and_result_produces_two_rows() {
        let wire = to_wire_messages(
            &[assistant(vec![call("t1")]), result("t1", "ok", json!({}))],
            "",
        );
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[0]["tool_calls"][0]["id"], "t1");
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["content"], "ok");
    }

    #[test]
    fn denied_result_gets_permission_envelope() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result("t1", "denied", json!({"status": "denied", "reason": "no"})),
            ],
            "",
        );
        let content = wire[1]["content"].as_str().unwrap();
        assert!(content.starts_with("[PERMISSION_DENIED]"));
        assert!(content.contains("status"));
    }

    #[test]
    fn image_results_produce_synthetic_user_message() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                image_result("t1", "shot", "QUJD"),
            ],
            "",
        );
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[2]["role"], "user");
        let parts = wire[2]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["type"], "image_url");
    }

    #[test]
    fn empty_assistant_is_omitted() {
        let wire = to_wire_messages(
            &[
                user(vec![ContentBlock::Text {
                    text: "task".into(),
                }]),
                assistant(vec![ContentBlock::Text {
                    text: "reply".into(),
                }]),
                assistant(vec![]),
                user(vec![ContentBlock::Text {
                    text: "next".into(),
                }]),
            ],
            "",
        );
        let roles: Vec<&str> = wire.iter().map(|r| r["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user"], "got: {wire:?}");
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

    #[test]
    fn duplicate_tool_rows_are_deduplicated() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result("t1", "first", json!({})),
                result("t1", "second", json!({})),
            ],
            "",
        );
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[1]["content"], "second");
    }
}
