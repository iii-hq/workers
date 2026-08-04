//! AgentMessage[] → DeepSeek Chat Completions wire shape, with the
//! orphan/dedup boundary sanitization the other providers carry (each rule
//! traces to a production incident).
//!
//! Two DeepSeek-specific rules live here:
//!   - **Text only.** The API takes a plain string for message content; no
//!     multimodal content-part array is documented. Image blocks are replaced
//!     by a marker instead of being sent as `image_url` parts, which would
//!     400 the whole turn. `carries_images` lets the caller warn.
//!   - **Reasoning replay.** "The intermediate assistant's `reasoning_content`
//!     must participate in the context concatenation and must be passed back
//!     to the API in all subsequent user interaction turns" — omitting it on
//!     a tool-calling turn is a 400 (guides/thinking_mode). Replay is scoped
//!     to exactly those messages: everywhere else the API documents replayed
//!     reasoning as ignored, so resending it would only re-bill the whole
//!     chain as input on every later turn and pad the cached prefix with
//!     dead tokens.
//!
//! Cache invariant: DeepSeek's automatic prompt cache hits on shared request
//! *prefixes*, so every rule in this module depends only on a message's own
//! content — never on what follows it — keeping a growing transcript
//! append-only on the wire (turn N's rows are byte-identical inside turn
//! N+1's request). The two deliberate exceptions mutate history for
//! correctness and cost one cache bust each: a late tool result replacing
//! its orphan placeholder, and latest-wins dedup of a duplicated result.
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Body of the synthetic `role: "tool"` row injected for an orphan tool call
/// (Chat Completions rejects assistant `tool_calls` without a tool message
/// per id).
const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

/// Stand-in for an image this text-only API cannot receive.
const IMAGE_PLACEHOLDER: &str = "[image omitted: DeepSeek takes text input only]";

/// True when any message carries an image block — the caller turns this into
/// a report-and-continue warning on the final message.
pub fn carries_images(messages: &[AgentMessage]) -> bool {
    messages.iter().any(|m| {
        let content = match m {
            AgentMessage::User(u) => &u.content,
            AgentMessage::FunctionResult(r) => &r.content,
            AgentMessage::Assistant(a) => &a.content,
            AgentMessage::Custom(_) => return false,
        };
        content
            .iter()
            .any(|c| matches!(c, ContentBlock::Image { .. }))
    })
}

