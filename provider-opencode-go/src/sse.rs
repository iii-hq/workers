//! OpenCode Go Chat Completions chunks → AssistantMessageEvent state machine.
//! [DONE] is the upstream pump's concern.
use crate::errors::classify;
use crate::wire::names::decode_tool_name;
use crate::{now_ms, PROVIDER_ID};
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
use serde_json::Value;

/// Upper bound on a tool-call index accepted from the upstream stream;
/// larger indices are dropped (the vec would otherwise grow to reach them).
const MAX_TOOL_CALL_INDEX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenBlock {
    Text,
    Thinking,
    Call(usize),
}

#[derive(Debug, Default)]
struct PartialFunctionCall {
    id: String,
    function_id: String,
    args_json: String,
}

pub struct PartialState {
    text: String,
    thinking: String,
    function_calls: Vec<PartialFunctionCall>,
    open_block: Option<OpenBlock>,
    usage: Usage,
    usage_seen: bool,
    stop_reason: StopReason,
    native_stop_reason: Option<String>,
    error_message: Option<String>,
    warnings: Vec<String>,
}

impl PartialState {
    pub fn new(warnings: Vec<String>) -> Self {
        PartialState {
            text: String::new(),
            thinking: String::new(),
            function_calls: Vec::new(),
            open_block: None,
            usage: Usage::default(),
            usage_seen: false,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            warnings,
        }
    }

    pub fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }

    pub fn has_content(&self) -> bool {
        !self.text.is_empty() || !self.thinking.is_empty() || !self.function_calls.is_empty()
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

fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    let mut out = Vec::new();
    if !state.thinking.is_empty() {
        out.push(ContentBlock::Thinking {
            text: state.thinking.clone(),
            signature: None,
        });
    }
    if !state.text.is_empty() {
        out.push(ContentBlock::Text {
            text: state.text.clone(),
        });
    }
    for fc in &state.function_calls {
        if fc.function_id.is_empty() {
            continue;
        }
        let arguments = if fc.args_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&fc.args_json)
                .ok()
                .filter(Value::is_object)
                .unwrap_or_else(|| llm_router::types::messages::degraded_arguments(&fc.args_json))
        };
        out.push(ContentBlock::FunctionCall {
            id: fc.id.clone(),
            function_id: fc.function_id.clone(),
            arguments,
        });
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
        usage: if state.usage_seen {
            Some(state.usage.clone())
        } else {
            None
        },
        model: model.to_string(),
        provider: PROVIDER_ID.to_string(),
        timestamp: now_ms(),
    }
}

pub fn build_final(state: &PartialState, model: &str) -> AssistantMessage {
    build_partial(state, model)
}

pub fn map_finish_reason(s: &str) -> StopReason {
    match s {
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::FunctionCall,
        _ => StopReason::End,
    }
}

pub fn merge_usage(raw: &Value, into: &mut Usage) {
    let num = |k: &str| raw.get(k).and_then(Value::as_u64);
    if let Some(v) = num("prompt_tokens").or_else(|| num("input_tokens")) {
        into.input = Some(v);
    }
    if let Some(v) = num("completion_tokens").or_else(|| num("output_tokens")) {
        into.output = Some(v);
    }
    for parent in ["prompt_tokens_details", "input_tokens_details"] {
        if let Some(v) = raw
            .pointer(&format!("/{parent}/cached_tokens"))
            .and_then(Value::as_u64)
        {
            into.cache_read = Some(v);
        }
    }
    if let Some(v) = raw
        .pointer("/completion_tokens_details/reasoning_tokens")
        .or_else(|| raw.pointer("/output_tokens_details/reasoning_tokens"))
        .and_then(Value::as_u64)
    {
        into.reasoning = Some(v);
    }
}

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

fn close_open_block(
    state: &mut PartialState,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    match state.open_block.take() {
        Some(OpenBlock::Text) => events.push(AssistantMessageEvent::TextEnd {
            partial: build_partial(state, model),
        }),
        Some(OpenBlock::Thinking) => events.push(AssistantMessageEvent::ThinkingEnd {
            partial: build_partial(state, model),
        }),
        Some(OpenBlock::Call(_)) => events.push(AssistantMessageEvent::FunctioncallEnd {
            partial: build_partial(state, model),
        }),
        None => {}
    }
}

