//! Interactions SSE event → AssistantMessageEvent state machine. Pure:
//! consumes one parsed event at a time, threads it through PartialState,
//! returns 0+ events.
//!
//! Every step carries an upstream `index`; each segment stores it, so a delta
//! or stop finds its block by index. Live streams are strictly sequential
//! (start, deltas, stop, one step after the other, parallel calls too); for a
//! proxy that interleaves, a delta for an already closed step updates it
//! silently and the next boundary snapshot carries it.
use crate::errors::classify_error;
use crate::wire::names::decode_tool_name;
use crate::{PROVIDER_ID, now_ms};
use llm_router::provider_scaffold::sse_transport::{StreamEndView, arguments_incomplete};
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
use serde_json::Value;

#[derive(Debug, Default)]
struct PartialFunctionCall {
    id: String,
    function_id: String,
    args_json: String,
}

/// One content block, in arrival order, tagged with its upstream step index.
#[derive(Debug)]
enum Block {
    Thinking {
        text: String,
        signature: Option<String>,
    },
    Text(String),
    Call(PartialFunctionCall),
}

#[derive(Debug)]
struct Segment {
    index: u64,
    block: Block,
}

pub struct PartialState {
    segments: Vec<Segment>,
    /// Position in `segments` of the block currently open, if any.
    open: Option<usize>,
    /// A `step.start` arrived: the model started (a thought step can carry a
    /// signature and no text, so text alone is no proof).
    started: bool,
    usage: Usage,
    usage_seen: bool,
    stop_reason: StopReason,
    native_stop_reason: Option<String>,
    error_message: Option<String>,
    warnings: Vec<String>,
    terminated: bool,
}

impl PartialState {
    pub fn new(warnings: Vec<String>) -> Self {
        PartialState {
            segments: Vec::new(),
            open: None,
            started: false,
            usage: Usage::default(),
            usage_seen: false,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            warnings,
            terminated: false,
        }
    }

    pub fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }

    pub(crate) fn generation_started(&self) -> bool {
        self.started || !self.segments.is_empty()
    }

    /// An empty delta for the open thought or text block: progress for the
    /// router's idle clock, a no-op for its accumulator.
    pub(crate) fn heartbeat(&self) -> Option<AssistantMessageEvent> {
        match &self.segments.get(self.open?)?.block {
            Block::Thinking { .. } => Some(AssistantMessageEvent::ThinkingDelta {
                partial: None,
                delta: String::new(),
            }),
            Block::Text(_) => Some(AssistantMessageEvent::TextDelta {
                partial: None,
                delta: String::new(),
            }),
            Block::Call(_) => None,
        }
    }

    fn slot(&self, index: u64) -> Option<usize> {
        self.segments.iter().position(|s| s.index == index)
    }

    fn has_call(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(&s.block, Block::Call(c) if !c.function_id.is_empty()))
    }
}

impl StreamEndView for PartialState {
    fn saw_terminator(&self) -> bool {
        self.terminated
    }

    fn has_content(&self) -> bool {
        !self.segments.is_empty()
    }

