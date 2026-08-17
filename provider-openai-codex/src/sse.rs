//! OpenAI Responses SSE event → AssistantMessageEvent state machine. Pure:
//! consumes one parsed event `data` object at a time (keyed off its `type`),
//! threads it through PartialState, returns 0+ events. Keeps the same public
//! API as the Chat Completions provider so `upstream.rs` is unchanged.
//! Unknown event types are ignored (forward-compat: the backend adds events).
use crate::wire::names::decode_tool_name;
use crate::{now_ms, PROVIDER_ID};
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenBlock {
    Text,
    Thinking,
    Call,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    function_id: String,
    args_json: String,
}

pub struct PartialState {
    text: String,
    thinking: String,
    tool_calls: Vec<PartialToolCall>,
    open_block: Option<OpenBlock>,
    usage: Usage,
    usage_seen: bool,
    stop_reason: StopReason,
    error_message: Option<String>,
    warnings: Vec<String>,
}

impl PartialState {
    pub fn new(warnings: Vec<String>) -> Self {
        PartialState {
            text: String::new(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            open_block: None,
            usage: Usage::default(),
            usage_seen: false,
            stop_reason: StopReason::End,
            error_message: None,
            warnings,
        }
    }

    pub fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }

    /// Whether any output accumulated (text, thinking, or tool calls) —
    /// distinguishes legitimate close-framing from a truncated/empty stream.
    pub fn has_content(&self) -> bool {
        !self.text.is_empty() || !self.thinking.is_empty() || !self.tool_calls.is_empty()
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
    for tc in &state.tool_calls {
        if tc.function_id.is_empty() {
            continue;
        }
        let arguments = if tc.args_json.is_empty() {
            serde_json::json!({})
        } else {
            // Unparseable args (mid-stream partials, malformed JSON) degrade
            // to the salvaged leading fields or `{"_raw": …}` — always an
            // object (replay-safe) that preserves the evidence.
            serde_json::from_str(&tc.args_json)
                .ok()
                .filter(Value::is_object)
                .unwrap_or_else(|| llm_router::types::messages::degraded_arguments(&tc.args_json))
        };
        out.push(ContentBlock::FunctionCall {
            id: tc.id.clone(),
            function_id: tc.function_id.clone(),
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
        native_stop_reason: None,
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

/// Merge Responses usage (`response.usage` or top-level `usage`). Last-wins.
pub fn merge_usage(data: &Value, into: &mut Usage) {
    let Some(u) = data
        .get("usage")
        .or_else(|| data.pointer("/response/usage"))
    else {
        return;
    };
    let num = |k: &str| u.get(k).and_then(Value::as_u64);
    // `Usage.input` is the cache-MISS slice, disjoint from `cache_read`
    // (pricing bills the splits additively). The Responses API's
    // `input_tokens` is a TOTAL that includes the cached slice, so the miss
    // slice is derived — mapping the total verbatim would bill the cached
    // prefix twice.
    let cached = u
        .pointer("/input_tokens_details/cached_tokens")
        .or_else(|| u.get("cached_tokens"))
        .and_then(Value::as_u64);
    if let Some(v) = cached {
        into.cache_read = Some(v);
    }
    if let Some(v) = num("input_tokens").or_else(|| num("prompt_tokens")) {
        into.input = Some(v.saturating_sub(cached.unwrap_or(0)));
    }
    if let Some(v) = num("output_tokens").or_else(|| num("completion_tokens")) {
        into.output = Some(v);
    }
    if let Some(v) = u
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
    {
        into.reasoning = Some(v);
    }
}

/// Terminal error frame for out-of-band failures (fetch/HTTP).
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
        Some(OpenBlock::Call) => events.push(AssistantMessageEvent::FunctioncallEnd {
            partial: build_partial(state, model),
        }),
        None => {}
    }
}

fn open_text(state: &mut PartialState, model: &str, events: &mut Vec<AssistantMessageEvent>) {
    if state.open_block != Some(OpenBlock::Text) {
        close_open_block(state, model, events);
        state.open_block = Some(OpenBlock::Text);
        events.push(AssistantMessageEvent::TextStart {
            partial: build_partial(state, model),
        });
    }
}

fn open_thinking(state: &mut PartialState, model: &str, events: &mut Vec<AssistantMessageEvent>) {
    if state.open_block != Some(OpenBlock::Thinking) {
        close_open_block(state, model, events);
        state.open_block = Some(OpenBlock::Thinking);
        events.push(AssistantMessageEvent::ThinkingStart {
            partial: build_partial(state, model),
        });
    }
}

fn str_delta(data: &Value) -> String {
    data.get("delta")
        .and_then(|d| d.as_str().or_else(|| d.get("text").and_then(Value::as_str)))
        .or_else(|| data.get("text").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// Process one parsed Responses event `data` object into 0+ events.
pub fn handle_chunk(
    chunk: &Value,
    state: &mut PartialState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let mut events = Vec::new();
    let name = chunk.get("type").and_then(Value::as_str).unwrap_or("");

    if name.is_empty() {
        // Some gateways send a bare {"error": {...}} object.
        if let Some(err) = chunk.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("upstream error")
                .to_string();
            state.stop_reason = StopReason::Error;
            state.error_message = Some(msg.clone());
            let mut error = build_final(state, model);
            error.error_kind = Some(crate::errors::classify(None, &chunk.to_string()));
            events.push(AssistantMessageEvent::Error { error });
        }
        return events;
    }

    match name {
        "response.created" => {}
        n if n.contains("output_item.added") => {
            let Some(item) = chunk.get("item").or_else(|| chunk.get("output_item")) else {
                return events;
            };
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let fname = item
                    .get("name")
                    .and_then(Value::as_str)
                    .map(decode_tool_name)
                    .unwrap_or_default();
                state.tool_calls.push(PartialToolCall {
                    id,
                    function_id: fname,
                    args_json: String::new(),
                });
                close_open_block(state, model, &mut events);
                state.open_block = Some(OpenBlock::Call);
                events.push(AssistantMessageEvent::FunctioncallStart {
                    partial: build_partial(state, model),
                });
            }
        }
        n if n.contains("output_text.delta") => {
            let delta = str_delta(chunk);
            if delta.is_empty() {
                return events;
            }
            open_text(state, model, &mut events);
            state.text.push_str(&delta);
            events.push(AssistantMessageEvent::TextDelta {
                partial: None,
                delta,
            });
        }
        n if n.contains("reasoning_summary_text.delta") || n.contains("reasoning_text.delta") => {
            let delta = str_delta(chunk);
            if delta.is_empty() {
                return events;
            }
            open_thinking(state, model, &mut events);
            state.thinking.push_str(&delta);
            events.push(AssistantMessageEvent::ThinkingDelta {
                partial: None,
                delta,
            });
        }
        n if n.contains("function_call_arguments.delta") => {
            let delta = chunk
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| chunk.get("arguments_delta").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            if delta.is_empty() {
                return events;
            }
            if let Some(last) = state.tool_calls.last_mut() {
                last.args_json.push_str(&delta);
            }
            events.push(AssistantMessageEvent::FunctioncallDelta {
                partial: None,
                delta,
                id: state
                    .tool_calls
                    .last()
                    .map(|c| c.id.clone())
                    .unwrap_or_default(),
            });
        }
        n if n.contains("completed") => {
            merge_usage(chunk, &mut state.usage);
            state.usage_seen = state.usage != Usage::default();
            state.stop_reason = if state.tool_calls.is_empty() {
                StopReason::End
            } else {
                StopReason::FunctionCall
            };
            close_open_block(state, model, &mut events);
            events.push(AssistantMessageEvent::Stop {
                stop_reason: state.stop_reason,
                error_message: None,
                error_kind: None,
            });
            events.push(AssistantMessageEvent::Done {
                message: build_final(state, model),
            });
        }
        // `error` is the Responses API's stream-fatal event: the message sits
        // at TOP level (`{"type":"error","code":…,"message":…}`), unlike
        // `response.failed`'s nested `response.error`. The body's message
        // lookup covers both shapes.
        n if n == "error"
            || n.contains("failed")
            || n.contains("incomplete")
            || (n.contains(".error") && !n.contains("delta")) =>
        {
            let msg = chunk
                .pointer("/error/message")
                .or_else(|| chunk.pointer("/response/error/message"))
                .and_then(Value::as_str)
                .or_else(|| chunk.get("message").and_then(Value::as_str))
                .unwrap_or("Codex stream reported failure")
                .to_string();
            state.stop_reason = StopReason::Error;
            state.error_message = Some(msg.clone());
            let mut error = build_final(state, model);
            error.error_kind = Some(crate::errors::classify(None, &msg));
            events.push(AssistantMessageEvent::Error { error });
        }
        _ => {}
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(events_in: &[Value]) -> (PartialState, Vec<AssistantMessageEvent>) {
        let mut state = PartialState::new(vec![]);
        let mut out = Vec::new();
        for c in events_in {
            out.extend(handle_chunk(c, &mut state, "codex/gpt-5.5"));
        }
        (state, out)
    }

    /// Contract pin (llm-router types::events): delta frames are slim —
    /// no cumulative partial per chunk — while block-boundary frames carry
    /// the authoritative snapshot (cumulative text here). Readers
    /// reconstruct via llm_router::chat::accumulate.
    #[test]
    fn deltas_are_slim_and_boundary_snapshots_are_cumulative() {
        let (_, events) = run(&[
            json!({ "type": "response.output_text.delta", "delta": "He" }),
            json!({ "type": "response.output_text.delta", "delta": "llo" }),
            json!({ "type": "response.completed" }),
        ]);
        for ev in &events {
            if let AssistantMessageEvent::TextDelta { partial, .. } = ev {
                assert!(partial.is_none(), "delta frames must not carry partial");
            }
        }
        let Some(AssistantMessageEvent::TextEnd { partial }) = events
            .iter()
            .find(|e| matches!(e, AssistantMessageEvent::TextEnd { .. }))
        else {
            panic!("want a text_end frame");
        };
        assert!(
            matches!(&partial.content[0], ContentBlock::Text { text } if text == "Hello"),
            "the End snapshot must carry the cumulative block text"
        );
    }

    #[test]
    fn text_stream_yields_start_delta_and_completed_done() {
        let (state, events) = run(&[
            json!({ "type": "response.created" }),
            json!({ "type": "response.output_text.delta", "delta": "He" }),
            json!({ "type": "response.output_text.delta", "delta": "llo" }),
            json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 3, "output_tokens": 2 } } }),
        ]);
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantMessageEvent::TextStart { .. })));
        let done = events.last().unwrap();
        match done {
            AssistantMessageEvent::Done { message } => {
                assert!(
                    matches!(&message.content[0], ContentBlock::Text { text } if text == "Hello")
                );
                assert_eq!(message.usage.as_ref().unwrap().input, Some(3));
                assert_eq!(message.usage.as_ref().unwrap().output, Some(2));
            }
            other => panic!("want done, got {other:?}"),
        }
        assert_eq!(state.stop_reason, StopReason::End);
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[test]
    fn usage_input_excludes_the_cached_slice() {
        // `input_tokens` on the wire is a TOTAL including the cached prefix;
        // `Usage.input` is the disjoint miss slice pricing bills at the full
        // input rate. 10 total with 4 cached reads as 6 in, 4 cache_read.
        let (_state, events) = run(&[
            json!({ "type": "response.created" }),
            json!({ "type": "response.output_text.delta", "delta": "Hello" }),
            json!({ "type": "response.completed", "response": { "usage": {
                "input_tokens": 10,
                "output_tokens": 2,
                "input_tokens_details": { "cached_tokens": 4 }
            } } }),
        ]);
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                let usage = message.usage.as_ref().unwrap();
                assert_eq!(usage.input, Some(6));
                assert_eq!(usage.cache_read, Some(4));
            }
            other => panic!("want done, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_decodes_name_and_parses_args() {
        let (_, events) = run(&[
            json!({ "type": "response.output_item.added", "item": { "type": "function_call", "call_id": "c1", "name": "shell__exec" } }),
            json!({ "type": "response.function_call_arguments.delta", "delta": "{\"cmd\":" }),
            json!({ "type": "response.function_call_arguments.delta", "delta": "\"ls\"}" }),
            json!({ "type": "response.completed" }),
        ]);
        match events.last().unwrap() {
            AssistantMessageEvent::Done { message } => match &message.content[0] {
                ContentBlock::FunctionCall {
                    id,
                    function_id,
                    arguments,
                } => {
                    assert_eq!(id, "c1");
                    assert_eq!(function_id, "shell::exec");
                    assert_eq!(arguments["cmd"], "ls");
                }
                other => panic!("want function_call, got {other:?}"),
            },
            other => panic!("want done, got {other:?}"),
        }
    }

    #[test]
    fn unknown_events_are_ignored() {
        let (_, events) = run(&[json!({ "type": "response.some_future_event", "x": 1 })]);
        assert!(events.is_empty());
    }

    #[test]
    fn failed_event_is_terminal_error() {
        let (_, events) = run(&[
            json!({ "type": "response.failed", "response": { "error": { "message": "boom" } } }),
        ]);
        match events.last().unwrap() {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_message.as_deref(), Some("boom"));
                assert_eq!(error.stop_reason, StopReason::Error);
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[test]
    fn top_level_error_event_is_terminal_error() {
        // The Responses API's stream-fatal signal: `{"type":"error"}` with the
        // message at TOP level (no nested `error` object, no `response.`
        // prefix). Swallowing it ends the run as an empty `done` or a
        // ping-until-timeout "ended without a terminal frame" (MOT-3855).
        let (_, events) = run(&[
            json!({ "type": "response.output_text.delta", "delta": "par" }),
            json!({ "type": "error", "code": "server_error", "message": "The model produced no output", "sequence_number": 7 }),
        ]);
        match events.last().unwrap() {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(
                    error.error_message.as_deref(),
                    Some("The model produced no output")
                );
                assert_eq!(error.stop_reason, StopReason::Error);
            }
            other => panic!("want error, got {other:?}"),
        }
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
    }
}
