//! Chat Completions chunk → AssistantMessageEvent state machine. Pure:
//! consumes one parsed chunk at a time, threads it through PartialState,
//! returns 0+ events. [DONE] is the upstream pump's concern.
use crate::errors::classify;
use crate::wire::names::decode_tool_name;
use crate::{now_ms, PROVIDER_ID};
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenBlock {
    Thinking,
    Text,
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
    /// Accumulated `delta.reasoning_content` (llama.cpp's `--reasoning-format
    /// deepseek` mode).
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
            // Unparseable args (mid-stream partials, local models misquoting
            // JSON) degrade to the salvaged leading fields or `{"_raw": …}` —
            // always an object (replay-safe) that preserves the evidence.
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
        // stop, anything unknown
        _ => StopReason::End,
    }
}

/// Last-wins merge: llama.cpp reports usage on the final SSE chunk when
/// `stream_options.include_usage` is set; overwriting (not adding) is
/// correct whether it reports once at the end or cumulatively per chunk.
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
        .and_then(Value::as_u64)
    {
        into.reasoning = Some(v);
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

/// Close the currently open block, emitting the matching end event.
fn close_open_block(
    state: &mut PartialState,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    match state.open_block.take() {
        Some(OpenBlock::Thinking) => events.push(AssistantMessageEvent::ThinkingEnd {
            partial: build_partial(state, model),
        }),
        Some(OpenBlock::Text) => events.push(AssistantMessageEvent::TextEnd {
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

    // Mid-stream error envelope (some gateways send {"error": {...}} as a
    // chunk): terminal error frame carrying the partial content.
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
        // spec: usage SHOULD be emitted as soon as it is known
        events.push(AssistantMessageEvent::Usage {
            usage: state.usage.clone(),
        });
    }

    let Some(choice) = chunk.pointer("/choices/0") else {
        return events;
    };

    if let Some(delta) = choice.get("delta") {
        // Under `--reasoning-format deepseek`, llama.cpp streams chain-of-thought
        // as `reasoning_content` deltas ahead of the answer `content`. Surface
        // it as a thinking block so the console renders the thoughts instead
        // of a bare "thinking…".
        if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
            if !reasoning.is_empty() {
                if state.open_block != Some(OpenBlock::Thinking) {
                    close_open_block(state, model, &mut events);
                    state.open_block = Some(OpenBlock::Thinking);
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: build_partial(state, model),
                    });
                }
                state.thinking.push_str(reasoning);
                events.push(AssistantMessageEvent::ThinkingDelta {
                    partial: build_partial(state, model),
                    delta: reasoning.to_string(),
                });
            }
        }
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
                    partial: build_partial(state, model),
                    delta: text.to_string(),
                });
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                // Trust boundary: api_url is operator-overridable, so a buggy
                // upstream could send an absurd index — cap it instead of
                // letting it size the Vec (OOM).
                if index > 128 {
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
                            partial: build_partial(state, model),
                            delta: args.to_string(),
                        });
                    }
                }
            }
        }
    }

    if let Some(finish) = choice.get("finish_reason").and_then(Value::as_str) {
        state.stop_reason = map_finish_reason(finish);
        state.native_stop_reason = Some(finish.to_string());
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
        let mut events = Vec::new();
        for c in chunks {
            events.extend(handle_chunk(c, &mut state, "llama-test"));
        }
        (state, events)
    }

    fn tags(events: &[AssistantMessageEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                AssistantMessageEvent::Usage { .. } => "usage",
                AssistantMessageEvent::TextStart { .. } => "text_start",
                AssistantMessageEvent::TextDelta { .. } => "text_delta",
                AssistantMessageEvent::TextEnd { .. } => "text_end",
                AssistantMessageEvent::ThinkingStart { .. } => "thinking_start",
                AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
                AssistantMessageEvent::ThinkingEnd { .. } => "thinking_end",
                AssistantMessageEvent::FunctioncallStart { .. } => "functioncall_start",
                AssistantMessageEvent::FunctioncallDelta { .. } => "functioncall_delta",
                AssistantMessageEvent::FunctioncallEnd { .. } => "functioncall_end",
                AssistantMessageEvent::Error { .. } => "error",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn text_stream_produces_start_delta_end_and_final_content() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"He"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"llo"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":2,
                "prompt_tokens_details":{"cached_tokens":4},
                "completion_tokens_details":{"reasoning_tokens":0}}}),
        ]);
        assert_eq!(
            tags(&events),
            vec![
                "text_start",
                "text_delta",
                "text_delta",
                "text_end",
                "usage"
            ]
        );
        let final_msg = build_final(&state, "llama-test");
        assert_eq!(
            final_msg.content,
            vec![ContentBlock::Text {
                text: "Hello".into()
            }]
        );
        assert_eq!(final_msg.stop_reason, StopReason::End);
        assert_eq!(final_msg.native_stop_reason.as_deref(), Some("stop"));
        let usage = final_msg.usage.unwrap();
        assert_eq!(usage.input, Some(12));
        assert_eq!(usage.output, Some(2));
        assert_eq!(usage.cache_read, Some(4));
        assert_eq!(usage.reasoning, Some(0));
    }

    #[test]
    fn reasoning_content_streams_as_thinking_block_before_text() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"let me "}}]}),
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"42"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ]);
        assert_eq!(
            tags(&events),
            vec![
                "thinking_start",
                "thinking_delta",
                "thinking_delta",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
            ]
        );
        let final_msg = build_final(&state, "llama-test");
        assert_eq!(
            final_msg.content,
            vec![
                ContentBlock::Thinking {
                    text: "let me think".into(),
                    signature: None,
                },
                ContentBlock::Text { text: "42".into() },
            ]
        );
    }

    #[test]
    fn tool_call_stream_decodes_name_and_parses_args() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_1","type":"function","function":{"name":"shell__exec","arguments":""}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"{\"cmd\":"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        assert_eq!(tags(&events)[0], "functioncall_start");
        assert_eq!(*tags(&events).last().unwrap(), "functioncall_end");
        let final_msg = build_final(&state, "llama-test");
        assert_eq!(final_msg.stop_reason, StopReason::FunctionCall);
        assert_eq!(final_msg.native_stop_reason.as_deref(), Some("tool_calls"));
        match &final_msg.content[0] {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(function_id, "shell::exec");
                assert_eq!(arguments["cmd"], "ls");
            }
            other => panic!("want function_call, got {other:?}"),
        }
    }

    // Local models misquote JSON args; the malformed text must survive as
    // `{"_raw": …}` evidence (a null would erase what the model actually
    // sent, leaving an undiagnosable empty call — and the harness turns
    // null wrapper args into a literal `agent_trigger` dispatch).
    #[test]
    fn malformed_args_survive_as_raw_evidence() {
        let (state, _) = run(&[
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_1","type":"function","function":{"name":"state__set","arguments":"{'key':"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"'v'}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        let final_msg = build_final(&state, "llama-test");
        match &final_msg.content[0] {
            ContentBlock::FunctionCall { arguments, .. } => {
                assert_eq!(arguments, &json!({ "_raw": "{'key':'v'}" }));
            }
            other => panic!("want function_call, got {other:?}"),
        }
    }

    // A wrapper call cut mid-payload keeps its already-known leading fields —
    // the console can show `ƒ state::set` while a huge payload streams.
    #[test]
    fn partial_args_salvage_leading_fields() {
        let (state, _) = run(&[json!({"choices":[{"index":0,"delta":{"tool_calls":[
            {"index":0,"id":"call_1","type":"function","function":{"name":"agent_trigger",
             "arguments":"{\"function\":\"state::set\",\"payload\":{\"key\":\"article\",\"value\":\"long"}}]}}]})]);
        let partial = build_partial(&state, "llama-test");
        match &partial.content[0] {
            ContentBlock::FunctionCall { arguments, .. } => {
                assert_eq!(arguments["function"], "state::set");
                // Salvage marker: the harness refuses to execute partial
                // intent, so it must survive the provider boundary.
                assert_eq!(arguments["_partial"], true);
            }
            other => panic!("want function_call, got {other:?}"),
        }
    }

    #[test]
    fn text_then_tool_calls_closes_text_block_first() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"content":"Let me check."}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_1","function":{"name":"web__fetch","arguments":"{}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        assert_eq!(
            tags(&events),
            vec![
                "text_start",
                "text_delta",
                "text_end",
                "functioncall_start",
                "functioncall_delta",
                "functioncall_end"
            ]
        );
        let final_msg = build_final(&state, "llama-test");
        assert_eq!(final_msg.content.len(), 2, "text block then function call");
    }

    #[test]
    fn parallel_tool_calls_emit_start_per_index() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_a","function":{"name":"f__a","arguments":"{}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":1,"id":"call_b","function":{"name":"f__b","arguments":"{}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        let starts = tags(&events)
            .iter()
            .filter(|t| **t == "functioncall_start")
            .count();
        assert_eq!(starts, 2);
        let final_msg = build_final(&state, "llama-test");
        assert_eq!(final_msg.content.len(), 2);
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
        let ev = synthetic_error_event("boom", "llama-test", ErrorKind::RateLimited);
        match ev {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.provider, "llamacpp");
            }
            other => panic!("want error, got {other:?}"),
        }
    }
}
