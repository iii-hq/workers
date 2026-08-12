//! AgentMessage[] → OpenAI Chat Completions wire shape. Port of the TS
//! provider's wire-messages.ts with the orphan/dedup boundary sanitization
//! from provider-anthropic (each rule traces to a production incident).
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};

/// Body of the synthetic `role: "tool"` row injected for an orphan tool call
/// (OpenAI rejects assistant `tool_calls` without a tool message per id).
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

/// The wire `tool_calls` entries for one assistant turn, paired with their
/// call ids (for positional result matching).
fn assistant_tool_calls(a: &llm_router::types::messages::AssistantMessage) -> Vec<(String, Value)> {
    a.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => Some((
                id.clone(),
                json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": encode_tool_name(function_id),
                        "arguments": arguments.to_string(),
                    }
                }),
            )),
            // Thinking/RedactedThinking: no reasoning replay on Chat
            // Completions; images never appear in assistant turns here.
            _ => None,
        })
        .collect()
}

/// Build the Chat Completions `messages` array. The hard requirement the
/// strictest upstreams enforce (the gateway relays every vendor's own
/// validation): an assistant message with `tool_calls` MUST be immediately
/// followed by one `role:"tool"` message per `tool_call_id`.
///
/// We cannot rely on a global "is this id answered anywhere" set, because the
/// harness reuses tool_call ids across turns (e.g. `agent_trigger_3` appears in
/// several turns) and emits parallel calls plus interspersed `custom` rows.
/// Instead we pair **positionally**: each assistant turn's calls are answered by
/// the contiguous run of `function_result`s that follows it (custom rows
/// skipped), matched by id and consumed once; any call with no following result
/// gets a synthetic placeholder, and any stray result is dropped.
pub fn to_wire_messages(messages: &[AgentMessage], system_prompt: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if !system_prompt.is_empty() {
        out.push(json!({ "role": "system", "content": system_prompt }));
    }

    let n = messages.len();
    let mut i = 0;
    while i < n {
        match &messages[i] {
            AgentMessage::User(u) => {
                out.push(json!({ "role": "user", "content": user_content_to_wire(&u.content) }));
                i += 1;
            }
            // Custom messages never belong on the model wire (compaction
            // markers, agent events). Drop.
            AgentMessage::Custom(_) => {
                i += 1;
            }
            // A function_result reached in the main loop has no immediately
            // preceding assistant tool_call (its call was compacted away, or it
            // is a stray/out-of-order duplicate already consumed by an earlier
            // turn). Chat Completions rejects a tool message that does not answer
            // a preceding tool_call, so drop it.
            AgentMessage::FunctionResult(_) => {
                i += 1;
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
                let calls = assistant_tool_calls(a);

                // Strict upstreams reject an assistant message with neither content nor
                // tool_calls ("must not be empty") — unlike OpenAI. A
                // thinking-only/empty turn serializes to exactly that; omit it
                // (mirrors provider-anthropic).
                if text.is_empty() && calls.is_empty() {
                    i += 1;
                    continue;
                }

                let mut entry = json!({ "role": "assistant" });
                if !text.is_empty() {
                    entry["content"] = Value::String(text);
                }
                if calls.is_empty() {
                    out.push(entry);
                    i += 1;
                    continue;
                }
                entry["tool_calls"] = Value::Array(calls.iter().map(|(_, v)| v.clone()).collect());
                out.push(entry);

                // The tool-response window: the contiguous run of
                // function_results after this assistant (custom rows skipped),
                // ending at the next assistant/user message.
                let mut window: Vec<(String, String)> = Vec::new();
                let mut j = i + 1;
                while j < n {
                    match &messages[j] {
                        AgentMessage::FunctionResult(r) => {
                            window.push((
                                r.function_call_id.clone(),
                                format_function_result_content(r),
                            ));
                            j += 1;
                        }
                        AgentMessage::Custom(_) => j += 1,
                        _ => break,
                    }
                }
                // Answer each call: latest-wins, consume-once. Missing → orphan
                // placeholder (keeps the assistant tool_call from dangling).
                let mut consumed = vec![false; window.len()];
                for (call_id, _) in &calls {
                    let chosen = (0..window.len())
                        .rev()
                        .find(|&k| !consumed[k] && &window[k].0 == call_id);
                    match chosen {
                        Some(k) => {
                            consumed[k] = true;
                            out.push(tool_row(call_id, window[k].1.clone()));
                        }
                        None => out.push(tool_row(call_id, ORPHAN_TOOL_PLACEHOLDER.to_string())),
                    }
                }
                // Unconsumed window results are stray (no matching call) and are
                // dropped by advancing past them.
                i = j;
            }
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
            provider: "openai".into(),
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

    #[test]
    fn empty_assistant_turn_is_omitted() {
        // thinking-only turn → no content, no tool_calls → strict upstreams would
        // reject it ("must not be empty"), so it is omitted entirely.
        let wire = to_wire_messages(
            &[assistant(vec![ContentBlock::Thinking {
                text: "hmm".into(),
                signature: None,
            }])],
            "",
        );
        assert!(wire.is_empty(), "thinking-only assistant omitted");

        // a genuinely empty assistant turn is omitted too
        assert!(to_wire_messages(&[assistant(vec![])], "").is_empty());

        // surrounding turns still serialize; the empty one is dropped in place
        let wire = to_wire_messages(
            &[
                user(vec![ContentBlock::Text { text: "q".into() }]),
                assistant(vec![ContentBlock::Thinking {
                    text: "hmm".into(),
                    signature: None,
                }]),
                user(vec![ContentBlock::Text {
                    text: "again".into(),
                }]),
            ],
            "",
        );
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["content"], "q");
        assert_eq!(wire[1]["content"], "again");
    }

    #[test]
    fn reused_tool_call_id_pairs_positionally_not_globally() {
        // The real session bug: the same id (`x`) is used in two turns. The
        // first is answered; the second is not. A global "answered anywhere"
        // set would suppress the placeholder for the second and leave it
        // dangling → strict upstreams reject. Positional pairing must give the second
        // occurrence a placeholder.
        let wire = to_wire_messages(
            &[
                assistant(vec![call("x")]),
                result("x", "first-answer", json!({})),
                assistant(vec![call("x")]), // reused id, no following result
                user(vec![ContentBlock::Text {
                    text: "next".into(),
                }]),
            ],
            "",
        );
        // assistant, tool(first), assistant, tool(placeholder), user
        assert_eq!(wire.len(), 5);
        assert_eq!(wire[0]["tool_calls"][0]["id"], "x");
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["content"], "first-answer");
        assert_eq!(wire[2]["tool_calls"][0]["id"], "x");
        assert_eq!(wire[3]["role"], "tool");
        assert_eq!(wire[3]["tool_call_id"], "x");
        assert!(wire[3]["content"].as_str().unwrap().contains("interrupted"));
        assert_eq!(wire[4]["role"], "user");
    }

    #[test]
    fn parallel_tool_calls_each_get_their_result_in_order() {
        let wire = to_wire_messages(
            &[
                assistant(vec![call("a"), call("b"), call("c")]),
                result("b", "rb", json!({})),
                result("a", "ra", json!({})),
                result("c", "rc", json!({})),
            ],
            "",
        );
        // one assistant row with 3 tool_calls, then a tool row per call id (in
        // call order, matched by id regardless of result order)
        assert_eq!(wire.len(), 4);
        assert_eq!(wire[0]["tool_calls"].as_array().unwrap().len(), 3);
        assert_eq!(wire[1]["tool_call_id"], "a");
        assert_eq!(wire[1]["content"], "ra");
        assert_eq!(wire[2]["tool_call_id"], "b");
        assert_eq!(wire[3]["tool_call_id"], "c");
    }

    #[test]
    fn custom_rows_inside_the_tool_window_are_skipped() {
        // compaction/agent-event rows between an assistant call and its result
        // must not break the pairing.
        let custom = AgentMessage::Custom(CustomMessage {
            role: CustomRoleTag::Custom,
            custom_type: "compaction".into(),
            content: vec![],
            display: None,
            details: None,
            timestamp: 1,
        });
        let wire = to_wire_messages(
            &[
                assistant(vec![call("a")]),
                custom,
                result("a", "ra", json!({})),
            ],
            "",
        );
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["tool_calls"][0]["id"], "a");
        assert_eq!(wire[1]["tool_call_id"], "a");
        assert_eq!(wire[1]["content"], "ra");
    }

    #[test]
    fn stray_result_without_a_preceding_call_is_dropped() {
        let wire = to_wire_messages(
            &[
                user(vec![ContentBlock::Text { text: "hi".into() }]),
                result("ghost", "x", json!({})),
            ],
            "",
        );
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
    }
}
