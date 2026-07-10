//! Anthropic SSE → AssistantMessageEvent state machine. Pure: consumes one
//! `data: {…}` SSE block at a time, threads it through PartialState, returns
//! 0+ events. Block arrival order is preserved (replayed turns must keep
//! thinking blocks in their original position relative to tool_use).
use crate::errors::classify;
use crate::wire::names::decode_tool_name;
use crate::{now_ms, PROVIDER_ID};
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    ToolUse,
    Thinking,
    RedactedThinking,
}

#[derive(Debug, Clone, Copy)]
enum BlockSlot {
    Text(usize),
    ToolUse(usize),
    Thinking(usize),
    RedactedThinking,
    Unknown,
}

#[derive(Debug, Default)]
struct PartialFunctionCall {
    id: String,
    function_id: String,
    args_json: String,
}

#[derive(Debug, Default)]
struct PartialThinking {
    text: String,
    signature: Option<String>,
}

pub struct PartialState {
    text_blocks: Vec<String>,
    thinking_blocks: Vec<PartialThinking>,
    redacted_blocks: Vec<String>,
    function_calls: Vec<PartialFunctionCall>,
    /// Wire arrival order of content blocks (kind, index-within-kind-array).
    block_order: Vec<(BlockKind, usize)>,
    /// Anthropic wire `index` → active block slot.
    block_slots: Vec<Option<BlockSlot>>,
    usage: Usage,
    stop_reason: StopReason,
    native_stop_reason: Option<String>,
    error_message: Option<String>,
    warnings: Vec<String>,
    pub saw_message_stop: bool,
}

impl PartialState {
    pub fn new(warnings: Vec<String>) -> Self {
        PartialState {
            text_blocks: Vec::new(),
            thinking_blocks: Vec::new(),
            redacted_blocks: Vec::new(),
            function_calls: Vec::new(),
            block_order: Vec::new(),
            block_slots: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            warnings,
            saw_message_stop: false,
        }
    }
}

fn push_block_content(
    out: &mut Vec<ContentBlock>,
    state: &PartialState,
    kind: BlockKind,
    idx: usize,
) {
    match kind {
        BlockKind::Thinking => {
            if let Some(th) = state.thinking_blocks.get(idx) {
                if !th.text.is_empty() || th.signature.is_some() {
                    out.push(ContentBlock::Thinking {
                        text: th.text.clone(),
                        signature: th.signature.clone(),
                    });
                }
            }
        }
        BlockKind::RedactedThinking => {
            if let Some(data) = state.redacted_blocks.get(idx) {
                if !data.is_empty() {
                    out.push(ContentBlock::RedactedThinking { data: data.clone() });
                }
            }
        }
        BlockKind::Text => {
            if let Some(t) = state.text_blocks.get(idx) {
                if !t.is_empty() {
                    out.push(ContentBlock::Text { text: t.clone() });
                }
            }
        }
        BlockKind::ToolUse => {
            if let Some(tc) = state.function_calls.get(idx) {
                // An interrupted/in-flight stream leaves `args_json` as a
                // partial, unparseable blob (e.g. `{"cmd":`). A tool_use
                // input must be a JSON object (null/string is a hard 400 on
                // replay), so degrade to the salvaged leading fields — long-
                // streaming calls keep their known prefix (`function` target)
                // instead of an anonymous `{}` — else `{"_raw": …}`.
                let arguments = if tc.args_json.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&tc.args_json)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| {
                            llm_router::types::messages::degraded_arguments(&tc.args_json)
                        })
                };
                out.push(ContentBlock::FunctionCall {
                    id: tc.id.clone(),
                    function_id: tc.function_id.clone(),
                    arguments,
                });
            }
        }
    }
}

fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    // block_order is pushed in lockstep with every kind-array push, so it is
    // the complete, duplicate-free arrival order of all blocks.
    let mut out = Vec::new();
    for &(kind, idx) in &state.block_order {
        push_block_content(&mut out, state, kind, idx);
    }
    out
}