/// Flatten content blocks to the plain string DeepSeek accepts: text verbatim,
/// images as a marker, everything else dropped.
fn flatten(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Image { .. } => Some(IMAGE_PLACEHOLDER),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Concatenated `Thinking` text, for the `reasoning_content` replay.
fn thinking_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Flat text body for a tool message; `details.status == "denied"` gets the
/// `[PERMISSION_DENIED]` marker + single-line JSON envelope so the LLM can
/// parse the structured denial (port of harness/src/types/wire.ts; same body
/// as provider-anthropic/src/wire/messages.rs).
fn format_function_result_content(m: &FunctionResultMessage) -> String {
    let body = flatten(&m.content);
    let denied = m.details.get("status").and_then(Value::as_str) == Some("denied");
    if denied {
        let envelope = serde_json::to_string(&m.details).unwrap_or_else(|_| "{}".into());
        format!("[PERMISSION_DENIED]\n{envelope}\n\n{body}")
    } else {
        body
    }
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
    // Results displaced behind an interleaved user message (notification /
    // steering injected mid call-window) must be pulled back next to their
    // call: Chat Completions rejects a user row between tool_calls and its
    // tool rows.
    let messages = llm_router::types::messages::reorder_displaced_results(messages);
    let mut out: Vec<Value> = Vec::new();
    if !system_prompt.is_empty() {
        out.push(json!({ "role": "system", "content": system_prompt }));
    }

    // Pre-pass: every function_call_id that has a matching function_result
    // anywhere in the conversation. tool_calls NOT in this set get a synthetic
    // placeholder so the API never sees an unanswered tool_call_id.
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
                out.push(json!({ "role": "user", "content": flatten(&u.content) }));
            }
            AgentMessage::Assistant(a) => {
                let text = flatten(&a.content);
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
                // {"role":"assistant"} the API rejects. A dead retry's
                // empty_assistant placeholder (the harness appends it before
                // streaming; a transient upstream failure can leave it durably
                // empty), a poisoned entry a prior failed turn left
                // mid-transcript, and a thinking-only turn all reduce to this
                // shape. It carries nothing for the model — omit it, the way
                // provider-anthropic and provider-openai-codex already do.
                if text.is_empty() && tool_calls.is_empty() {
                    continue;
                }
                let mut entry = json!({ "role": "assistant" });
                if !text.is_empty() {
                    entry["content"] = Value::String(text);
                }
                if !tool_calls.is_empty() {
                    // Reasoning replay, scoped to tool-calling messages: the
                    // API 400s a tool round whose intermediate reasoning was
                    // dropped, and documents replayed reasoning as ignored
                    // everywhere else — where it would only re-bill the whole
                    // chain as input on every later turn. The scope is a
                    // property of this message alone, so the serialized
                    // transcript stays prefix-stable for the prompt cache.
                    let reasoning = thinking_text(&a.content);
                    if !reasoning.is_empty() {
                        entry["reasoning_content"] = Value::String(reasoning);
                    }
                    entry["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(entry);
                // Placeholders for orphans go directly after the assistant
                // row — exactly where the API expects the tool messages.
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
            provider: "deepseek".into(),
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
    fn image() -> ContentBlock {
        ContentBlock::Image {
            mime: "image/png".into(),
            data: "QUJD".into(),
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
    fn thinking_replays_as_reasoning_content_on_tool_calling_turns() {
        // Documented 400 otherwise: the intermediate assistant's
        // reasoning_content must be passed back on every later turn.
        let wire = to_wire_messages(
            &[
                assistant(vec![
                    ContentBlock::Thinking {
                        text: "the user wants a listing".into(),
                        signature: None,
                    },
                    call("t1"),
                ]),
                result("t1", "ok", json!({})),
            ],
            "",
        );
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(wire[0]["reasoning_content"], "the user wants a listing");
        assert_eq!(wire[0]["tool_calls"][0]["id"], "t1");
        assert!(
            wire[0].get("content").is_none(),
            "no text block, no content field: {wire:?}"
        );
        // A call-less answer turn does NOT replay reasoning: the API ignores
        // it there, and resending it would re-bill the whole chain as input
        // on every later turn of the session.
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
        assert!(
            wire[0].get("reasoning_content").is_none(),
            "call-less turns drop reasoning on replay: {wire:?}"
        );
    }

    /// Cache pin (module doc § Cache invariant): DeepSeek's automatic prompt
    /// cache hits on shared request prefixes, so a growing transcript must
    /// serialize append-only — every earlier wire row byte-identical from
    /// turn to turn.
    #[test]
    fn growing_transcript_serializes_append_only_for_the_prompt_cache() {
        // The same session at three successive requests: mid tool round,
        // after the final answer, and after the next user message.
        let turn = |n: usize| {
            let mut msgs = vec![
                user(vec![ContentBlock::Text {
                    text: "task".into(),
                }]),
                assistant(vec![
                    ContentBlock::Thinking {
                        text: "plan the listing".into(),
                        signature: None,
                    },
                    call("t1"),
                ]),
                result("t1", "ok", json!({})),
            ];
            if n >= 2 {
                msgs.push(assistant(vec![
                    ContentBlock::Thinking {
                        text: "now conclude".into(),
                        signature: None,
                    },
                    ContentBlock::Text {
                        text: "done".into(),
                    },
                ]));
            }
            if n >= 3 {
                msgs.push(user(vec![ContentBlock::Text {
                    text: "next".into(),
                }]));
            }
            msgs
        };
        let w1 = to_wire_messages(&turn(1), "sys");
        let w2 = to_wire_messages(&turn(2), "sys");
        let w3 = to_wire_messages(&turn(3), "sys");
        assert_eq!(w2.len(), w1.len() + 1);
        assert_eq!(w3.len(), w2.len() + 1);
        assert_eq!(w2[..w1.len()], w1[..], "turn 2 must extend turn 1 verbatim");
        assert_eq!(w3[..w2.len()], w2[..], "turn 3 must extend turn 2 verbatim");
        // and the required replay is present while the ignorable one is not
        assert_eq!(w1[2]["reasoning_content"], "plan the listing");
        assert!(w2[4].get("reasoning_content").is_none());
    }

    #[test]
    fn user_message_between_call_and_result_keeps_tool_row_adjacent() {
        // Live-repro class: a notification/steering user entry injected into a
        // parked call window lands between the call and its result in the
        // transcript. The API rejects a user row between the assistant
        // tool_calls row and its tool rows.
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
        assert_eq!(wire[0]["role"], "assistant");
        assert_eq!(
            wire[1]["role"], "tool",
            "tool row must directly follow tool_calls, got: {wire:?}"
        );
        assert_eq!(wire[1]["tool_call_id"], "t1");
        assert_eq!(wire[2]["role"], "user");
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
    fn images_degrade_to_a_marker_and_content_stays_a_plain_string() {
        // DeepSeek documents no multimodal content-part array; sending
        // image_url parts 400s the whole turn.
        let msgs = [user(vec![
            ContentBlock::Text {
                text: "what is this".into(),
            },
            image(),
        ])];
        assert!(carries_images(&msgs));
        let wire = to_wire_messages(&msgs, "");
        let content = wire[0]["content"].as_str().expect("plain string content");
        assert_eq!(
            content,
            "what is this\n[image omitted: DeepSeek takes text input only]"
        );

        // images inside a tool result degrade the same way
        let msgs = [
            assistant(vec![call("t1")]),
            AgentMessage::FunctionResult(FunctionResultMessage {
                role: FunctionResultRoleTag::FunctionResult,
                function_call_id: "t1".into(),
                function_id: "shell::exec".into(),
                content: vec![
                    ContentBlock::Text {
                        text: "page".into(),
                    },
                    image(),
                ],
                details: json!({}),
                is_error: false,
                timestamp: 3,
            }),
        ];
        assert!(carries_images(&msgs));
        let wire = to_wire_messages(&msgs, "");
        assert_eq!(wire.len(), 2, "no synthetic image row: {wire:?}");
        assert!(wire[1]["content"]
            .as_str()
            .unwrap()
            .contains("[image omitted"));
    }

    #[test]
    fn image_free_transcripts_are_not_flagged() {
        assert!(!carries_images(&[
            user(vec![ContentBlock::Text { text: "hi".into() }]),
            assistant(vec![call("t1")]),
            result("t1", "ok", json!({})),
        ]));
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
    fn empty_assistant_is_omitted_wherever_it_sits() {
        // A content-less, call-less assistant — a dead retry's empty_assistant
        // placeholder, a poisoned entry a prior failed turn persisted, or a
        // thinking-only turn — must never reach the wire as a bare
        // {"role":"assistant"} row. Dropped mid-list (poisoned session heals):
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
        // Dropped when trailing (the dead-placeholder shape) — no
        // assistant-final row survives to trigger the reject.
        let wire = to_wire_messages(
            &[
                user(vec![ContentBlock::Text {
                    text: "task".into(),
                }]),
                assistant(vec![]),
            ],
            "",
        );
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["role"], "user");
        // A thinking-only assistant carries no answer and no call: same shape,
        // dropped rather than shipped as a bare reasoning_content row.
        let wire = to_wire_messages(
            &[assistant(vec![ContentBlock::Thinking {
                text: "hmm".into(),
                signature: None,
            }])],
            "",
        );
        assert!(wire.is_empty(), "thinking-only assistant omitted: {wire:?}");
    }
}
