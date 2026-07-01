//! AgentMessage[] → OpenAI Responses API `input` items. Ports the orphan/dedup
//! tool-call sanitization from the Chat Completions provider (each rule traces
//! to a production incident): the Codex backend 400s on an unanswered or
//! duplicated `call_id` just like Chat Completions does.
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Output body injected for a tool call that never got a result.
const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

fn text_of(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_function_result_content(m: &FunctionResultMessage) -> String {
    let body = text_of(&m.content);
    let denied = m.details.get("status").and_then(Value::as_str) == Some("denied");
    if denied {
        let envelope = serde_json::to_string(&m.details).unwrap_or_else(|_| "{}".into());
        format!("[PERMISSION_DENIED]\n{envelope}\n\n{body}")
    } else {
        body
    }
}

/// User content parts: `input_text` + `input_image` (data URIs). Always at
/// least one part (empty `input_text`) so the backend never sees empty content.
fn user_content_parts(content: &[ContentBlock]) -> Vec<Value> {
    let mut parts = Vec::new();
    let text = text_of(content);
    if !text.is_empty() {
        parts.push(json!({ "type": "input_text", "text": text }));
    }
    for c in content {
        if let ContentBlock::Image { mime, data } = c {
            parts.push(json!({
                "type": "input_image",
                "image_url": format!("data:{mime};base64,{data}")
            }));
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "type": "input_text", "text": "" }));
    }
    parts
}

fn function_call_output(call_id: &str, output: String) -> Value {
    json!({ "type": "function_call_output", "call_id": call_id, "output": output })
}

/// Latest-wins dedup for a `function_call_output` with the same `call_id`.
fn upsert_output(out: &mut Vec<Value>, row: Value) {
    let id = row.get("call_id").and_then(Value::as_str).unwrap_or("");
    let existing = out.iter().position(|e| {
        e.get("type").and_then(Value::as_str) == Some("function_call_output")
            && e.get("call_id").and_then(Value::as_str) == Some(id)
    });
    match existing {
        Some(i) => out[i] = row,
        None => out.push(row),
    }
}

/// Build the Responses `input` array. `system_prompt`, when non-empty, is the
/// first `system` input item (Codex also accepts top-level `instructions`; a
/// system item keeps ordering explicit and matches the reference).
pub fn to_wire_messages(messages: &[AgentMessage], system_prompt: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if !system_prompt.is_empty() {
        out.push(json!({
            "role": "system",
            "content": [{ "type": "input_text", "text": system_prompt }],
        }));
    }

    // Pre-pass: every function_call_id that has a matching result anywhere.
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
                out.push(json!({ "role": "user", "content": user_content_parts(&u.content) }));
            }
            AgentMessage::Assistant(a) => {
                let text = text_of(&a.content);
                if !text.is_empty() {
                    out.push(json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }],
                    }));
                }
                for c in &a.content {
                    if let ContentBlock::FunctionCall {
                        id,
                        function_id,
                        arguments,
                    } = c
                    {
                        out.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": encode_tool_name(function_id),
                            "arguments": arguments.to_string(),
                        }));
                        // Orphan (no result anywhere): synthetic output so the
                        // backend never sees an unanswered call_id.
                        if !resolved_ids.contains(id) {
                            out.push(function_call_output(
                                id,
                                ORPHAN_TOOL_PLACEHOLDER.to_string(),
                            ));
                            resolved_ids.insert(id.clone());
                        }
                    }
                }
            }
            AgentMessage::FunctionResult(r) => {
                upsert_output(
                    &mut out,
                    function_call_output(&r.function_call_id, format_function_result_content(r)),
                );
            }
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
        AssistantMessage, AssistantRoleTag, FunctionResultRoleTag, UserMessage, UserRoleTag,
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
            provider: "openai-codex".into(),
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
    fn system_then_user_input_items() {
        let wire = to_wire_messages(
            &[user(vec![ContentBlock::Text { text: "hi".into() }])],
            "be brief",
        );
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[0]["content"][0]["type"], "input_text");
        assert_eq!(wire[1]["role"], "user");
        assert_eq!(wire[1]["content"][0]["text"], "hi");
    }

    #[test]
    fn assistant_call_and_result_become_functioncall_items() {
        let wire = to_wire_messages(
            &[
                assistant(vec![ContentBlock::Text { text: "run".into() }, call("t1")]),
                result("t1", "ok", json!({})),
            ],
            "",
        );
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[0]["content"][0]["type"], "output_text");
        assert_eq!(wire[1]["type"], "function_call");
        assert_eq!(wire[1]["call_id"], "t1");
        assert_eq!(wire[1]["name"], "shell__exec");
        assert_eq!(wire[2]["type"], "function_call_output");
        assert_eq!(wire[2]["output"], "ok");
    }

    #[test]
    fn orphan_call_gets_synthetic_output() {
        let wire = to_wire_messages(&[assistant(vec![call("orphan")])], "");
        assert_eq!(wire[0]["type"], "function_call");
        assert_eq!(wire[1]["type"], "function_call_output");
        assert!(wire[1]["output"].as_str().unwrap().contains("interrupted"));
    }

    #[test]
    fn duplicate_results_dedup_latest_wins() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result("t1", "first", json!({})),
                result("t1", "second", json!({})),
            ],
            "",
        );
        let outputs: Vec<&Value> = wire
            .iter()
            .filter(|r| r["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0]["output"], "second");
    }
}