    fn has_unfinished_call(&self) -> bool {
        let open_call = self
            .open
            .and_then(|i| self.segments.get(i))
            .is_some_and(|s| matches!(s.block, Block::Call(_)));
        open_call
            || self.segments.iter().any(|s| match &s.block {
                Block::Call(c) => arguments_incomplete(&c.args_json),
                _ => false,
            })
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

/// Segments → content blocks, in arrival order. Empty segments are dropped:
/// a block that opened but never received a delta carries nothing, and a tool
/// call whose name never arrived is not invocable. A thought with only a
/// signature is kept: the signature is what replay needs.
fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    state
        .segments
        .iter()
        .filter_map(|segment| match &segment.block {
            Block::Thinking { text, signature } if !text.is_empty() || signature.is_some() => {
                Some(ContentBlock::Thinking {
                    text: text.clone(),
                    signature: signature.clone(),
                })
            }
            Block::Text(text) if !text.is_empty() => {
                Some(ContentBlock::Text { text: text.clone() })
            }
            Block::Call(fc) if !fc.function_id.is_empty() => {
                let arguments = if fc.args_json.is_empty() {
                    serde_json::json!({})
                } else {
                    // Unparseable args (mid-stream partials, malformed JSON)
                    // degrade to the salvaged leading fields or `{"_raw": …}` —
                    // always an object (replay-safe) that preserves the evidence.
                    serde_json::from_str(&fc.args_json)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| {
                            llm_router::types::messages::degraded_arguments(&fc.args_json)
                        })
                };
                Some(ContentBlock::FunctionCall {
                    id: fc.id.clone(),
                    function_id: fc.function_id.clone(),
                    arguments,
                })
            }
            _ => None,
        })
        .collect()
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

/// Last-wins merge of an Interactions `usage` object. Cached tokens are
/// inside `total_input_tokens`; `total_output_tokens` excludes thought tokens,
/// which bill at the output rate, and `fill_cost_usd` bills `output` only, so
/// thought tokens go into `output` as well as `reasoning`.
pub fn merge_usage(raw: &Value, into: &mut Usage) {
    let num = |k: &str| raw.get(k).and_then(Value::as_u64);
    let cached = num("total_cached_tokens");
    if cached.is_some() {
        into.cache_read = cached;
    }
    if let Some(input) = num("total_input_tokens") {
        into.input = Some(input.saturating_sub(cached.unwrap_or(0)));
    }
    let thought = num("total_thought_tokens");
    if let Some(output) = num("total_output_tokens") {
        into.output = Some(output + thought.unwrap_or(0));
    }
    if thought.is_some() {
        into.reasoning = thought;
    }
}

/// Build a terminal error frame outside the SSE flow (fetch/HTTP failures).
/// Diagnostics belong in error_message; no model output was generated.
pub fn synthetic_error_event(message: &str, model: &str, kind: ErrorKind) -> AssistantMessageEvent {
    let mut error = empty_assistant(model);
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
    let Some(slot) = state.open.take() else {
        return;
    };
    let partial = build_partial(state, model);
    events.push(match &state.segments[slot].block {
        Block::Thinking { .. } => AssistantMessageEvent::ThinkingEnd { partial },
        Block::Text(_) => AssistantMessageEvent::TextEnd { partial },
        Block::Call(_) => AssistantMessageEvent::FunctioncallEnd { partial },
    });
}

/// Make a new segment for `index` the open block. One block is open at a time
/// (the router accumulator applies a delta to its single open block), so a
/// different open block closes first. A known segment is never reopened: the
/// accumulator seeds a `*Start` from the snapshot's last block, which a
/// reopened segment is not. Callers emit a delta only when the returned slot
/// is open. `make` builds the segment when the index is new.
fn open_slot(
    state: &mut PartialState,
    index: u64,
    make: impl FnOnce() -> Block,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) -> usize {
    if let Some(slot) = state.slot(index) {
        return slot;
    }
    close_open_block(state, model, events);
    state.segments.push(Segment {
        index,
        block: make(),
    });
    let slot = state.segments.len() - 1;
    state.open = Some(slot);
    let partial = build_partial(state, model);
    events.push(match &state.segments[slot].block {
        Block::Thinking { .. } => AssistantMessageEvent::ThinkingStart { partial },
        Block::Text(_) => AssistantMessageEvent::TextStart { partial },
        Block::Call(_) => AssistantMessageEvent::FunctioncallStart { partial },
    });
    slot
}

fn thinking() -> Block {
    Block::Thinking {
        text: String::new(),
        signature: None,
    }
}

fn push_thought_text(
    state: &mut PartialState,
    index: u64,
    text: &str,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    let slot = open_slot(state, index, thinking, model, events);
    if let Block::Thinking { text: t, .. } = &mut state.segments[slot].block {
        t.push_str(text);
    }
    if state.open == Some(slot) {
        events.push(AssistantMessageEvent::ThinkingDelta {
            partial: None,
            delta: text.to_string(),
        });
    }
}

fn set_signature(
    state: &mut PartialState,
    index: u64,
    signature: &str,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    let slot = match state.slot(index) {
        Some(slot) => slot,
        None => open_slot(state, index, thinking, model, events),
    };
    // No event: the signature rides the ThinkingEnd snapshot.
    if let Block::Thinking { signature: s, .. } = &mut state.segments[slot].block {
        *s = Some(signature.to_string());
    }
}

fn push_text(
    state: &mut PartialState,
    index: u64,
    text: &str,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    if text.is_empty() {
        return;
    }
    let slot = open_slot(state, index, || Block::Text(String::new()), model, events);
    if let Block::Text(t) = &mut state.segments[slot].block {
        t.push_str(text);
    }
    if state.open == Some(slot) {
        events.push(AssistantMessageEvent::TextDelta {
            partial: None,
            delta: text.to_string(),
        });
    }
}

fn push_arguments(
    state: &mut PartialState,
    index: u64,
    args: &str,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    if args.is_empty() {
        return;
    }
    let slot = open_slot(
        state,
        index,
        || Block::Call(PartialFunctionCall::default()),
        model,
        events,
    );
    let open = state.open == Some(slot);
    if let Block::Call(c) = &mut state.segments[slot].block {
        c.args_json.push_str(args);
        if open {
            events.push(AssistantMessageEvent::FunctioncallDelta {
                partial: None,
                delta: args.to_string(),
                id: c.id.clone(),
                arguments_preview: None,
            });
        }
    }
}

/// Text items of a `content` array (`step.start` may carry them).
fn content_texts(content: Option<&Value>) -> impl Iterator<Item = &str> {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "text")
        .filter_map(|c| c["text"].as_str())
}

