//! AgentMessage[] → Interactions `input` Step array.
//!
//! There is no `role`: the step `type` tells turns apart. Pairing rules are
//! ported from provider-anthropic (orphan placeholder, results without a call
//! dropped, latest-wins dedup).
//!
//! Cache invariant: each step depends only on its own message, so turn N's
//! steps are byte-identical inside turn N+1's request and implicit caching
//! hits. `function_result.name` is read from the paired call, which never
//! changes, so its bytes are stable too.
use crate::PROVIDER_ID;
use crate::wire::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, AssistantMessage, FunctionResultMessage};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Body of the synthetic `function_result` injected for a call with no result.
const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

/// An empty text item is a 400 ("Missing text in content of type text."), so a
/// result with nothing in it carries this instead.
const EMPTY_RESULT_PLACEHOLDER: &str = "(empty result)";

/// A function call in the current turn with no thought step before it is rejected.
/// This signature is the validator bypass Google documents for generateContent;
/// the Interactions API accepts it on a thought step (checked live 2026-11-19).
const PLACEHOLDER_SIGNATURE: &str = "skip_thought_signature_validator";

/// Image types accepted in `user_input`; any other type is a 400 that would
/// fail every later turn of the session.
const INPUT_IMAGE_MIMES: [&str; 8] = [
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/heic",
    "image/heif",
    "image/gif",
    "image/bmp",
    "image/tiff",
];

/// Image types accepted inside a `function_result`.
const RESULT_IMAGE_MIMES: [&str; 6] = [
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
    "image/tiff",
];

fn text_item(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn image_item(mime: &str, data: &str, allowed: &[&str], warnings: &mut Vec<String>) -> Value {
    if allowed.contains(&mime) {
        json!({ "type": "image", "data": data, "mime_type": mime })
    } else {
        warnings.push(format!("image omitted: unsupported type {mime}"));
        text_item(&format!("[image omitted: unsupported type {mime}]"))
    }
}

/// Flat text body for a function result; `details.status == "denied"` gets the
/// `[PERMISSION_DENIED]` marker + single-line JSON envelope so the LLM can
/// parse the structured denial (same body as provider-anthropic).
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

fn function_result_step(
    m: &FunctionResultMessage,
    name: &str,
    warnings: &mut Vec<String>,
) -> Value {
    let mut result = Vec::new();
    let body = format_function_result_content(m);
    if !body.is_empty() {
        result.push(text_item(&body));
    }
    for c in &m.content {
        if let ContentBlock::Image { mime, data, .. } = c {
            result.push(image_item(mime, data, &RESULT_IMAGE_MIMES, warnings));
        }
    }
    if result.is_empty() {
        result.push(text_item(EMPTY_RESULT_PLACEHOLDER));
    }
    json!({
        "type": "function_result",
        "call_id": m.function_call_id,
        "name": name,
        "result": result,
        "is_error": m.is_error,
    })
}

fn assistant_steps(a: &AssistantMessage) -> Vec<Value> {
    let own = a.provider == PROVIDER_ID;
    let mut steps: Vec<Value> = a
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } if !text.is_empty() => Some(json!({
                "type": "model_output",
                "content": [text_item(text)],
            })),
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => Some(json!({
                "type": "function_call",
                "id": id,
                "name": encode_tool_name(function_id),
                "arguments": if arguments.is_object() { arguments.clone() } else { json!({}) },
            })),
            // Signature only: the summary is never sent back. A thought
            // without a signature (aborted stream) or from another provider
            // cannot be verified and is dropped.
            ContentBlock::Thinking {
                signature: Some(sig),
                ..
            } if own => Some(json!({ "type": "thought", "signature": sig })),
            _ => None,
        })
        .collect();
    // A model run (everything since the last user_input or function_result)
    // must open with a thought before any call: [model_output, thought, call]
    // is a 400 even when the model_output is a previous assistant message.
    // Opening every message with one keeps the rule per message, so turn N's
    // steps stay a prefix of turn N+1's.
    if steps.first().is_some_and(|s| s["type"] != "thought") {
        steps.insert(
            0,
            json!({ "type": "thought", "signature": PLACEHOLDER_SIGNATURE }),
        );
    }
    steps
}