pub fn build_partial(state: &PartialState, model: &str) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content: build_content(state),
        stop_reason: state.stop_reason,
        native_stop_reason: state.native_stop_reason.clone(),
        error_message: state.error_message.clone(),
        error_kind: None,
        warnings: if state.warnings.is_empty() {
            None
        } else {
            Some(state.warnings.clone())
        },
        usage: Some(state.usage.clone()),
        model: model.to_string(),
        provider: PROVIDER_ID.to_string(),
        timestamp: now_ms(),
    }
}

pub fn map_stop_reason(s: &str) -> StopReason {
    match s {
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::FunctionCall,
        // end_turn, stop_sequence, anything unknown
        _ => StopReason::End,
    }
}

/// Fold an Anthropic usage payload into the running totals. Anthropic's
/// counters are cumulative (message_delta carries running totals), so each
/// present key replaces the prior value.
pub fn merge_usage(raw: &Value, into: &mut Usage) {
    let num = |k: &str| raw.get(k).and_then(Value::as_u64);
    if let Some(v) = num("input_tokens") {
        into.input = Some(v);
    }
    if let Some(v) = num("output_tokens") {
        into.output = Some(v);
    }
    if let Some(v) = num("cache_read_input_tokens") {
        into.cache_read = Some(v);
    }
    if let Some(v) = num("cache_creation_input_tokens") {
        into.cache_write = Some(v);
    }
}

/// Build a terminal error frame outside the SSE flow (fetch/HTTP failures).
pub fn synthetic_error_event(message: &str, model: &str, kind: ErrorKind) -> AssistantMessageEvent {
    synthetic_error_event_from_state(&PartialState::new(vec![]), message, model, kind)
}

/// Terminal error frame preserving any partial stream state (usage, content).
pub fn synthetic_error_event_from_state(
    state: &PartialState,
    message: &str,
    model: &str,
    kind: ErrorKind,
) -> AssistantMessageEvent {
    let mut error = build_partial(state, model);
    error.content.push(ContentBlock::Text {
        text: message.to_string(),
    });
    error.stop_reason = StopReason::Error;
    error.error_message = Some(message.to_string());
    error.error_kind = Some(kind);
    AssistantMessageEvent::Error { error }
}

fn wire_index(parsed: &Value) -> Option<usize> {
    parsed
        .get("index")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
}

fn ensure_slot(state: &mut PartialState, index: usize) {
    if state.block_slots.len() <= index {
        state.block_slots.resize(index + 1, None);
    }
}