fn step_start(
    event: &Value,
    index: u64,
    state: &mut PartialState,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    let step = &event["step"];
    match step["type"].as_str() {
        Some("thought") => {
            open_slot(state, index, thinking, model, events);
            for text in content_texts(step.get("summary")) {
                push_thought_text(state, index, text, model, events);
            }
            if let Some(sig) = step["signature"].as_str() {
                set_signature(state, index, sig, model, events);
            }
        }
        Some("model_output") => {
            for text in content_texts(step.get("content")) {
                push_text(state, index, text, model, events);
            }
        }
        Some("function_call") => {
            let id = match step["id"].as_str() {
                Some(id) if !id.is_empty() => id.to_string(),
                _ => format!("call_{}", uuid::Uuid::new_v4().simple()),
            };
            let function_id = decode_tool_name(step["name"].as_str().unwrap_or(""));
            open_slot(
                state,
                index,
                || {
                    Block::Call(PartialFunctionCall {
                        id,
                        function_id,
                        args_json: String::new(),
                    })
                },
                model,
                events,
            );
            // Live, `arguments` is `{}` here and the JSON follows in a delta.
            if let Some(args) = step["arguments"].as_object()
                && !args.is_empty()
            {
                push_arguments(state, index, &step["arguments"].to_string(), model, events);
            }
        }
        _ => {}
    }
}

fn step_delta(
    event: &Value,
    index: u64,
    state: &mut PartialState,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    let delta = &event["delta"];
    match delta["type"].as_str() {
        Some("text") => push_text(
            state,
            index,
            delta["text"].as_str().unwrap_or(""),
            model,
            events,
        ),
        Some("thought_summary") => {
            if delta["content"]["type"] == "text"
                && let Some(text) = delta["content"]["text"].as_str()
            {
                push_thought_text(state, index, text, model, events);
            }
        }
        Some("thought_signature") => {
            if let Some(sig) = delta["signature"].as_str() {
                set_signature(state, index, sig, model, events);
            }
        }
        Some("arguments_delta") => push_arguments(
            state,
            index,
            delta["arguments"].as_str().unwrap_or(""),
            model,
            events,
        ),
        _ => {}
    }
}

fn error_frame(
    state: &mut PartialState,
    model: &str,
    kind: ErrorKind,
    message: String,
) -> AssistantMessageEvent {
    state.stop_reason = StopReason::Error;
    state.error_message = Some(message);
    let mut error = build_final(state, model);
    error.error_kind = Some(kind);
    AssistantMessageEvent::Error { error }
}

