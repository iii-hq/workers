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
    function_calls: Vec<PartialFunctionCall>,
    /// Wire arrival order of content blocks (kind, index-within-kind-array).
    block_order: Vec<(BlockKind, usize)>,
    /// Kind of the currently open block so content_block_stop emits the
    /// matching end event.
    open_block: Option<BlockKind>,
    usage: Usage,
    stop_reason: StopReason,
    native_stop_reason: Option<String>,
    error_message: Option<String>,
    warnings: Vec<String>,
}

impl PartialState {
    pub fn new(warnings: Vec<String>) -> Self {
        PartialState {
            text_blocks: Vec::new(),
            thinking_blocks: Vec::new(),
            function_calls: Vec::new(),
            block_order: Vec::new(),
            open_block: None,
            usage: Usage::default(),
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            warnings,
        }
    }
}

pub fn empty_assistant(model: &str) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content: vec![],
        stop_reason: StopReason::End,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: None,
        usage: None,
        model: model.to_string(),
        provider: PROVIDER_ID.to_string(),
        timestamp: now_ms(),
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
                if !th.text.is_empty() {
                    out.push(ContentBlock::Thinking {
                        text: th.text.clone(),
                        signature: th.signature.clone(),
                    });
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
                let arguments = if tc.args_json.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&tc.args_json).unwrap_or(Value::Null)
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
    let mut out = Vec::new();
    let mut seen_text = vec![false; state.text_blocks.len()];
    let mut seen_thinking = vec![false; state.thinking_blocks.len()];
    let mut seen_tool = vec![false; state.function_calls.len()];
    for &(kind, idx) in &state.block_order {
        let seen = match kind {
            BlockKind::Text => &mut seen_text,
            BlockKind::Thinking => &mut seen_thinking,
            BlockKind::ToolUse => &mut seen_tool,
        };
        if idx < seen.len() && !seen[idx] {
            seen[idx] = true;
            push_block_content(&mut out, state, kind, idx);
        }
    }
    // Indices not tracked in block_order (state built directly in tests)
    // are appended grouped afterwards.
    for (i, s) in seen_thinking.iter().enumerate() {
        if !s {
            push_block_content(&mut out, state, BlockKind::Thinking, i);
        }
    }
    for (i, s) in seen_text.iter().enumerate() {
        if !s {
            push_block_content(&mut out, state, BlockKind::Text, i);
        }
    }
    for (i, s) in seen_tool.iter().enumerate() {
        if !s {
            push_block_content(&mut out, state, BlockKind::ToolUse, i);
        }
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

pub fn build_final(state: &PartialState, model: &str) -> AssistantMessage {
    build_partial(state, model)
}

pub fn map_stop_reason(s: &str) -> StopReason {
    match s {
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::FunctionCall,
        // end_turn, stop_sequence, anything unknown
        _ => StopReason::End,
    }
}

pub fn merge_usage(raw: &Value, into: &mut Usage) {
    let num = |k: &str| raw.get(k).and_then(Value::as_u64);
    if let Some(v) = num("input_tokens") {
        into.input = Some(into.input.unwrap_or(0) + v);
    }
    if let Some(v) = num("output_tokens") {
        into.output = Some(into.output.unwrap_or(0) + v);
    }
    if let Some(v) = num("cache_read_input_tokens") {
        into.cache_read = Some(into.cache_read.unwrap_or(0) + v);
    }
    if let Some(v) = num("cache_creation_input_tokens") {
        into.cache_write = Some(into.cache_write.unwrap_or(0) + v);
    }
}

/// Build a terminal error frame outside the SSE flow (fetch/HTTP failures).
pub fn synthetic_error_event(message: &str, model: &str, kind: ErrorKind) -> AssistantMessageEvent {
    let mut error = empty_assistant(model);
    error.content = vec![ContentBlock::Text {
        text: message.to_string(),
    }];
    error.stop_reason = StopReason::Error;
    error.error_message = Some(message.to_string());
    error.error_kind = Some(kind);
    AssistantMessageEvent::Error { error }
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
            let block_type = parsed
                .pointer("/content_block/type")
                .and_then(Value::as_str)
                .unwrap_or("");
            match block_type {
                "text" => {
                    state
                        .block_order
                        .push((BlockKind::Text, state.text_blocks.len()));
                    state.text_blocks.push(String::new());
                    state.open_block = Some(BlockKind::Text);
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
                    state
                        .block_order
                        .push((BlockKind::ToolUse, state.function_calls.len()));
                    state.function_calls.push(PartialFunctionCall {
                        id,
                        function_id: name,
                        args_json: String::new(),
                    });
                    state.open_block = Some(BlockKind::ToolUse);
                    events.push(AssistantMessageEvent::FunctioncallStart {
                        partial: build_partial(state, model),
                    });
                }
                "thinking" | "redacted_thinking" => {
                    // Redacted thinking is opaque and not persisted/round-tripped
                    // (needs a ContentBlock extension — follow-up); logged because
                    // the API expects it back during tool use.
                    if block_type == "redacted_thinking" {
                        eprintln!(
                            "[provider-anthropic] redacted_thinking block received; not persisted (model {model})"
                        );
                    }
                    state
                        .block_order
                        .push((BlockKind::Thinking, state.thinking_blocks.len()));
                    state.thinking_blocks.push(PartialThinking::default());
                    state.open_block = Some(BlockKind::Thinking);
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: build_partial(state, model),
                    });
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let delta_type = parsed
                .pointer("/delta/type")
                .and_then(Value::as_str)
                .unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = parsed
                        .pointer("/delta/text")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if let Some(last) = state.text_blocks.last_mut() {
                        last.push_str(text);
                    }
                    events.push(AssistantMessageEvent::TextDelta {
                        partial: build_partial(state, model),
                        delta: text.to_string(),
                    });
                }
                "input_json_delta" => {
                    let json = parsed
                        .pointer("/delta/partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if let Some(last) = state.function_calls.last_mut() {
                        last.args_json.push_str(json);
                    }
                    events.push(AssistantMessageEvent::FunctioncallDelta {
                        partial: build_partial(state, model),
                        delta: json.to_string(),
                    });
                }
                "thinking_delta" => {
                    let text = parsed
                        .pointer("/delta/thinking")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if let Some(last) = state.thinking_blocks.last_mut() {
                        last.text.push_str(text);
                    }
                    events.push(AssistantMessageEvent::ThinkingDelta {
                        partial: build_partial(state, model),
                        delta: text.to_string(),
                    });
                }
                "signature_delta" => {
                    let sig = parsed
                        .pointer("/delta/signature")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !sig.is_empty() {
                        if let Some(last) = state.thinking_blocks.last_mut() {
                            last.signature = Some(match last.signature.take() {
                                Some(mut s) => {
                                    s.push_str(sig);
                                    s
                                }
                                None => sig.to_string(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            // Emit the end event matching the open block; default to text_end
            // for unknown/untracked blocks (preserves pre-thinking behavior).
            let kind = state.open_block.take();
            events.push(match kind {
                Some(BlockKind::Thinking) => AssistantMessageEvent::ThinkingEnd {
                    partial: build_partial(state, model),
                },
                Some(BlockKind::ToolUse) => AssistantMessageEvent::FunctioncallEnd {
                    partial: build_partial(state, model),
                },
                _ => AssistantMessageEvent::TextEnd {
                    partial: build_partial(state, model),
                },
            });
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
            let mut error = build_final(state, model);
            error.error_kind = Some(classify(None, &msg));
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
        let final_msg = build_final(&state, "claude-test");
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
        let final_msg = build_final(&state, "claude-test");
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
        let final_msg = build_final(&state, "claude-test");
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
        let final_msg = build_final(&state, "m");
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