/// Process a single SSE event block into 0+ AssistantMessageEvents.
pub fn handle_sse_event(
    block: &str,
    state: &mut PartialState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let Some(data_line) = block
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .next_back()
    else {
        return vec![];
    };
    let Ok(parsed) = serde_json::from_str::<Value>(data_line) else {
        return vec![];
    };
    let Some(event_type) = parsed.get("type").and_then(Value::as_str) else {
        return vec![];
    };
    let mut events = Vec::new();
    match event_type {
        "message_start" => {
            if let Some(u) = parsed.pointer("/message/usage") {
                merge_usage(u, &mut state.usage);
                // spec: usage SHOULD be emitted as soon as it is known
                events.push(AssistantMessageEvent::Usage {
                    usage: state.usage.clone(),
                });
            }
        }
        "content_block_start" => {
            let Some(index) = wire_index(&parsed) else {
                return events;
            };
            ensure_slot(state, index);
            let block_type = parsed
                .pointer("/content_block/type")
                .and_then(Value::as_str)
                .unwrap_or("");
            match block_type {
                "text" => {
                    let idx = state.text_blocks.len();
                    state.block_order.push((BlockKind::Text, idx));
                    state.text_blocks.push(String::new());
                    state.block_slots[index] = Some(BlockSlot::Text(idx));
                    events.push(AssistantMessageEvent::TextStart {
                        partial: build_partial(state, model),
                    });
                }
                "tool_use" => {
                    let id = parsed
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let name = parsed
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .map(decode_tool_name)
                        .unwrap_or_default();
                    let idx = state.function_calls.len();
                    state.block_order.push((BlockKind::ToolUse, idx));
                    state.function_calls.push(PartialFunctionCall {
                        id,
                        function_id: name,
                        args_json: String::new(),
                    });
                    state.block_slots[index] = Some(BlockSlot::ToolUse(idx));
                    events.push(AssistantMessageEvent::FunctioncallStart {
                        partial: build_partial(state, model),
                    });
                }
                "thinking" => {
                    let idx = state.thinking_blocks.len();
                    state.block_order.push((BlockKind::Thinking, idx));
                    state.thinking_blocks.push(PartialThinking::default());
                    state.block_slots[index] = Some(BlockSlot::Thinking(idx));
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: build_partial(state, model),
                    });
                }
                "redacted_thinking" => {
                    let data = parsed
                        .pointer("/content_block/data")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let idx = state.redacted_blocks.len();
                    state.block_order.push((BlockKind::RedactedThinking, idx));
                    state.redacted_blocks.push(data);
                    state.block_slots[index] = Some(BlockSlot::RedactedThinking);
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: build_partial(state, model),
                    });
                }
                _ => {
                    state.block_slots[index] = Some(BlockSlot::Unknown);
                }
            }
        }
        "content_block_delta" => {
            let Some(index) = wire_index(&parsed) else {
                return events;
            };
            let delta_type = parsed
                .pointer("/delta/type")
                .and_then(Value::as_str)
                .unwrap_or("");
            match state.block_slots.get(index).copied().flatten() {
                Some(BlockSlot::Text(idx)) if delta_type == "text_delta" => {
                    let text = parsed
                        .pointer("/delta/text")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    state.text_blocks[idx].push_str(text);
                    events.push(AssistantMessageEvent::TextDelta {
                        partial: build_partial(state, model),
                        delta: text.to_string(),
                    });
                }
                Some(BlockSlot::ToolUse(idx)) if delta_type == "input_json_delta" => {
                    let json = parsed
                        .pointer("/delta/partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    state.function_calls[idx].args_json.push_str(json);
                    events.push(AssistantMessageEvent::FunctioncallDelta {
                        partial: build_partial(state, model),
                        delta: json.to_string(),
                    });
                }
                Some(BlockSlot::Thinking(idx)) if delta_type == "thinking_delta" => {
                    let text = parsed
                        .pointer("/delta/thinking")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    state.thinking_blocks[idx].text.push_str(text);
                    events.push(AssistantMessageEvent::ThinkingDelta {
                        partial: build_partial(state, model),
                        delta: text.to_string(),
                    });
                }
                Some(BlockSlot::Thinking(idx)) if delta_type == "signature_delta" => {
                    let sig = parsed
                        .pointer("/delta/signature")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !sig.is_empty() {
                        state.thinking_blocks[idx]
                            .signature
                            .get_or_insert_with(String::new)
                            .push_str(sig);
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let Some(index) = wire_index(&parsed) else {
                return events;
            };
            let end_event = match state.block_slots.get(index).copied().flatten() {
                Some(BlockSlot::Thinking(_)) | Some(BlockSlot::RedactedThinking) => {
                    Some(AssistantMessageEvent::ThinkingEnd {
                        partial: build_partial(state, model),
                    })
                }
                Some(BlockSlot::ToolUse(_)) => Some(AssistantMessageEvent::FunctioncallEnd {
                    partial: build_partial(state, model),
                }),
                Some(BlockSlot::Text(_)) => Some(AssistantMessageEvent::TextEnd {
                    partial: build_partial(state, model),
                }),
                Some(BlockSlot::Unknown) | None => None,
            };
            // get_mut: a stop for an index never started must not panic.
            if let Some(slot) = state.block_slots.get_mut(index) {
                *slot = None;
            }
            if let Some(ev) = end_event {
                events.push(ev);
            }
        }
        "message_delta" => {
            if let Some(sr) = parsed.pointer("/delta/stop_reason").and_then(Value::as_str) {
                state.stop_reason = map_stop_reason(sr);
                state.native_stop_reason = Some(sr.to_string());
            }
            if let Some(u) = parsed.get("usage") {
                merge_usage(u, &mut state.usage);
                events.push(AssistantMessageEvent::Usage {
                    usage: state.usage.clone(),
                });
            }
        }
        "message_stop" => {
            state.saw_message_stop = true;
            events.push(AssistantMessageEvent::Stop {
                stop_reason: state.stop_reason,
                error_message: state.error_message.clone(),
                error_kind: None,
            });
        }
        // Mid-stream upstream error (e.g. overloaded_error): terminal error
        // frame. The TS port ignored these and emitted a clean done — fixed.
        "error" => {
            let msg = parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or(data_line)
                .to_string();
            state.stop_reason = StopReason::Error;
            state.error_message = Some(msg.clone());
            let mut error = build_partial(state, model);
            error.error_kind = Some(classify(None, data_line));
            events.push(AssistantMessageEvent::Error { error });
        }
        // ping and unknown event types: ignored
        _ => {}
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(blocks: &[&str]) -> (PartialState, Vec<AssistantMessageEvent>) {
        let mut state = PartialState::new(vec![]);
        let mut events = Vec::new();
        for b in blocks {
            events.extend(handle_sse_event(b, &mut state, "claude-test"));
        }
        (state, events)
    }

    #[test]
    fn text_stream_produces_start_delta_end_and_final_content() {
        let (state, events) = run(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"He\"}}",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"llo\"}}",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}",
        ]);
        let tags: Vec<&str> = events
            .iter()
            .map(|e| match e {
                AssistantMessageEvent::Usage { .. } => "usage",
                AssistantMessageEvent::TextStart { .. } => "text_start",
                AssistantMessageEvent::TextDelta { .. } => "text_delta",
                AssistantMessageEvent::TextEnd { .. } => "text_end",
                AssistantMessageEvent::Stop { .. } => "stop",
                _ => "other",
            })
            .collect();
        assert_eq!(
            tags,
            vec![
                "usage",
                "text_start",
                "text_delta",
                "text_delta",
                "text_end",
                "usage",
                "stop"
            ]
        );
        let final_msg = build_partial(&state, "claude-test");
        assert_eq!(
            final_msg.content,
            vec![ContentBlock::Text {
                text: "Hello".into()
            }]
        );
        assert_eq!(final_msg.stop_reason, StopReason::End);
        assert_eq!(final_msg.native_stop_reason.as_deref(), Some("end_turn"));
        let usage = final_msg.usage.unwrap();
        assert_eq!(usage.input, Some(12));
        assert_eq!(usage.output, Some(2));
    }

    #[test]
    fn tool_use_stream_decodes_name_and_parses_args() {
        let (state, events) = run(&[
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"shell__exec\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\\\":\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"ls\\\"}\"}}",
            "data: {\"type\":\"content_block_stop\",\"index\":0}",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}",
        ]);
        assert!(matches!(
            events[0],
            AssistantMessageEvent::FunctioncallStart { .. }
        ));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::FunctioncallEnd { .. })
        ));
        let final_msg = build_partial(&state, "claude-test");
        assert_eq!(final_msg.stop_reason, StopReason::FunctionCall);
        match &final_msg.content[0] {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(function_id, "shell::exec");
                assert_eq!(arguments["cmd"], "ls");
            }
            other => panic!("want function_call, got {other:?}"),
        }
    }

    #[test]
    fn interrupted_tool_use_args_degrade_to_empty_object() {
        // Stop hit mid tool-call: some partial_json arrived but the JSON never
        // closed (no content_block_stop). The accumulated args must degrade to
        // a valid object, never null — a null `tool_use.input` is a hard
        // Anthropic 400 ("Input should be an object") when the aborted turn is
        // replayed on the next request.
        let (state, _events) = run(&[
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"shell__exec\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\\\":\"}}",
        ]);
        let final_msg = build_partial(&state, "claude-test");
        match &final_msg.content[0] {
            ContentBlock::FunctionCall { arguments, .. } => assert!(
                arguments.is_object(),
                "interrupted tool args must be an object, got {arguments}"
            ),
            other => panic!("want function_call, got {other:?}"),
        }
    }

    #[test]
    fn thinking_with_signature_preserves_block_order() {
        let (state, events) = run(&[
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig1\"}}",
            "data: {\"type\":\"content_block_stop\",\"index\":0}",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"answer\"}}",
            "data: {\"type\":\"content_block_stop\",\"index\":1}",
        ]);
        assert!(matches!(
            events[0],
            AssistantMessageEvent::ThinkingStart { .. }
        ));
        let final_msg = build_partial(&state, "claude-test");
        assert_eq!(final_msg.content.len(), 2);
        match &final_msg.content[0] {
            ContentBlock::Thinking { text, signature } => {
                assert_eq!(text, "hmm");
                assert_eq!(signature.as_deref(), Some("sig1"));
            }
            other => panic!("want thinking first, got {other:?}"),
        }
        assert!(matches!(&final_msg.content[1], ContentBlock::Text { text } if text == "answer"));
    }

    #[test]
    fn mid_stream_error_event_is_terminal() {
        let (_, events) = run(&[
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"par\"}}",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}",
        ]);
        let last = events.last().unwrap();
        assert!(last.is_terminal());
        match last {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.error_message.as_deref(), Some("Overloaded"));
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
                // partial content carried on the terminal frame
                assert!(matches!(&error.content[0], ContentBlock::Text { text } if text == "par"));
            }
            other => panic!("want error frame, got {other:?}"),
        }
    }

    #[test]
    fn message_delta_usage_replaces_instead_of_adding() {
        let (state, _) = run(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":312}}",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}",
        ]);
        let usage = build_partial(&state, "claude-test").usage.unwrap();
        assert_eq!(usage.input, Some(25));
        assert_eq!(usage.output, Some(312));
    }

    #[test]
    fn unknown_block_type_does_not_corrupt_prior_tool_use() {
        let (state, events) = run(&[
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"shell__exec\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}",
            "data: {\"type\":\"content_block_stop\",\"index\":0}",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srv_1\",\"name\":\"web_search\"}}",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\"}}",
            "data: {\"type\":\"content_block_stop\",\"index\":1}",
        ]);
        let final_msg = build_partial(&state, "claude-test");
        assert_eq!(final_msg.content.len(), 1);
        match &final_msg.content[0] {
            ContentBlock::FunctionCall { arguments, .. } => {
                assert_eq!(arguments["cmd"], "ls");
            }
            other => panic!("want function_call, got {other:?}"),
        }
        assert!(!events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextEnd { .. })));
    }

    #[test]
    fn malformed_and_unknown_blocks_are_ignored() {
        let (_, events) = run(&[
            "data: not-json",
            ": comment only",
            "data: {\"no_type\":true}",
            "data: {\"type\":\"ping\"}",
        ]);
        assert!(events.is_empty());
    }

    #[test]
    fn warnings_ride_the_final_message() {
        let mut state = PartialState::new(vec!["response_format ignored".into()]);
        let _ = handle_sse_event("data: {\"type\":\"message_stop\"}", &mut state, "m");
        let final_msg = build_partial(&state, "m");
        assert_eq!(
            final_msg.warnings,
            Some(vec!["response_format ignored".to_string()])
        );
    }

    #[test]
    fn synthetic_error_event_shape() {
        let ev = synthetic_error_event("boom", "claude-test", ErrorKind::RateLimited);
        match ev {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.provider, "anthropic");
            }
            other => panic!("want error, got {other:?}"),
        }
    }
}