fn completed(
    interaction: &Value,
    state: &mut PartialState,
    model: &str,
    events: &mut Vec<AssistantMessageEvent>,
) {
    state.terminated = true;
    close_open_block(state, model, events);
    let status = interaction["status"].as_str().unwrap_or("");
    state.native_stop_reason = Some(status.to_string());
    let has_call = state.has_call();
    state.stop_reason = match status {
        "completed" | "requires_action" if has_call => StopReason::FunctionCall,
        "requires_action" => {
            state
                .warnings
                .push("gemini ended with requires_action but sent no function call".into());
            StopReason::End
        }
        "incomplete" => StopReason::Length,
        "requires_continuation" => {
            state.warnings.push(
                "gemini asked for a continuation (requires_continuation); long-decode \
                 continuation is not implemented"
                    .into(),
            );
            StopReason::Length
        }
        "failed" => {
            let error = &interaction["errors"][0];
            let kind = if error.is_object() {
                classify_error(None, error)
            } else {
                ErrorKind::Transient
            };
            let message = error["message"]
                .as_str()
                .unwrap_or("gemini interaction failed")
                .to_string();
            events.push(error_frame(state, model, kind, message));
            return;
        }
        "cancelled" => {
            events.push(error_frame(
                state,
                model,
                ErrorKind::Transient,
                "gemini interaction was cancelled".into(),
            ));
            return;
        }
        // A completed message may hold only a thought step: still End.
        _ => StopReason::End,
    };
    events.push(AssistantMessageEvent::Stop {
        stop_reason: state.stop_reason,
        error_message: None,
        error_kind: None,
    });
    events.push(AssistantMessageEvent::Done {
        message: build_final(state, model),
    });
}

/// Process one parsed Interactions SSE event into 0+ AssistantMessageEvents.
pub fn handle_event(
    event: &Value,
    state: &mut PartialState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let mut events = Vec::new();
    let index = event["index"].as_u64().unwrap_or(0);
    match event["event_type"].as_str().unwrap_or("") {
        "step.start" => {
            state.started = true;
            step_start(event, index, state, model, &mut events);
        }
        "step.delta" => step_delta(event, index, state, model, &mut events),
        "step.stop" => {
            if state.open.is_some() && state.open == state.slot(index) {
                close_open_block(state, model, &mut events);
            }
        }
        "interaction.completed" => {
            let interaction = &event["interaction"];
            if let Some(usage) = interaction.get("usage").filter(|u| u.is_object()) {
                merge_usage(usage, &mut state.usage);
                state.usage_seen = true;
                events.push(AssistantMessageEvent::Usage {
                    usage: state.usage.clone(),
                });
            }
            completed(interaction, state, model, &mut events);
            return events;
        }
        "error" => {
            let error = &event["error"];
            let message = error["message"]
                .as_str()
                .unwrap_or("upstream error")
                .to_string();
            let kind = classify_error(None, error);
            events.push(error_frame(state, model, kind, message));
            return events;
        }
        _ => {}
    }
    // `interaction.status_update` and `step.stop` may carry usage (never seen live).
    if let Some(usage) = event.get("usage").filter(|u| u.is_object()) {
        merge_usage(usage, &mut state.usage);
        state.usage_seen = true;
        events.push(AssistantMessageEvent::Usage {
            usage: state.usage.clone(),
        });
    }
    events
}