/// Map the router history to Steps; unsupported images become text and push
/// a warning.
pub fn to_wire_steps(messages: &[AgentMessage], warnings: &mut Vec<String>) -> Vec<Value> {
    // Results displaced behind an interleaved user message are pulled back
    // next to their call.
    let messages = llm_router::types::messages::reorder_displaced_results(messages);

    // `function_result.name` must equal the call's name. Under the default
    // `agent_call` exposure the call is `agent_call` while the result's
    // `function_id` is the unwrapped target, so the name comes from the call.
    let mut call_names: HashMap<&str, String> = HashMap::new();
    for m in &messages {
        if let AgentMessage::Assistant(a) = m {
            for b in &a.content {
                if let ContentBlock::FunctionCall {
                    id, function_id, ..
                } = b
                {
                    call_names.insert(id, encode_tool_name(function_id));
                }
            }
        }
    }
    let mut resolved_ids: HashSet<&str> = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::FunctionResult(r) => Some(r.function_call_id.as_str()),
            _ => None,
        })
        .collect();

    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        match m {
            AgentMessage::User(u) => {
                let content: Vec<Value> = u
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        // An empty text item is a 400; whitespace is accepted.
                        ContentBlock::Text { text } if !text.is_empty() => Some(text_item(text)),
                        ContentBlock::Image { mime, data, .. } => {
                            Some(image_item(mime, data, &INPUT_IMAGE_MIMES, warnings))
                        }
                        _ => None,
                    })
                    .collect();
                if !content.is_empty() {
                    out.push(json!({ "type": "user_input", "content": content }));
                }
            }
            AgentMessage::Assistant(a) => {
                out.extend(assistant_steps(a));
                for b in &a.content {
                    if let ContentBlock::FunctionCall { id, .. } = b
                        && resolved_ids.insert(id)
                    {
                        out.push(json!({
                            "type": "function_result",
                            "call_id": id,
                            "name": call_names[id.as_str()],
                            "result": [text_item(ORPHAN_TOOL_PLACEHOLDER)],
                            "is_error": true,
                        }));
                    }
                }
            }
            AgentMessage::FunctionResult(r) => {
                let Some(name) = call_names.get(r.function_call_id.as_str()) else {
                    continue;
                };
                let step = function_result_step(r, name, warnings);
                // Latest wins: a second result for one call replaces the first.
                let existing = out.iter().position(|s| {
                    s["type"] == "function_result" && s["call_id"] == step["call_id"]
                });
                match existing {
                    Some(i) => out[i] = step,
                    None => out.push(step),
                }
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
        AssistantRoleTag, FunctionResultRoleTag, UserMessage, UserRoleTag,
    };

    fn user(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content,
            timestamp: 1,
        })
    }

    fn text(t: &str) -> ContentBlock {
        ContentBlock::Text { text: t.into() }
    }

    fn image(mime: &str) -> ContentBlock {
        ContentBlock::Image {
            mime: mime.into(),
            data: "AAAA".into(),
            attachment_id: None,
        }
    }

    fn call(id: &str, function_id: &str, arguments: Value) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: function_id.into(),
            arguments,
        }
    }

    fn thought(sig: Option<&str>) -> ContentBlock {
        ContentBlock::Thinking {
            text: "summary".into(),
            signature: sig.map(str::to_string),
        }
    }

    fn assistant(provider: &str, content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            error_details: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: provider.into(),
            timestamp: 1,
        })
    }

    fn result(call_id: &str, function_id: &str, content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: call_id.into(),
            function_id: function_id.into(),
            content,
            details: json!({}),
            is_error: false,
            timestamp: 1,
        })
    }

    fn steps(messages: &[AgentMessage]) -> Vec<Value> {
        to_wire_steps(messages, &mut Vec::new())
    }

    fn fixture_input(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/requests/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let body: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        body["input"].clone()
    }

    #[test]
    fn user_text_and_images() {
        let mut warnings = Vec::new();
        let out = to_wire_steps(
            &[user(vec![
                text("look"),
                image("image/png"),
                image("image/heic"),
                image("image/x-icon"),
                ContentBlock::File {
                    attachment_id: "a".into(),
                    name: "f".into(),
                    mime: "text/plain".into(),
                    size: 1,
                },
            ])],
            &mut warnings,
        );
        assert_eq!(
            out,
            vec![json!({
                "type": "user_input",
                "content": [
                    { "type": "text", "text": "look" },
                    { "type": "image", "data": "AAAA", "mime_type": "image/png" },
                    { "type": "image", "data": "AAAA", "mime_type": "image/heic" },
                    { "type": "text", "text": "[image omitted: unsupported type image/x-icon]" },
                ],
            })]
        );
        assert_eq!(
            warnings,
            vec!["image omitted: unsupported type image/x-icon"]
        );
    }

    #[test]
    fn empty_text_is_skipped_whitespace_is_kept() {
        let out = steps(&[
            user(vec![text("")]),
            user(vec![text(" ")]),
            assistant("gemini", vec![text(""), text("ok")]),
        ]);
        assert_eq!(
            out,
            vec![
                json!({ "type": "user_input", "content": [{ "type": "text", "text": " " }] }),
                json!({ "type": "thought", "signature": PLACEHOLDER_SIGNATURE }),
                json!({ "type": "model_output", "content": [{ "type": "text", "text": "ok" }] }),
            ]
        );
    }

    #[test]
    fn assistant_texts_are_one_step_each_and_calls_coerce_arguments() {
        let out = steps(&[assistant(
            "gemini",
            vec![
                thought(Some("sig")),
                text("a"),
                text("b"),
                call("c1", "shell::exec", json!("not an object")),
            ],
        )]);
        assert_eq!(out[1]["type"], "model_output");
        assert_eq!(out[2]["type"], "model_output");
        assert_eq!(
            out[3],
            json!({ "type": "function_call", "id": "c1", "name": "shell__exec", "arguments": {} })
        );
    }

    #[test]
    fn result_name_comes_from_the_paired_call() {
        // Under agent_call exposure the result names the unwrapped target.
        let out = steps(&[
            user(vec![text("go")]),
            assistant(
                "gemini",
                vec![thought(Some("s")), call("c1", "agent_call", json!({}))],
            ),
            result("c1", "shell::exec", vec![text("done")]),
        ]);
        assert_eq!(
            out[3],
            json!({
                "type": "function_result",
                "call_id": "c1",
                "name": "agent_call",
                "result": [{ "type": "text", "text": "done" }],
                "is_error": false,
            })
        );
    }

    #[test]
    fn empty_results_and_image_results() {
        let mut warnings = Vec::new();
        let out = to_wire_steps(
            &[
                assistant(
                    "gemini",
                    vec![
                        thought(Some("s")),
                        call("c1", "f", json!({})),
                        call("c2", "f", json!({})),
                        call("c3", "f", json!({})),
                    ],
                ),
                result("c1", "f", vec![text("")]),
                result("c2", "f", vec![image("image/gif")]),
                result("c3", "f", vec![text("x"), image("image/heic")]),
            ],
            &mut warnings,
        );
        assert_eq!(
            out[4]["result"],
            json!([{ "type": "text", "text": "(empty result)" }])
        );
        assert_eq!(
            out[5]["result"],
            json!([{ "type": "image", "data": "AAAA", "mime_type": "image/gif" }])
        );
        assert_eq!(
            out[6]["result"],
            json!([
                { "type": "text", "text": "x" },
                { "type": "text", "text": "[image omitted: unsupported type image/heic]" },
            ])
        );
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn orphan_call_gets_a_placeholder_after_its_message() {
        let out = steps(&[
            assistant(
                "gemini",
                vec![thought(Some("s")), call("c1", "f", json!({}))],
            ),
            user(vec![text("next")]),
        ]);
        assert_eq!(out[2]["type"], "function_result");
        assert_eq!(out[2]["call_id"], "c1");
        assert_eq!(out[2]["name"], "f");
        assert_eq!(out[2]["is_error"], true);
        assert_eq!(out[2]["result"][0]["text"], ORPHAN_TOOL_PLACEHOLDER);
        assert_eq!(out[3]["type"], "user_input");
    }

    #[test]
    fn result_without_a_call_is_dropped() {
        let out = steps(&[
            user(vec![text("hi")]),
            result("ghost", "f", vec![text("x")]),
        ]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn duplicate_result_latest_wins() {
        let out = steps(&[
            assistant(
                "gemini",
                vec![thought(Some("s")), call("c1", "f", json!({}))],
            ),
            result("c1", "f", vec![text("first")]),
            result("c1", "f", vec![text("second")]),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2]["result"][0]["text"], "second");
    }

    #[test]
    fn displaced_result_moves_next_to_its_call() {
        let out = steps(&[
            assistant(
                "gemini",
                vec![thought(Some("s")), call("c1", "f", json!({}))],
            ),
            user(vec![text("steer")]),
            result("c1", "f", vec![text("r")]),
        ]);
        let types: Vec<&str> = out.iter().map(|s| s["type"].as_str().unwrap()).collect();
        assert_eq!(
            types,
            ["thought", "function_call", "function_result", "user_input"]
        );
    }

    #[test]
    fn thought_replay_is_signature_only_and_own_provider_only() {
        let out = steps(&[assistant(
            "gemini",
            vec![
                thought(Some("sig")),
                text("a"),
                thought(None),
                thought(Some("tail")),
            ],
        )]);
        assert_eq!(
            out,
            vec![
                json!({ "type": "thought", "signature": "sig" }),
                json!({ "type": "model_output", "content": [{ "type": "text", "text": "a" }] }),
                json!({ "type": "thought", "signature": "tail" }),
            ]
        );
        let out = steps(&[assistant(
            "anthropic",
            vec![
                thought(Some("sig")),
                ContentBlock::RedactedThinking { data: "r".into() },
                text("a"),
            ],
        )]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["signature"], PLACEHOLDER_SIGNATURE);
        assert_eq!(out[1]["type"], "model_output");
    }

    #[test]
    fn foreign_call_gets_the_placeholder_thought_first() {
        let out = steps(&[
            user(vec![text("What is the weather in Lisbon? Use the tool.")]),
            assistant(
                "anthropic",
                vec![
                    text("I'll check."),
                    call("toolu_01abc", "get_weather", json!({ "city": "Lisbon" })),
                ],
            ),
            result("toolu_01abc", "get_weather", vec![text("18 C, sunny")]),
        ]);
        assert_eq!(
            Value::Array(out),
            fixture_input("foreign_call_placeholder.json")
        );
    }

    #[test]
    fn placeholder_rules() {
        let types = |content: Vec<ContentBlock>| -> Vec<String> {
            steps(&[assistant("gemini", content)])
                .iter()
                .filter(|s| s["type"] != "function_result")
                .map(|s| match s["signature"].as_str() {
                    Some(PLACEHOLDER_SIGNATURE) => "placeholder".to_string(),
                    _ => s["type"].as_str().unwrap().to_string(),
                })
                .collect()
        };
        assert_eq!(
            types(vec![thought(Some("s")), call("c1", "f", json!({}))]),
            ["thought", "function_call"]
        );
        assert_eq!(
            types(vec![call("c1", "f", json!({}))]),
            ["placeholder", "function_call"]
        );
        assert_eq!(
            types(vec![call("c1", "f", json!({})), call("c2", "f", json!({}))]),
            ["placeholder", "function_call", "function_call"]
        );
        assert_eq!(
            types(vec![text("no call")]),
            ["placeholder", "model_output"]
        );
        // The unsigned thought of an aborted stream is dropped.
        assert_eq!(
            types(vec![thought(None), text("cut")]),
            ["placeholder", "model_output"]
        );
    }

    #[test]
    fn text_only_message_before_a_call_message_opens_with_a_thought() {
        let out = steps(&[
            user(vec![text(
                "What is the population of Lisbon? Use the tool.",
            )]),
            assistant("anthropic", vec![text("Let me check.")]),
            assistant("anthropic", vec![call("c1", "lookup", json!({}))]),
            result("c1", "lookup", vec![text("545000")]),
        ]);
        let types: Vec<&str> = out
            .iter()
            .map(|s| match s["signature"].as_str() {
                Some(PLACEHOLDER_SIGNATURE) => "placeholder",
                _ => s["type"].as_str().unwrap(),
            })
            .collect();
        assert_eq!(
            types,
            [
                "user_input",
                "placeholder",
                "model_output",
                "placeholder",
                "function_call",
                "function_result"
            ]
        );
    }

    #[test]
    fn parallel_calls_replay_as_captured() {
        let input = fixture_input("parallel_replay.json");
        let sig = input[1]["signature"].as_str().unwrap();
        let prompt = input[0]["content"][0]["text"].as_str().unwrap();
        let (id1, id2) = (
            input[2]["id"].as_str().unwrap(),
            input[3]["id"].as_str().unwrap(),
        );
        let out = steps(&[
            user(vec![text(prompt)]),
            assistant(
                "gemini",
                vec![
                    thought(Some(sig)),
                    call(id1, "get_weather", json!({ "city": "Lisbon" })),
                    call(id2, "get_weather", json!({ "city": "Porto" })),
                ],
            ),
            result(id1, "get_weather", vec![text("18 C, sunny")]),
            result(id2, "get_weather", vec![text("16 C, rain")]),
        ]);
        assert_eq!(Value::Array(out), input);
    }

    #[test]
    fn turn_n_steps_are_a_prefix_of_turn_n_plus_one() {
        let mut history = vec![
            user(vec![text("go")]),
            assistant("anthropic", vec![call("c1", "f", json!({}))]),
            result("c1", "f", vec![text("r1")]),
        ];
        let before = steps(&history);
        history.push(assistant(
            "gemini",
            vec![thought(Some("s")), call("c2", "f", json!({}))],
        ));
        history.push(result("c2", "f", vec![text("r2")]));
        history.push(user(vec![text("more")]));
        let after = steps(&history);
        assert_eq!(&after[..before.len()], &before[..]);
    }

    #[test]
    fn denied_result_carries_the_envelope() {
        let AgentMessage::FunctionResult(mut r) = result("c1", "f", vec![text("no")]) else {
            unreachable!()
        };
        r.details = json!({ "status": "denied" });
        let out = steps(&[
            assistant(
                "gemini",
                vec![thought(Some("s")), call("c1", "f", json!({}))],
            ),
            AgentMessage::FunctionResult(r),
        ]);
        assert_eq!(
            out[2]["result"][0]["text"],
            "[PERMISSION_DENIED]\n{\"status\":\"denied\"}\n\nno"
        );
    }
}
