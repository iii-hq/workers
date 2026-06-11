//! AgentMessage[] → Anthropic Messages API wire shape. Port of the TS
//! provider's wire-messages.ts; the boundary-sanitization comments there
//! document the production incidents each rule prevents.
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Body of the synthetic `tool_result` injected for an orphan `tool_use`
/// (Anthropic rejects orphans: "tool_use IDs were found without tool_result
/// blocks immediately after").
const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

pub fn content_block_to_wire(b: &ContentBlock) -> Option<Value> {
    match b {
        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentBlock::Image { mime, data } => Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": mime, "data": data }
        })),
        ContentBlock::FunctionCall {
            id,
            function_id,
            arguments,
        } => Some(json!({
            "type": "tool_use",
            "id": id,
            "name": encode_tool_name(function_id),
            "input": arguments,
        })),
        // During tool use Anthropic requires signed thinking blocks passed
        // back unmodified (400 otherwise); unsigned blocks (aborted/partial
        // stream) would fail signature verification and are dropped.
        ContentBlock::Thinking { text, signature } => signature
            .as_ref()
            .map(|sig| json!({ "type": "thinking", "thinking": text, "signature": sig })),
        // Only valid inside a FunctionResultMessage, handled there.
        ContentBlock::FunctionResult { .. } => None,
    }
}

/// Flat text body for a tool_result; `details.status == "denied"` gets the
/// `[PERMISSION_DENIED]` marker + single-line JSON envelope so the LLM can
/// parse the structured denial (port of harness/src/types/wire.ts).
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

/// Anthropic tool_result content accepts a flat string or an array of
/// text/image blocks. Keep the flat string when there are no images (the
/// long-standing wire shape prompt caching has seen); switch to the array
/// form only when an image must reach the model.
fn function_result_to_wire(m: &FunctionResultMessage) -> Value {
    let has_images = m
        .content
        .iter()
        .any(|c| matches!(c, ContentBlock::Image { .. }));
    let content: Value = if has_images {
        let mut blocks = Vec::new();
        let body = format_function_result_content(m);
        if !body.is_empty() {
            blocks.push(json!({ "type": "text", "text": body }));
        }
        for c in &m.content {
            if let ContentBlock::Image { mime, data } = c {
                blocks.push(json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": mime, "data": data }
                }));
            }
        }
        Value::Array(blocks)
    } else {
        Value::String(format_function_result_content(m))
    };
    json!({
        "type": "tool_result",
        "tool_use_id": m.function_call_id,
        "content": content,
        "is_error": m.is_error,
    })
}