/// Decode one SSE block: the last `data: ` line, with the `event:` line as
/// the fallback when the JSON has no `event_type`. `[DONE]` and comment-only
/// blocks decode to nothing: the terminal frame comes from
/// `interaction.completed`.
pub fn decode_block(
    block: &str,
    state: &mut PartialState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let Some(data) = block
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .next_back()
    else {
        return vec![];
    };
    let Ok(mut parsed) = serde_json::from_str::<Value>(data) else {
        return vec![];
    };
    if parsed.get("event_type").is_none()
        && let Some(name) = block.lines().find_map(|l| l.strip_prefix("event: "))
        && let Some(obj) = parsed.as_object_mut()
    {
        obj.insert("event_type".into(), Value::String(name.trim().to_string()));
    }
    handle_event(&parsed, state, model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MODEL: &str = "gemini-test";

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/sse/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn run_sse(body: &str) -> (PartialState, Vec<AssistantMessageEvent>) {
        let mut state = PartialState::new(vec![]);
        let mut events = Vec::new();
        for block in body.split("\n\n") {
            events.extend(decode_block(block, &mut state, MODEL));
        }
        (state, events)
    }

    fn run(chunks: &[Value]) -> (PartialState, Vec<AssistantMessageEvent>) {
        let mut state = PartialState::new(vec![]);
        let mut events = Vec::new();
        for c in chunks {
            events.extend(handle_event(c, &mut state, MODEL));
        }
        (state, events)
    }

    fn done(events: &[AssistantMessageEvent]) -> &AssistantMessage {
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => message,
            other => panic!("want done, got {other:?}"),
        }
    }

    fn terminal_error(events: &[AssistantMessageEvent]) -> &AssistantMessage {
        match events.last() {
            Some(AssistantMessageEvent::Error { error }) => error,
            other => panic!("want error, got {other:?}"),
        }
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
                AssistantMessageEvent::Stop { .. } => "stop",
                AssistantMessageEvent::Done { .. } => "done",
                AssistantMessageEvent::Error { .. } => "error",
                _ => "other",
            })
            .collect()
    }

    fn calls(m: &AssistantMessage) -> Vec<(&str, &str, &Value)> {
        m.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::FunctionCall {
                    id,
                    function_id,
                    arguments,
                } => Some((id.as_str(), function_id.as_str(), arguments)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_fixture() {
        let (_, events) = run_sse(&fixture("text.sse"));
        let m = done(&events);
        assert_eq!(m.stop_reason, StopReason::End);
        assert_eq!(m.native_stop_reason.as_deref(), Some("completed"));
        assert!(matches!(
            &m.content[..],
            [ContentBlock::Thinking { text, signature: Some(_) }, ContentBlock::Text { .. }] if text.is_empty()
        ));
        let usage = m.usage.as_ref().unwrap();
        assert_eq!(usage.input, Some(4));
        assert_eq!(usage.output, Some(105));
        assert_eq!(usage.reasoning, Some(95));
        assert_eq!(usage.cache_read, Some(0));
        assert_eq!(
            tags(&events),
            [
                "thinking_start",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
                "usage",
                "stop",
                "done"
            ]
        );
    }

    #[test]
    fn thought_summary_and_call_fixture() {
        let (_, events) = run_sse(&fixture("thought_summary_call.sse"));
        let m = done(&events);
        assert_eq!(m.stop_reason, StopReason::FunctionCall);
        let ContentBlock::Thinking { text, signature } = &m.content[0] else {
            panic!("{:?}", m.content)
        };
        assert!(text.contains("**Defining the Lisbon Route**"));
        assert!(text.contains("**Analyzing Route Priorities**"));
        assert!(signature.is_some());
        assert_eq!(
            calls(m),
            [(
                "call_15365121779638325276",
                "get_weather",
                &json!({ "city": "Lisbon" })
            )]
        );
    }

    #[test]
    fn thinking_end_snapshot_carries_the_signature() {
        let (_, events) = run_sse(&fixture("thought_summary_call.sse"));
        let end = events
            .iter()
            .find_map(|e| match e {
                AssistantMessageEvent::ThinkingEnd { partial } => Some(partial),
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            &end.content[0],
            ContentBlock::Thinking {
                signature: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn parallel_calls_fixture() {
        let (_, events) = run_sse(&fixture("parallel_calls.sse"));
        let m = done(&events);
        assert!(matches!(&m.content[0], ContentBlock::Thinking { .. }));
        assert_eq!(
            calls(m),
            [
                (
                    "call_15652254846631736128",
                    "get_weather",
                    &json!({ "city": "Lisbon" })
                ),
                (
                    "call_15652254846631735505",
                    "get_weather",
                    &json!({ "city": "Porto" })
                ),
            ]
        );
    }

    #[test]
    fn call_without_arguments_fixture() {
        let (_, events) = run_sse(&fixture("call_no_args.sse"));
        let m = done(&events);
        let c = calls(m);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].2, &json!({}));
    }

    #[test]
    fn agent_call_nested_fixture() {
        let (_, events) = run_sse(&fixture("agent_call_nested.sse"));
        let m = done(&events);
        let c = calls(m);
        assert_eq!(c[0].1, "agent_call");
        assert_eq!(
            c[0].2["payload"],
            json!({
                "scope": { "env": "staging", "project": "alpha" },
                "values": { "flags": ["fast", "safe"], "retries": 3 }
            })
        );
    }

    #[test]
    fn incomplete_thought_only_fixture() {
        let (_, events) = run_sse(&fixture("incomplete_thought_only.sse"));
        let m = done(&events);
        assert_eq!(m.stop_reason, StopReason::Length);
        assert_eq!(m.native_stop_reason.as_deref(), Some("incomplete"));
        assert_eq!(m.content.len(), 1);
        assert!(matches!(&m.content[0], ContentBlock::Thinking { .. }));
    }

    #[test]
    fn completed_thought_only_fixture_is_end_not_error() {
        let (_, events) = run_sse(&fixture("completed_thought_only.sse"));
        let m = done(&events);
        assert_eq!(m.stop_reason, StopReason::End);
        // A signature-only thought must survive: replay needs the signature.
        assert!(matches!(
            &m.content[..],
            [ContentBlock::Thinking { text, signature: Some(s) }]
                if text.is_empty() && s.starts_with("EsYBCsMB")
        ));
    }

    #[test]
    fn cached_usage_fixture() {
        let (_, events) = run_sse(&fixture("cached_usage.sse"));
        let usage = done(&events).usage.clone().unwrap();
        assert_eq!(usage.cache_read, Some(34_348));
        assert_eq!(usage.input, Some(2_054));
        let thought = usage.reasoning.unwrap();
        assert!(usage.output.unwrap() > thought);
    }

    #[test]
    fn content_inside_step_start() {
        let (_, events) = run(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"thought","summary":[{"type":"text","text":"plan"}],"signature":"sig"}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"step.start","index":1,"step":{"type":"model_output","content":[{"type":"text","text":"hi"}]}}),
            json!({"event_type":"step.stop","index":1}),
            json!({"event_type":"step.start","index":2,"step":{"type":"function_call","id":"c1","name":"shell__exec","arguments":{"cmd":"ls"}}}),
            json!({"event_type":"step.stop","index":2}),
            json!({"event_type":"interaction.completed","interaction":{"status":"requires_action"}}),
        ]);
        let m = done(&events);
        assert_eq!(
            m.content,
            vec![
                ContentBlock::Thinking {
                    text: "plan".into(),
                    signature: Some("sig".into())
                },
                ContentBlock::Text { text: "hi".into() },
                ContentBlock::FunctionCall {
                    id: "c1".into(),
                    function_id: "shell::exec".into(),
                    arguments: json!({"cmd":"ls"})
                },
            ]
        );
        assert_eq!(m.stop_reason, StopReason::FunctionCall);
    }

    #[test]
    fn delta_for_a_closed_call_rides_the_next_snapshot() {
        use llm_router::chat::accumulate::PartialAccumulator;
        let (_, events) = run(&[
            json!({"event_type":"step.start","index":1,"step":{"type":"function_call","id":"a","name":"f","arguments":{}}}),
            json!({"event_type":"step.start","index":2,"step":{"type":"function_call","id":"b","name":"f","arguments":{}}}),
            json!({"event_type":"step.delta","index":1,"delta":{"type":"arguments_delta","arguments":"{\"x\":1}"}}),
            json!({"event_type":"step.delta","index":2,"delta":{"type":"arguments_delta","arguments":"{\"y\":"}}),
        ]);
        assert_eq!(
            tags(&events),
            [
                "functioncall_start",
                "functioncall_end",
                "functioncall_start",
                "functioncall_delta",
            ]
        );
        // The router's in-flight view keeps each call's arguments on its call.
        let mut acc = PartialAccumulator::default();
        for event in &events {
            acc.apply(event);
        }
        let current = acc.current().unwrap();
        let args: Vec<_> = calls(&current).into_iter().map(|c| c.2.clone()).collect();
        assert_eq!(args[0], json!({}), "a's delta is not in a snapshot yet");
        assert_eq!(args[1], json!({"_partial": true}));
        // b's end snapshot carries a's silent delta; the stop for 1 closes nothing.
        let (mut state, _) = run(&[
            json!({"event_type":"step.start","index":1,"step":{"type":"function_call","id":"a","name":"f","arguments":{}}}),
            json!({"event_type":"step.start","index":2,"step":{"type":"function_call","id":"b","name":"f","arguments":{}}}),
            json!({"event_type":"step.delta","index":1,"delta":{"type":"arguments_delta","arguments":"{\"x\":1}"}}),
        ]);
        let stop = |i: u64| json!({"event_type":"step.stop","index":i});
        let end = handle_event(&stop(2), &mut state, MODEL);
        let [AssistantMessageEvent::FunctioncallEnd { partial }] = &end[..] else {
            panic!("{end:?}")
        };
        assert_eq!(calls(partial)[0].2, &json!({"x": 1}));
        assert!(handle_event(&stop(1), &mut state, MODEL).is_empty());
    }

    #[test]
    fn heartbeat_is_an_empty_delta_of_the_open_block() {
        let (mut state, _) =
            run(&[json!({"event_type":"step.start","index":0,"step":{"type":"thought"}})]);
        assert!(matches!(
            state.heartbeat(),
            Some(AssistantMessageEvent::ThinkingDelta { partial: None, delta }) if delta.is_empty()
        ));
        handle_event(
            &json!({"event_type":"step.delta","index":1,"delta":{"type":"text","text":"hi"}}),
            &mut state,
            MODEL,
        );
        assert!(matches!(
            state.heartbeat(),
            Some(AssistantMessageEvent::TextDelta { partial: None, delta }) if delta.is_empty()
        ));
        handle_event(
            &json!({"event_type":"step.stop","index":1}),
            &mut state,
            MODEL,
        );
        assert!(state.heartbeat().is_none());
    }

    #[test]
    fn missing_call_id_gets_a_generated_one() {
        let (_, events) = run(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"function_call","name":"f"}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"interaction.completed","interaction":{"status":"requires_action"}}),
        ]);
        let id = calls(done(&events))[0].0.to_string();
        assert!(id.starts_with("call_") && id.len() > 10, "{id}");
    }

    #[test]
    fn failed_status_is_an_error_with_the_partial() {
        let (_, events) = run(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"model_output"}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"text","text":"so far"}}),
            json!({"event_type":"interaction.completed","interaction":{"status":"failed","errors":[{"code":"safety","message":"blocked"}]}}),
        ]);
        let e = terminal_error(&events);
        assert_eq!(e.error_kind, Some(ErrorKind::Permanent));
        assert_eq!(e.error_message.as_deref(), Some("blocked"));
        assert_eq!(
            e.content,
            vec![ContentBlock::Text {
                text: "so far".into()
            }]
        );

        let (_, events) =
            run(&[json!({"event_type":"interaction.completed","interaction":{"status":"failed"}})]);
        assert_eq!(
            terminal_error(&events).error_kind,
            Some(ErrorKind::Transient)
        );
        let (_, events) = run(&[
            json!({"event_type":"interaction.completed","interaction":{"status":"cancelled"}}),
        ]);
        assert_eq!(
            terminal_error(&events).error_kind,
            Some(ErrorKind::Transient)
        );
    }

    #[test]
    fn mid_stream_error_event_is_terminal() {
        let (_, events) = run(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"model_output"}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"text","text":"x"}}),
            json!({"event_type":"error","error":{"message":"busy","code":"service_unavailable"}}),
        ]);
        let e = terminal_error(&events);
        assert_eq!(e.error_kind, Some(ErrorKind::Transient));
        assert_eq!(e.error_message.as_deref(), Some("busy"));
        assert_eq!(e.content.len(), 1);
        let (_, events) =
            run(&[json!({"event_type":"error","error":{"message":"?","code":"new_code"}})]);
        assert_eq!(
            terminal_error(&events).error_kind,
            Some(ErrorKind::Transient)
        );
    }

    #[test]
    fn other_statuses() {
        let status = |s: &str| {
            let (state, events) =
                run(&[json!({"event_type":"interaction.completed","interaction":{"status":s}})]);
            (done(&events).stop_reason, state.warnings.len())
        };
        assert_eq!(status("requires_action"), (StopReason::End, 1));
        assert_eq!(status("requires_continuation"), (StopReason::Length, 1));
        assert_eq!(status("something_new"), (StopReason::End, 0));
        let (_, events) = run(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"function_call","id":"c1","name":"f","arguments":{}}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"interaction.completed","interaction":{"status":"completed"}}),
        ]);
        assert_eq!(done(&events).stop_reason, StopReason::FunctionCall);
    }

    #[test]
    fn unknown_events_and_deltas_are_skipped() {
        let (state, events) = run(&[
            json!({"event_type":"interaction.created","interaction":{"object":"interaction"}}),
            json!({"event_type":"interaction.status_update","status":"in_progress"}),
            json!({"event_type":"step.start","index":0,"step":{"type":"code_execution"}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"mystery"}}),
            json!({"event_type":"brand.new"}),
        ]);
        assert!(events.is_empty());
        assert!(state.generation_started());
        assert!(build_content(&state).is_empty());
    }

    #[test]
    fn event_line_is_the_fallback_for_event_type() {
        let mut state = PartialState::new(vec![]);
        let events = decode_block(
            "event: step.start\ndata: {\"index\":0,\"step\":{\"type\":\"thought\"}}",
            &mut state,
            MODEL,
        );
        assert_eq!(tags(&events), ["thinking_start"]);
        assert!(decode_block("event: done\ndata: [DONE]", &mut state, MODEL).is_empty());
    }

    /// Contract pin (llm-router types::events): delta frames are slim —
    /// no cumulative partial per chunk — while block-boundary frames carry
    /// the authoritative snapshot.
    #[test]
    fn deltas_are_slim_and_boundary_snapshots_are_cumulative() {
        let (_, events) = run_sse(&fixture("loop_r3.sse"));
        for ev in &events {
            match ev {
                AssistantMessageEvent::TextDelta { partial, .. }
                | AssistantMessageEvent::ThinkingDelta { partial, .. }
                | AssistantMessageEvent::FunctioncallDelta { partial, .. } => {
                    assert!(partial.is_none(), "delta frames must not carry partial")
                }
                _ => {}
            }
        }
        let Some(AssistantMessageEvent::TextEnd { partial }) = events
            .iter()
            .find(|e| matches!(e, AssistantMessageEvent::TextEnd { .. }))
        else {
            panic!("want a text_end frame");
        };
        assert!(matches!(
            partial.content.last(),
            Some(ContentBlock::Text { text }) if text.ends_with("= **970,000**")
        ));
    }

    /// The strongest offline check: four live streams decode into the
    /// assistant messages whose replay is a body the live API accepted.
    #[test]
    fn loop_round_trip_rebuilds_the_accepted_body() {
        use crate::request::{BodyArgs, build_body};
        use llm_router::types::messages::{
            AgentMessage, FunctionResultMessage, FunctionResultRoleTag, UserMessage, UserRoleTag,
        };
        use llm_router::types::model::AgentFunction;

        let user = |t: &str| {
            AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: t.into() }],
                timestamp: 1,
            })
        };
        let mut messages = vec![user(
            "Look up the population of Lisbon, then Porto, then Braga with the tool, \
             strictly one tool call per turn, then tell me the sum.",
        )];
        let results = ["545000", "232000", "193000"];
        for round in 0..4 {
            let (_, events) = run_sse(&fixture(&format!("loop_r{round}.sse")));
            let mut message = done(&events).clone();
            message.model = "gemini-3.8-flash".into();
            messages.push(AgentMessage::Assistant(message.clone()));
            if let Some(result) = results.get(round) {
                let (id, function_id, _) = calls(&message)[0];
                messages.push(AgentMessage::FunctionResult(FunctionResultMessage {
                    role: FunctionResultRoleTag::FunctionResult,
                    function_call_id: id.into(),
                    function_id: function_id.into(),
                    content: vec![ContentBlock::Text {
                        text: result.to_string(),
                    }],
                    details: json!({}),
                    is_error: false,
                    timestamp: 1,
                }));
            }
        }
        messages.push(user("Which city was the largest?"));
        let body = build_body(
            &BodyArgs {
                model: "gemini-3.8-flash".into(),
                max_tokens: 65_536,
                system_prompt: String::new(),
                messages,
                tools: vec![AgentFunction {
                    name: "lookup_population".into(),
                    description: "Look up the population of one city.".into(),
                    parameters: json!({
                        "type": "object",
                        "properties": { "city": { "type": "string", "description": "City name." } },
                        "required": ["city"]
                    }),
                    label: None,
                    execution_mode: None,
                }],
                thinking_level: None,
                thinking_summaries: true,
                response_format: None,
            },
            &mut Vec::new(),
        )
        .unwrap();
        let expected: Value = serde_json::from_str(
            &std::fs::read_to_string(format!(
                "{}/tests/fixtures/requests/loop_replay.json",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(body, expected);
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
        let ev = synthetic_error_event("boom", "gemini-test", ErrorKind::RateLimited);
        match ev {
            AssistantMessageEvent::Error { error } => {
                assert!(
                    error.content.is_empty(),
                    "provider errors are not model output"
                );
                assert_eq!(error.error_message.as_deref(), Some("boom"));
                assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.provider, "gemini");
            }
            other => panic!("want error, got {other:?}"),
        }
    }
}
