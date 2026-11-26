//! AgentMessage[] → OpenAI Responses API `input` items. Ports the orphan/dedup
//! tool-call sanitization from the Chat Completions provider (each rule traces
//! to a production incident): the Codex backend 400s on an unanswered or
//! duplicated `call_id` just like Chat Completions does.
use crate::config::upstream_model_id;
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{Value, json};
use std::collections::HashSet;

/// Output body injected for a tool call that never got a result.
pub(crate) const ORPHAN_TOOL_PLACEHOLDER: &str =
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

/// Content parts: `input_text` + `input_image` (data URIs). Always at least
/// one part (empty `input_text`) so the backend never sees empty content.
fn content_parts(text: String, content: &[ContentBlock]) -> Vec<Value> {
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({ "type": "input_text", "text": text }));
    }
    for c in content {
        if let ContentBlock::Image { mime, data, .. } = c {
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

fn function_call_output(call_id: &str, output: Value) -> Value {
    json!({ "type": "function_call_output", "call_id": call_id, "output": output })
}

/// A finished `reasoning` output item as a `Thinking` signature: a JSON
/// envelope naming its model, so replay can tell it from another provider's
/// signature or another model's blob. None without `encrypted_content`.
pub(crate) fn pack_reasoning(item: &Value, model: &str) -> Option<String> {
    let encrypted = item["encrypted_content"]
        .as_str()
        .filter(|blob| !blob.is_empty())?;
    let summary = item
        .get("summary")
        .filter(|summary| summary.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));
    Some(
        json!({
            "type": "reasoning",
            "model": upstream_model_id(model),
            "id": item["id"].as_str(),
            "summary": summary,
            "encrypted_content": encrypted,
        })
        .to_string(),
    )
}

/// The `reasoning` input item for a Thinking block packed for `model`, built
/// from picked fields; the stored `id` is kept but never sent.
fn reasoning_item(block: &ContentBlock, model: &str) -> Option<Value> {
    let ContentBlock::Thinking {
        signature: Some(signature),
        ..
    } = block
    else {
        return None;
    };
    let sealed: Value = serde_json::from_str(signature).ok()?;
    let encrypted = sealed["encrypted_content"]
        .as_str()
        .filter(|blob| !blob.is_empty())?;
    let summary = sealed["summary"].as_array()?;
    (sealed["type"] == "reasoning" && sealed["model"] == upstream_model_id(model)).then(|| {
        json!({
            "type": "reasoning",
            "summary": summary,
            "content": null,
            "encrypted_content": encrypted,
        })
    })
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
/// system item keeps ordering explicit and matches the reference). `model`
/// selects which stored reasoning items are replayed.
pub fn to_wire_messages(messages: &[AgentMessage], system_prompt: &str, model: &str) -> Vec<Value> {
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

    for m in messages {
        match m {
            AgentMessage::User(u) => {
                out.push(json!({
                    "role": "user",
                    "content": content_parts(text_of(&u.content), &u.content),
                }));
            }
            AgentMessage::Assistant(a) => {
                // Leading reasoning goes before the reply's text, later reasoning
                // before its next call; reasoning with nothing after it (a cut
                // reply) is the Responses 400 class, so it is dropped.
                let start = out.len();
                let lead = a
                    .content
                    .iter()
                    .take_while(|c| matches!(c, ContentBlock::Thinking { .. }))
                    .count();
                out.extend(
                    a.content[..lead]
                        .iter()
                        .filter_map(|c| reasoning_item(c, model)),
                );
                let reasoned = out.len();
                // The API asks for each reply's phase back and the message does
                // not store it, so it is derived. The Codex CLI also knows
                // `partial_answer`, which this never sends.
                let has_call = a
                    .content
                    .iter()
                    .any(|c| matches!(c, ContentBlock::FunctionCall { .. }));
                let closing_at = if has_call {
                    a.content.len()
                } else {
                    a.content
                        .iter()
                        .rposition(
                            |c| matches!(c, ContentBlock::Text { text } if !text.trim().is_empty()),
                        )
                        .unwrap_or(0)
                };
                let (before, closing) = a.content.split_at(closing_at);
                for (phase, blocks) in [("commentary", before), ("final_answer", closing)] {
                    let text = text_of(blocks);
                    if !text.is_empty() {
                        out.push(json!({
                            "role": "assistant",
                            "phase": phase,
                            "content": [{ "type": "output_text", "text": text }],
                        }));
                    }
                }
                let mut pending: Vec<Value> = Vec::new();
                for c in &a.content[lead..] {
                    match c {
                        ContentBlock::Thinking { .. } => pending.extend(reasoning_item(c, model)),
                        ContentBlock::Text { .. } => pending.clear(),
                        ContentBlock::FunctionCall {
                            id,
                            function_id,
                            arguments,
                        } => {
                            out.append(&mut pending);
                            out.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": encode_tool_name(function_id),
                                "arguments": arguments.to_string(),
                            }));
                            // Orphan (no result anywhere): synthetic output so
                            // the backend never sees an unanswered call_id.
                            if !resolved_ids.contains(id) {
                                out.push(function_call_output(id, json!(ORPHAN_TOOL_PLACEHOLDER)));
                                resolved_ids.insert(id.clone());
                            }
                        }
                        _ => {}
                    }
                }
                if out.len() == reasoned {
                    out.truncate(start);
                }
            }
            AgentMessage::FunctionResult(r) => {
                let text = format_function_result_content(r);
                // Images ride inside the output as Codex sends them: a user
                // item after each result would end the turn for the backend.
                let output = if r
                    .content
                    .iter()
                    .any(|c| matches!(c, ContentBlock::Image { .. }))
                {
                    Value::Array(content_parts(text, &r.content))
                } else {
                    Value::String(text)
                };
                upsert_output(&mut out, function_call_output(&r.function_call_id, output));
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
            attachment_id: None,
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
            "m",
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
            "m",
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
            "m",
        );
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[0]["content"][0]["type"], "output_text");
        assert_eq!(wire[1]["type"], "function_call");
        assert_eq!(wire[1]["call_id"], "t1");
        assert_eq!(wire[1]["name"], "shell__exec");
        assert_eq!(wire[2]["type"], "function_call_output");
        assert_eq!(wire[2]["output"], "ok");
    }

    fn text(text: &str) -> ContentBlock {
        ContentBlock::Text { text: text.into() }
    }
    fn message_item(phase: &str, text: &str) -> Value {
        json!({
            "role": "assistant",
            "phase": phase,
            "content": [{ "type": "output_text", "text": text }],
        })
    }

    #[test]
    fn each_reply_text_carries_the_phase_its_reply_implies() {
        let commentary = |text: &str| message_item("commentary", text);
        let closing = |text: &str| message_item("final_answer", text);
        let thinking = ContentBlock::Thinking {
            text: "hm".into(),
            signature: None,
        };
        let cases = [
            (vec![text("a"), call("t1")], vec![commentary("a")]),
            (
                vec![text("a"), text("b"), call("t1")],
                vec![commentary("a\nb")],
            ),
            (
                vec![text("a"), call("t1"), text("b")],
                vec![commentary("a\nb")],
            ),
            (vec![call("t1")], vec![]),
            (vec![text("a")], vec![closing("a")]),
            (vec![thinking, text("a")], vec![closing("a")]),
            (
                vec![text("a"), text("b")],
                vec![commentary("a"), closing("b")],
            ),
            (
                vec![text("a"), text("b"), text("c")],
                vec![commentary("a\nb"), closing("c")],
            ),
            // A blank last block is not the closing text.
            (vec![text("a"), text("")], vec![closing("a\n")]),
            (vec![text("a"), text("\n")], vec![closing("a\n\n")]),
            (vec![text("")], vec![]),
            (vec![], vec![]),
        ];
        for (content, want) in cases {
            let mut wire = to_wire_messages(&[assistant(content.clone())], "", "m");
            wire.retain(|item| item["role"] == "assistant");
            assert_eq!(wire, want, "{content:?}");
        }
    }

    #[test]
    fn only_assistant_message_items_carry_a_phase() {
        let wire = to_wire_messages(
            &[
                user(vec![text("go")]),
                assistant(vec![text("looking"), call("t1")]),
                result("t1", "ok", json!({})),
                assistant(vec![text("found it"), text("done")]),
            ],
            "be brief",
            "m",
        );
        assert_eq!(
            wire,
            vec![
                json!({ "role": "system", "content": [{ "type": "input_text", "text": "be brief" }] }),
                json!({ "role": "user", "content": [{ "type": "input_text", "text": "go" }] }),
                message_item("commentary", "looking"),
                json!({
                    "type": "function_call",
                    "call_id": "t1",
                    "name": "shell__exec",
                    "arguments": "{\"cmd\":\"ls\"}",
                }),
                json!({ "type": "function_call_output", "call_id": "t1", "output": "ok" }),
                message_item("commentary", "found it"),
                message_item("final_answer", "done"),
            ]
        );
    }

    #[test]
    fn orphan_call_gets_synthetic_output() {
        let wire = to_wire_messages(&[assistant(vec![call("orphan")])], "", "m");
        assert_eq!(wire[0]["type"], "function_call");
        assert_eq!(wire[1]["type"], "function_call_output");
        assert!(wire[1]["output"].as_str().unwrap().contains("interrupted"));
    }

    #[test]
    fn result_images_ride_inside_the_function_call_output() {
        // Incident class: base64 screenshots inside a FunctionResult were
        // silently dropped, so the model never saw them. They ride inside
        // the output, as Codex sends them, so no user item ends the turn.
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
            "m",
        );
        assert_eq!(wire[0]["type"], "function_call");
        assert_eq!(
            wire[1],
            json!({
                "type": "function_call_output",
                "call_id": "t1",
                "output": [
                    { "type": "input_text", "text": "ok" },
                    { "type": "input_image", "image_url": "data:image/png;base64,AAAA" },
                ],
            })
        );
        assert_eq!(wire[2]["content"][0]["text"], "next");
        assert_eq!(wire.len(), 3, "no hoisted user item: {wire:?}");
    }

    #[test]
    fn sibling_results_each_carry_their_own_image() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1"), call("t2")]),
                result_blocks("t1", vec![image("AAAA")], json!({})),
                result_blocks("t2", vec![image("BBBB")], json!({})),
            ],
            "",
            "m",
        );
        let only_image = |data: &str| json!([{ "type": "input_image", "image_url": format!("data:image/png;base64,{data}") }]);
        assert_eq!(wire[2]["call_id"], "t1");
        assert_eq!(wire[2]["output"], only_image("AAAA"));
        assert_eq!(wire[3]["call_id"], "t2");
        assert_eq!(wire[3]["output"], only_image("BBBB"));
        assert_eq!(wire.len(), 4, "{wire:?}");
    }

    #[test]
    fn duplicate_result_latest_wins_with_its_images() {
        // Deferred-resolve replay upserts the tool row latest-wins, images
        // included.
        let wire = to_wire_messages(
            &[
                assistant(vec![call("t1")]),
                result_blocks("t1", vec![image("OLD1")], json!({})),
                result_blocks("t1", vec![image("NEW1")], json!({})),
            ],
            "",
            "m",
        );
        assert_eq!(
            wire[1]["output"],
            json!([{ "type": "input_image", "image_url": "data:image/png;base64,NEW1" }])
        );
        assert_eq!(wire.len(), 2, "{wire:?}");
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
            "m",
        );
        let outputs: Vec<&Value> = wire
            .iter()
            .filter(|r| r["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0]["output"], "second");
    }

    fn reasoning_done(id: Option<&str>, blob: &str) -> Value {
        let mut item = json!({
            "type": "reasoning",
            "status": "completed",
            "summary": [
                { "type": "summary_text", "text": "**A**" },
                { "type": "summary_text", "text": "**B**" },
            ],
            "encrypted_content": blob,
        });
        if let Some(id) = id {
            item["id"] = json!(id);
        }
        item
    }
    fn thought(item: &Value, model: &str) -> ContentBlock {
        ContentBlock::Thinking {
            text: "**A**\n\n**B**".into(),
            signature: pack_reasoning(item, model),
        }
    }

    #[test]
    fn signed_reasoning_goes_back_before_the_item_it_preceded() {
        let item = reasoning_done(Some("rs_1"), "BLOB1");
        let sealed: Value =
            serde_json::from_str(&pack_reasoning(&item, "codex/m").unwrap()).unwrap();
        assert_eq!(sealed["id"], "rs_1");
        assert_eq!(sealed["model"], "m");
        assert!(sealed.get("status").is_none(), "{sealed}");
        let unnamed = reasoning_done(None, "BLOB1");
        let sealed: Value =
            serde_json::from_str(&pack_reasoning(&unnamed, "codex/m").unwrap()).unwrap();
        assert_eq!(sealed.get("id"), Some(&Value::Null));
        assert!(pack_reasoning(&reasoning_done(None, ""), "codex/m").is_none());
        let mut bare = reasoning_done(None, "BLOB1");
        bare.as_object_mut().unwrap().remove("summary");
        let sealed: Value =
            serde_json::from_str(&pack_reasoning(&bare, "codex/m").unwrap()).unwrap();
        assert_eq!(sealed["summary"], json!([]));

        let reasoning = |blob: &str| {
            json!({
                "type": "reasoning",
                "summary": item["summary"],
                "content": null,
                "encrypted_content": blob,
            })
        };
        let function_call = |id: &str| {
            json!({
                "type": "function_call",
                "call_id": id,
                "name": "shell__exec",
                "arguments": "{\"cmd\":\"ls\"}",
            })
        };
        let output = |id: &str, output: &str| json!({ "type": "function_call_output", "call_id": id, "output": output });
        let th1 = thought(&item, "codex/m");
        let th2 = thought(&reasoning_done(Some("rs_2"), "BLOB2"), "codex/m");
        let cases = [
            (
                vec![assistant(vec![th1.clone(), call("t1")])],
                vec![
                    reasoning("BLOB1"),
                    function_call("t1"),
                    output("t1", ORPHAN_TOOL_PLACEHOLDER),
                ],
            ),
            (
                vec![assistant(vec![thought(&unnamed, "codex/m"), call("t1")])],
                vec![
                    reasoning("BLOB1"),
                    function_call("t1"),
                    output("t1", ORPHAN_TOOL_PLACEHOLDER),
                ],
            ),
            (
                vec![assistant(vec![th1.clone(), text("a")])],
                vec![reasoning("BLOB1"), message_item("final_answer", "a")],
            ),
            (
                vec![assistant(vec![
                    th1.clone(),
                    text("a"),
                    th2.clone(),
                    call("t1"),
                ])],
                vec![
                    reasoning("BLOB1"),
                    message_item("commentary", "a"),
                    reasoning("BLOB2"),
                    function_call("t1"),
                    output("t1", ORPHAN_TOOL_PLACEHOLDER),
                ],
            ),
            (
                vec![
                    assistant(vec![th1.clone(), call("t1"), th2.clone(), call("t2")]),
                    result("t1", "one", json!({})),
                    result("t2", "two", json!({})),
                ],
                vec![
                    reasoning("BLOB1"),
                    function_call("t1"),
                    reasoning("BLOB2"),
                    function_call("t2"),
                    output("t1", "one"),
                    output("t2", "two"),
                ],
            ),
        ];
        for (messages, want) in cases {
            assert_eq!(to_wire_messages(&messages, "", "m"), want, "{messages:?}");
        }
    }

    #[test]
    fn reasoning_that_cannot_be_replayed_is_dropped() {
        let item = reasoning_done(Some("rs_1"), "BLOB1");
        let signed = thought(&item, "codex/m");
        let sealed = |signature: String| ContentBlock::Thinking {
            text: "x".into(),
            signature: Some(signature),
        };
        let rows = [
            vec![
                ContentBlock::Thinking {
                    text: "x".into(),
                    signature: None,
                },
                call("t1"),
            ],
            vec![sealed("EqQBCkgIARABGAIiQLb3c2lnbmF0dXJl".into()), call("t1")],
            vec![
                sealed(json!({ "type": "reasoning", "model": "m", "summary": [] }).to_string()),
                call("t1"),
            ],
            vec![
                sealed(
                    json!({ "type": "thinking", "model": "m", "summary": [], "encrypted_content": "B" })
                        .to_string(),
                ),
                call("t1"),
            ],
            vec![
                sealed(
                    json!({ "type": "reasoning", "model": "m", "summary": [], "encrypted_content": "" })
                        .to_string(),
                ),
                call("t1"),
            ],
            vec![
                sealed(
                    json!({ "type": "reasoning", "model": "m", "encrypted_content": "B" })
                        .to_string(),
                ),
                call("t1"),
            ],
            vec![thought(&item, "codex/other"), call("t1")],
            vec![signed.clone()],
            vec![call("t1"), signed.clone()],
            vec![signed.clone(), text("")],
            vec![text("a"), signed.clone(), text("b")],
        ];
        for content in rows {
            let wire = to_wire_messages(&[assistant(content.clone())], "", "m");
            assert!(
                !wire.iter().any(|i| i["type"] == "reasoning"),
                "{content:?}: {wire:?}"
            );
        }
    }
}
