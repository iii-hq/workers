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

/// `function_call_output.output` is a flat string, so Image blocks inside a
/// FunctionResult are hoisted into one synthetic user message emitted after
/// the contiguous run of tool outputs (never between a function_call and its
/// output, and never splitting sibling tool rows).
fn flush_result_images(out: &mut Vec<Value>, buf: &mut Vec<(String, Vec<Value>)>) {
    if buf.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    for (call_id, images) in buf.drain(..) {
        parts.push(json!({
            "type": "input_text",
            "text": format!("[image result of tool call {call_id}]")
        }));
        parts.extend(images);
    }
    out.push(json!({ "role": "user", "content": parts }));
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
    // Results displaced behind an interleaved user message (notification /
    // steering injected mid call-window) must be pulled back next to their
    // call: the Responses API rejects a user item between a function_call
    // and its function_call_output.
    let messages = llm_router::types::messages::reorder_displaced_results(messages);
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

    // call_id -> hoisted input_image parts, flushed when a result run ends.
    let mut image_buf: Vec<(String, Vec<Value>)> = Vec::new();
    for m in messages {
        match m {
            AgentMessage::User(u) => {
                flush_result_images(&mut out, &mut image_buf);
                out.push(json!({ "role": "user", "content": user_content_parts(&u.content) }));
            }
            AgentMessage::Assistant(a) => {
                flush_result_images(&mut out, &mut image_buf);
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
                let images: Vec<Value> = r
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentBlock::Image { mime, data } => Some(json!({
                            "type": "input_image",
                            "image_url": format!("data:{mime};base64,{data}")
                        })),
                        _ => None,
                    })
                    .collect();
                // Latest-wins per call_id, mirroring upsert_output.
                // ponytail: a duplicate replayed after its images were
                // flushed re-emits them; cross-flush dedup would need a
                // seen-id set.
                let existing = image_buf
                    .iter()
                    .position(|(id, _)| id == &r.function_call_id);
                match existing {
                    Some(i) if images.is_empty() => {
                        image_buf.remove(i);
                    }
                    Some(i) => image_buf[i].1 = images,
                    None if !images.is_empty() => {
                        image_buf.push((r.function_call_id.clone(), images));
                    }
                    None => {}
                }
            }
            AgentMessage::Custom(_) => {}
        }
    }
    flush_result_images(&mut out, &mut image_buf);
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
        result_blocks(id, vec![ContentBlock::Text { text: text.into() }], details)
    }
    fn result_blocks(id: &str, content: Vec<ContentBlock>, details: Value) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "shell::exec".into(),
            content,
            details,
            is_error: false,
            timestamp: 3,
        })
    }
    fn image(data: &str) -> ContentBlock {
        ContentBlock::Image {
            mime: "image/png".into(),
            data: data.into(),
        }
    }
    fn call(id: &str) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: "shell::exec".into(),
            arguments: json!({ "cmd": "ls" }),
        }
    }

    #[test]
    fn user_message_between_call_and_result_keeps_output_adjacent() {
        // Live-repro class: a notification/steering user entry injected into a
        // parked call window lands between the call and its result in the
        // transcript. The Responses API rejects a user item between a
        // function_call and its function_call_output.
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
        assert_eq!(wire[0]["type"], "function_call");
        assert_eq!(
            wire[1]["type"], "function_call_output",
            "output must directly follow its function_call, got: {wire:?}"
        );
        assert_eq!(wire[1]["call_id"], "t1");
        assert_eq!(wire[2]["role"], "user");
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
    fn result_images_hoisted_into_user_message_after_output() {
        // Incident class: base64 screenshots inside a FunctionResult were
        // silently dropped (output is a flat string), so the model never saw
        // them. Images must reach the model as a synthetic user item that
        // never lands between a function_call and its output.
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result_blocks(
                    "t1",
                    vec![ContentBlock::Text { text: "ok".into() }, image("AAAA")],
                    json!({}),
                ),
                user(vec![ContentBlock::Text {
                    text: "next".into(),
                }]),
            ],
            "",
        );
        assert_eq!(wire[0]["type"], "function_call");
        assert_eq!(wire[1]["type"], "function_call_output");
        assert_eq!(wire[1]["output"], "ok", "output stays a flat text string");
        assert_eq!(wire[2]["role"], "user", "hoisted images, got: {wire:?}");
        assert_eq!(
            wire[2]["content"][0]["text"],
            "[image result of tool call t1]"
        );
        assert_eq!(wire[2]["content"][1]["type"], "input_image");
        assert_eq!(
            wire[2]["content"][1]["image_url"],
            "data:image/png;base64,AAAA"
        );
        assert_eq!(wire[3]["content"][0]["text"], "next");
    }

    #[test]
    fn sibling_result_images_flush_as_one_user_message_at_end() {
        // Sibling tool rows must stay contiguous; images from a run of
        // results flush as ONE user item, including at end-of-transcript.
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1"), call("t2")]),
                result_blocks("t1", vec![image("AAAA")], json!({})),
                result_blocks("t2", vec![image("BBBB")], json!({})),
            ],
            "",
        );
        assert_eq!(wire[2]["type"], "function_call_output");
        assert_eq!(wire[3]["type"], "function_call_output");
        assert_eq!(wire[4]["role"], "user");
        let parts = wire[4]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 4, "text+image per call, got: {wire:?}");
        assert_eq!(parts[0]["text"], "[image result of tool call t1]");
        assert_eq!(parts[1]["image_url"], "data:image/png;base64,AAAA");
        assert_eq!(parts[2]["text"], "[image result of tool call t2]");
        assert_eq!(parts[3]["image_url"], "data:image/png;base64,BBBB");
        assert_eq!(wire.len(), 5);
    }

    #[test]
    fn duplicate_result_images_latest_wins_before_flush() {
        // Deferred-resolve replay upserts the tool row latest-wins; buffered
        // images must follow the same rule.
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result_blocks("t1", vec![image("OLD1")], json!({})),
                result_blocks("t1", vec![image("NEW1")], json!({})),
            ],
            "",
        );
        assert_eq!(wire[2]["role"], "user");
        let parts = wire[2]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["image_url"], "data:image/png;base64,NEW1");
        assert_eq!(wire.len(), 3);
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