/// Process one parsed Chat Completions chunk into 0+ AssistantMessageEvents.
pub fn handle_chunk(
    chunk: &Value,
    state: &mut PartialState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let mut events = Vec::new();

    // Mid-stream error envelope (some gateways send {"error": {...}} as a chunk)
    if let Some(err) = chunk.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("upstream error")
            .to_string();
        state.stop_reason = StopReason::Error;
        state.error_message = Some(msg.clone());
        let mut error = build_final(state, model);
        error.error_kind = Some(classify(None, &chunk.to_string()));
        events.push(AssistantMessageEvent::Error { error });
        return events;
    }

    if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
        merge_usage(usage, &mut state.usage);
        state.usage_seen = true;
        events.push(AssistantMessageEvent::Usage {
            usage: state.usage.clone(),
        });
    }

    let Some(choice) = chunk.pointer("/choices/0") else {
        return events;
    };

    if let Some(delta) = choice.get("delta") {
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                if state.open_block != Some(OpenBlock::Text) {
                    close_open_block(state, model, &mut events);
                    state.open_block = Some(OpenBlock::Text);
                    events.push(AssistantMessageEvent::TextStart {
                        partial: build_partial(state, model),
                    });
                }
                state.text.push_str(text);
                events.push(AssistantMessageEvent::TextDelta {
                    partial: None,
                    delta: text.to_string(),
                });
            }
        }
        if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
            if !text.is_empty() {
                if state.open_block != Some(OpenBlock::Thinking) {
                    close_open_block(state, model, &mut events);
                    state.open_block = Some(OpenBlock::Thinking);
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: build_partial(state, model),
                    });
                }
                state.thinking.push_str(text);
                events.push(AssistantMessageEvent::ThinkingDelta {
                    partial: None,
                    delta: text.to_string(),
                });
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                // A hostile or malformed upstream could name an arbitrary
                // index; the while loop below grows the vec to reach it.
                if index >= MAX_TOOL_CALL_INDEX {
                    tracing::debug!(index, "dropping tool-call delta with oversized index");
                    continue;
                }
                while state.function_calls.len() <= index {
                    state.function_calls.push(PartialFunctionCall::default());
                }
                if state.open_block != Some(OpenBlock::Call(index)) {
                    close_open_block(state, model, &mut events);
                    state.open_block = Some(OpenBlock::Call(index));
                    events.push(AssistantMessageEvent::FunctioncallStart {
                        partial: build_partial(state, model),
                    });
                }
                let entry = &mut state.function_calls[index];
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    if !id.is_empty() {
                        entry.id = id.to_string();
                    }
                }
                if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                    if !name.is_empty() {
                        entry.function_id = decode_tool_name(name);
                    }
                }
                if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                    if !args.is_empty() {
                        state.function_calls[index].args_json.push_str(args);
                        events.push(AssistantMessageEvent::FunctioncallDelta {
                            partial: None,
                            delta: args.to_string(),
                            id: state.function_calls[index].id.clone(),
                        });
                    }
                }
            }
        }
    }

    if let Some(finish) = choice.get("finish_reason").and_then(Value::as_str) {
        state.stop_reason = map_finish_reason(finish);
        state.native_stop_reason = Some(finish.to_string());
        if finish == "content_filter" {
            state.warnings.push(
                "opencode_go filtered the completion (finish_reason: content_filter)".to_string(),
            );
        }
        close_open_block(state, model, &mut events);
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(chunks: &[Value]) -> (PartialState, Vec<AssistantMessageEvent>) {
        let mut state = PartialState::new(vec![]);
        let mut all = Vec::new();
        for chunk in chunks {
            all.extend(handle_chunk(chunk, &mut state, "m"));
        }
        (state, all)
    }

    #[test]
    fn text_chunks_have_start_delta_end() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"Hel"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"lo"}}]}),
        ]);
        assert_eq!(state.text, "Hello");
        assert!(matches!(events[0], AssistantMessageEvent::TextStart { .. }));
        assert!(matches!(events[1], AssistantMessageEvent::TextDelta { .. }));
        assert!(matches!(events[2], AssistantMessageEvent::TextDelta { .. }));
    }

    #[test]
    fn start_through_stop_and_done() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"Hi"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ]);
        assert_eq!(state.stop_reason, StopReason::End);
        assert_eq!(state.native_stop_reason.as_deref(), Some("stop"));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextDelta { .. })));
    }

    #[test]
    fn finish_reason_length() {
        let (state, _) = run(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
        ]);
        assert_eq!(state.stop_reason, StopReason::Length);
    }

    #[test]
    fn finish_reason_tool_calls() {
        let (state, _) =
            run(&[json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})]);
        assert_eq!(state.stop_reason, StopReason::FunctionCall);
    }

    #[test]
    fn usage_emits_as_event() {
        let (_, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"content":"Hi"}}]}),
            json!({"usage":{"prompt_tokens":10,"completion_tokens":1}}),
        ]);
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::Usage { .. })));
    }

    #[test]
    fn tool_call_flow() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"shell__exec","arguments":""}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        assert_eq!(state.stop_reason, StopReason::FunctionCall);
        assert_eq!(state.function_calls[0].id, "call_1");
        assert_eq!(state.function_calls[0].function_id, "shell::exec");
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::FunctioncallStart { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::FunctioncallDelta { .. })));
    }

    #[test]
    fn mid_stream_error_chunk_is_terminal_with_partial_content() {
        let (_, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"content":"par"}}]}),
            json!({"error":{"message":"The server is overloaded","type":"server_error"}}),
        ]);
        let last = events.last().unwrap();
        assert!(last.is_terminal());
        match last {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(
                    error.error_message.as_deref(),
                    Some("The server is overloaded")
                );
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
                assert!(matches!(&error.content[0], ContentBlock::Text { text } if text == "par"));
            }
            other => panic!("want error frame, got {other:?}"),
        }
    }

    #[test]
    fn malformed_and_empty_chunks_are_ignored() {
        let (_, events) = run(&[
            json!({"no_choices": true}),
            json!({"choices": []}),
            json!({"choices":[{"index":0}]}),
            json!({"choices":[{"index":0,"delta":{"content":""}}]}),
        ]);
        assert!(events.is_empty());
    }

    #[test]
    fn warnings_ride_the_final_message() {
        let state = PartialState::new(vec!["response_format degraded".into()]);
        let final_msg = build_final(&state, "m");
        assert_eq!(
            final_msg.warnings,
            Some(vec!["response_format degraded".to_string()])
        );
    }

    #[test]
    fn synthetic_error_event_shape() {
        let ev = synthetic_error_event("boom", "deepseek-v4-flash", ErrorKind::RateLimited);
        match ev {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.provider, "opencode_go");
            }
            other => panic!("want error, got {other:?}"),
        }
    }
}