pub fn to_wire_messages(messages: &[AgentMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut pending: Vec<Value> = Vec::new();

    // Pre-pass: every function_call_id that has a matching function_result
    // anywhere in the conversation. tool_uses NOT in this set get a synthetic
    // placeholder so Anthropic never sees an orphan.
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
                // Merge pending tool_results INTO this user message: Anthropic
                // allows tool_result + regular content together and forbids
                // consecutive user messages.
                let user_content = u.content.iter().filter_map(content_block_to_wire);
                let content: Vec<Value> = pending.drain(..).chain(user_content).collect();
                out.push(json!({ "role": "user", "content": content }));
            }
            AgentMessage::Assistant(a) => {
                if !pending.is_empty() {
                    out.push(json!({ "role": "user", "content": std::mem::take(&mut pending) }));
                }
                let content: Vec<Value> =
                    a.content.iter().filter_map(content_block_to_wire).collect();
                out.push(json!({ "role": "assistant", "content": content }));
                // Placeholders for orphans land in `pending` → flushed into
                // the NEXT user message, exactly where Anthropic expects them.
                for block in &a.content {
                    if let ContentBlock::FunctionCall { id, .. } = block {
                        if !resolved_ids.contains(id) {
                            pending.push(json!({
                                "type": "tool_result",
                                "tool_use_id": id,
                                "content": ORPHAN_TOOL_PLACEHOLDER,
                                "is_error": true,
                            }));
                            resolved_ids.insert(id.clone());
                        }
                    }
                }
            }
            AgentMessage::FunctionResult(r) => {
                // Latest-wins dedup: Anthropic rejects multiple tool_result
                // blocks with one id and the whole turn fails.
                let block = function_result_to_wire(r);
                let existing = pending.iter().position(|b| {
                    b.get("tool_use_id").and_then(Value::as_str)
                        == Some(r.function_call_id.as_str())
                });
                match existing {
                    Some(i) => pending[i] = block,
                    None => pending.push(block),
                }
            }
            // Never reach the provider per spec (stripped upstream); defensive.
            AgentMessage::Custom(_) => {}
        }
    }
    if !pending.is_empty() {
        out.push(json!({ "role": "user", "content": pending }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AgentMessage, AssistantMessage, AssistantRoleTag, CustomMessage, CustomRoleTag,
        FunctionResultMessage, FunctionResultRoleTag, UserMessage, UserRoleTag,
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
            provider: "anthropic".into(),
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
    fn function_result_merges_into_next_user_message() {
        let wire = to_wire_messages(&[
            assistant(vec![call("t1")]),
            result("t1", "ok", json!({})),
            user(vec![ContentBlock::Text {
                text: "next".into(),
            }]),
        ]);
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[1]["role"], "user");
        let content = wire[1]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "t1");
        assert_eq!(content[0]["content"], "ok");
        assert_eq!(content[1]["type"], "text");
    }

    #[test]
    fn trailing_function_result_flushes_as_final_user_message() {
        let wire = to_wire_messages(&[assistant(vec![call("t1")]), result("t1", "ok", json!({}))]);
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[1]["role"], "user");
        assert_eq!(wire[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn orphan_tool_use_gets_synthetic_placeholder() {
        let wire = to_wire_messages(&[
            assistant(vec![call("orphan")]),
            user(vec![ContentBlock::Text { text: "hi".into() }]),
        ]);
        let content = wire[1]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "orphan");
        assert_eq!(content[0]["is_error"], true);
        assert!(content[0]["content"]
            .as_str()
            .unwrap()
            .contains("interrupted"));
    }

    #[test]
    fn duplicate_tool_results_dedup_latest_wins() {
        let wire = to_wire_messages(&[
            assistant(vec![call("t1")]),
            result("t1", "first", json!({})),
            result("t1", "second", json!({})),
        ]);
        let content = wire[1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["content"], "second");
    }

    #[test]
    fn unsigned_thinking_dropped_signed_replayed() {
        let wire = to_wire_messages(&[assistant(vec![
            ContentBlock::Thinking {
                text: "unsigned".into(),
                signature: None,
            },
            ContentBlock::Thinking {
                text: "signed".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::Text {
                text: "answer".into(),
            },
        ])]);
        let content = wire[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["signature"], "sig");
        assert_eq!(content[1]["type"], "text");
    }

    #[test]
    fn denied_result_carries_permission_envelope() {
        let wire = to_wire_messages(&[
            assistant(vec![call("t1")]),
            result(
                "t1",
                "nope",
                json!({ "status": "denied", "reason": "operator" }),
            ),
        ]);
        let body = wire[1]["content"][0]["content"].as_str().unwrap();
        assert!(body.starts_with("[PERMISSION_DENIED]\n"));
        assert!(body.contains("\"status\":\"denied\""));
        assert!(body.ends_with("\n\nnope"));
    }

    #[test]
    fn image_in_result_switches_to_block_array() {
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
        let wire = to_wire_messages(&[assistant(vec![call("t1")]), msg]);
        let content = &wire[1]["content"][0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "QUJD");
    }

    #[test]
    fn user_images_map_to_base64_source() {
        let wire = to_wire_messages(&[user(vec![ContentBlock::Image {
            mime: "image/jpeg".into(),
            data: "Zm9v".into(),
        }])]);
        assert_eq!(wire[0]["content"][0]["type"], "image");
        assert_eq!(wire[0]["content"][0]["source"]["type"], "base64");
    }

    #[test]
    fn custom_messages_are_skipped() {
        let wire = to_wire_messages(&[
            AgentMessage::Custom(CustomMessage {
                role: CustomRoleTag::Custom,
                custom_type: "note".into(),
                content: vec![],
                display: None,
                details: None,
                timestamp: 1,
            }),
            user(vec![ContentBlock::Text { text: "hi".into() }]),
        ]);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
    }
}
