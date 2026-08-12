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

/// DeepSeek-only finish reason: the server ran out of capacity part-way
/// through generation (api-docs.deepseek.com, create-chat-completion). The
/// answer is truncated through no fault of the request, so it surfaces as a
/// transient error the router can retry — not a clean stop.
const INSUFFICIENT_RESOURCE: &str = "insufficient_system_resource";

#[derive(Debug, Default)]
struct PartialFunctionCall {
    /// The upstream `tool_calls[].index`, which identifies the call a later
    /// delta belongs to — not this segment's position.
    index: usize,
    id: String,
    function_id: String,
    args_json: String,
}

/// One content block, in arrival order. The message is a *sequence* of these,
/// not one bucket per kind: a model that reasons, answers, reasons again and
/// answers again produces four blocks in that order, and a tool call lands
/// between the blocks it actually fell between. Same shape
/// provider-anthropic assembles via its `block_order`.
#[derive(Debug)]
enum Segment {
    Thinking(String),
    Text(String),
    Call(PartialFunctionCall),
}

pub struct PartialState {
    segments: Vec<Segment>,
    /// Position in `segments` of the block currently open, if any.
    open: Option<usize>,
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
            segments: Vec::new(),
            open: None,
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

    /// True when the open block is a thinking (resp. text) block, so a delta
    /// of that kind extends it instead of starting a new one.
    fn open_is_thinking(&self) -> bool {
        matches!(self.open_segment(), Some(Segment::Thinking(_)))
    }

    fn open_is_text(&self) -> bool {
        matches!(self.open_segment(), Some(Segment::Text(_)))
    }

    fn open_segment(&self) -> Option<&Segment> {
        self.open.map(|i| &self.segments[i])
    }

    /// Append `delta` to the open block, which the caller has just ensured is
    /// of the matching kind.
    fn extend_open(&mut self, delta: &str) {
        match self.open.map(|i| &mut self.segments[i]) {
            Some(Segment::Thinking(s)) | Some(Segment::Text(s)) => s.push_str(delta),
            _ => debug_assert!(false, "extend_open with no open text/thinking block"),
        }
    }

    fn push_segment(&mut self, segment: Segment) -> usize {
        self.segments.push(segment);
        self.segments.len() - 1
    }

    /// Where the call carrying upstream index `index` already lives, if it has
    /// been seen. Deltas for one call can resume after a sibling call started,
    /// so this reopens the original segment rather than appending a duplicate.
    fn call_slot(&self, index: usize) -> Option<usize> {
        self.segments
            .iter()
            .position(|s| matches!(s, Segment::Call(c) if c.index == index))
    }

    fn call_at(&mut self, slot: usize) -> &mut PartialFunctionCall {
        match &mut self.segments[slot] {
            Segment::Call(c) => c,
            _ => unreachable!("slot came from call_slot / a Call push"),
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

/// Segments → content blocks, in arrival order. Empty segments are dropped:
/// a block that opened but never received a delta carries nothing, and a tool
/// call whose name never arrived is not invocable.
fn build_content(state: &PartialState) -> Vec<ContentBlock> {
    state
        .segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Thinking(text) if !text.is_empty() => Some(ContentBlock::Thinking {
                text: text.clone(),
                signature: None,
            }),
            Segment::Text(text) if !text.is_empty() => {
                Some(ContentBlock::Text { text: text.clone() })
            }
            Segment::Call(fc) if !fc.function_id.is_empty() => {
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

pub fn map_finish_reason(s: &str) -> StopReason {
    match s {
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::FunctionCall,
        INSUFFICIENT_RESOURCE => StopReason::Error,
        // stop, content_filter, anything unknown
        _ => StopReason::End,
    }
}

/// Last-wins merge.
///
/// `Usage.input` / `cache_read` / `cache_write` are disjoint prompt-cache
/// *splits* (tech-specs § Usage) and `llm_router::chat::pricing` bills them
/// additively, so `input` carries the tokens charged at the full input rate —
/// the cache MISS slice — not the prompt total. DeepSeek reports the split
/// directly (`prompt_cache_miss_tokens + prompt_cache_hit_tokens =
/// prompt_tokens`), so no arithmetic is needed on the happy path. Feeding the
/// total instead would bill the cached prefix twice, and DeepSeek's cache
/// discount is ~120x (0.435 vs 0.003625 USD/MTok on v4-pro) — on an agent
/// loop that resends a large cached prefix every turn, that roughly doubles
/// the reported cost.
///
/// OpenAI-compatible endpoints behind an `api_url` override report a
/// `prompt_tokens` total that *includes* the cached slice under
/// `prompt_tokens_details.cached_tokens`; there the miss slice is derived.
pub fn merge_usage(raw: &Value, into: &mut Usage) {
    let num = |k: &str| raw.get(k).and_then(Value::as_u64);
    let cached = num("prompt_cache_hit_tokens").or_else(|| {
        ["prompt_tokens_details", "input_tokens_details"]
            .iter()
            .find_map(|parent| {
                raw.pointer(&format!("/{parent}/cached_tokens"))
                    .and_then(Value::as_u64)
            })
    });
    if let Some(v) = cached {
        into.cache_read = Some(v);
    }
    let prompt_total = num("prompt_tokens").or_else(|| num("input_tokens"));
    if let Some(v) = num("prompt_cache_miss_tokens")
        .or_else(|| prompt_total.map(|t| t.saturating_sub(cached.unwrap_or(0))))
    {
        into.input = Some(v);
    }
    if let Some(v) = num("completion_tokens").or_else(|| num("output_tokens")) {
        into.output = Some(v);
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
    let Some(slot) = state.open.take() else {
        return;
    };
    let partial = build_partial(state, model);
    events.push(match &state.segments[slot] {
        Segment::Thinking(_) => AssistantMessageEvent::ThinkingEnd { partial },
        Segment::Text(_) => AssistantMessageEvent::TextEnd { partial },
        Segment::Call(_) => AssistantMessageEvent::FunctioncallEnd { partial },
    });
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
        // Thinking mode streams the chain of thought as `reasoning_content`
        // deltas ahead of the answer `content`. Surface it as a thinking block
        // so the console renders the thoughts instead of a bare "thinking…".
        if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
            if !reasoning.is_empty() {
                // A thinking delta after an answer (or a tool call) opens a
                // NEW thinking block rather than reopening the first one, so
                // the message keeps the order the model produced.
                if !state.open_is_thinking() {
                    close_open_block(state, model, &mut events);
                    state.open = Some(state.push_segment(Segment::Thinking(String::new())));
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: build_partial(state, model),
                    });
                }
                state.extend_open(reasoning);
                events.push(AssistantMessageEvent::ThinkingDelta {
                    partial: None,
                    delta: reasoning.to_string(),
                });
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                if !state.open_is_text() {
                    close_open_block(state, model, &mut events);
                    state.open = Some(state.push_segment(Segment::Text(String::new())));
                    events.push(AssistantMessageEvent::TextStart {
                        partial: build_partial(state, model),
                    });
                }
                state.extend_open(text);
                events.push(AssistantMessageEvent::TextDelta {
                    partial: None,
                    delta: text.to_string(),
                });
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                // Trust boundary: api_url is operator-overridable, so a buggy
                // upstream could send an absurd index. Segments are appended,
                // never indexed by it, so this only bounds how many distinct
                // calls one message can open.
                if index > 128 {
                    continue;
                }
                // An index already seen reopens ITS segment (arguments for one
                // call can resume after a sibling started); a new index appends.
                // `known.is_none()` is load-bearing: an unseen call with no
                // block currently open still has to open one.
                let known = state.call_slot(index);
                if known.is_none() || known != state.open {
                    close_open_block(state, model, &mut events);
                    let slot = known.unwrap_or_else(|| {
                        state.push_segment(Segment::Call(PartialFunctionCall {
                            index,
                            ..PartialFunctionCall::default()
                        }))
                    });
                    state.open = Some(slot);
                    events.push(AssistantMessageEvent::FunctioncallStart {
                        partial: build_partial(state, model),
                    });
                }
                let slot = state.open.expect("a call block is open");
                let entry = state.call_at(slot);
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
                        let entry = state.call_at(slot);
                        entry.args_json.push_str(args);
                        let id = entry.id.clone();
                        events.push(AssistantMessageEvent::FunctioncallDelta {
                            partial: None,
                            delta: args.to_string(),
                            id,
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
                "deepseek filtered the completion (finish_reason: content_filter)".to_string(),
            );
        }
        close_open_block(state, model, &mut events);
        if finish == INSUFFICIENT_RESOURCE {
            // Truncated by upstream capacity, not by the request: emit the
            // terminal error here so the router retries instead of handing a
            // silently-short answer to the caller.
            let msg = "deepseek ran out of system resources mid-generation \
                       (finish_reason: insufficient_system_resource)";
            state.error_message = Some(msg.to_string());
            let mut error = build_final(state, model);
            error.error_kind = Some(ErrorKind::Transient);
            events.push(AssistantMessageEvent::Error { error });
        }
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
            events.extend(handle_chunk(c, &mut state, "deepseek-test"));
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

    /// Contract pin (llm-router types::events): delta frames are slim —
    /// no cumulative partial per chunk — while block-boundary frames carry
    /// the authoritative snapshot. Readers reconstruct via
    /// llm_router::chat::accumulate.
    #[test]
    fn deltas_are_slim_and_boundary_snapshots_are_cumulative() {
        let (_, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"content":"He"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"llo"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
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
            matches!(
                &partial.content[0],
                ContentBlock::Text { text } if text == "Hello"
            ),
            "the End snapshot must carry the cumulative block text"
        );
    }

    #[test]
    fn text_stream_produces_start_delta_end_and_final_content() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"He"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"llo"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":2,
                "prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":8,
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
        let final_msg = build_final(&state, "deepseek-test");
        assert_eq!(
            final_msg.content,
            vec![ContentBlock::Text {
                text: "Hello".into()
            }]
        );
        assert_eq!(final_msg.stop_reason, StopReason::End);
        assert_eq!(final_msg.native_stop_reason.as_deref(), Some("stop"));
        let usage = final_msg.usage.unwrap();
        // prompt_tokens 12 = 8 miss + 4 hit; `input` is the miss slice, so
        // the router's additive cost fill bills each token exactly once.
        assert_eq!(usage.input, Some(8));
        assert_eq!(usage.output, Some(2));
        assert_eq!(usage.cache_read, Some(4));
        assert_eq!(usage.input.unwrap() + usage.cache_read.unwrap(), 12);
        assert_eq!(usage.reasoning, Some(0));
    }

    /// Ordering pin: the final message is a SEQUENCE of blocks in the order
    /// the model produced them, not one merged bucket per kind. A model that
    /// reasons, answers, reasons again and answers again yields four blocks.
    #[test]
    fn interleaved_reasoning_and_answer_keep_their_order_as_separate_blocks() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"first thought"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"partial answer"}}]}),
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"second thought"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"final answer"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ]);
        // every switch closes the previous block and opens a fresh one
        assert_eq!(
            tags(&events),
            vec![
                "thinking_start",
                "thinking_delta",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
                "thinking_start",
                "thinking_delta",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
            ]
        );
        let final_msg = build_final(&state, "deepseek-test");
        assert_eq!(
            final_msg.content,
            vec![
                ContentBlock::Thinking {
                    text: "first thought".into(),
                    signature: None,
                },
                ContentBlock::Text {
                    text: "partial answer".into(),
                },
                ContentBlock::Thinking {
                    text: "second thought".into(),
                    signature: None,
                },
                ContentBlock::Text {
                    text: "final answer".into(),
                },
            ],
            "blocks must not be coalesced per kind"
        );
    }

    /// A tool call lands between the blocks it actually fell between, and
    /// reasoning after it opens a new thinking block rather than merging back
    /// into the first one.
    #[test]
    fn tool_calls_sit_in_arrival_order_between_thinking_and_text() {
        let (state, _) = run(&[
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"need the listing"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"Checking."}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_1","function":{"name":"shell__exec","arguments":"{}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"now answer"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"Done."}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        let kinds: Vec<&str> = build_final(&state, "deepseek-test")
            .content
            .iter()
            .map(|b| match b {
                ContentBlock::Thinking { .. } => "thinking",
                ContentBlock::Text { .. } => "text",
                ContentBlock::FunctionCall { .. } => "call",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            ["thinking", "text", "call", "thinking", "text"],
            "arrival order preserved"
        );
    }

    /// Argument deltas for one call can resume after a sibling call started.
    /// They must append to the call that owns the upstream `index`, not open a
    /// duplicate block or land on the wrong call.
    #[test]
    fn resumed_tool_call_arguments_reopen_the_owning_segment() {
        let (state, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"id":"call_a","function":{"name":"f__a","arguments":"{\"x\":"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":1,"id":"call_b","function":{"name":"f__b","arguments":"{\"y\":2}"}}]}}]}),
            // back to call 0 — must extend call_a, not call_b
            json!({"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"1}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ]);
        let content = build_final(&state, "deepseek-test").content;
        assert_eq!(
            content.len(),
            2,
            "two calls, no duplicate block: {content:?}"
        );
        match (&content[0], &content[1]) {
            (
                ContentBlock::FunctionCall {
                    id: id_a,
                    arguments: args_a,
                    ..
                },
                ContentBlock::FunctionCall {
                    id: id_b,
                    arguments: args_b,
                    ..
                },
            ) => {
                assert_eq!(id_a, "call_a");
                assert_eq!(args_a["x"], 1, "resumed delta landed on its own call");
                assert_eq!(id_b, "call_b");
                assert_eq!(args_b["y"], 2);
            }
            other => panic!("want two function calls, got {other:?}"),
        }
        // reopening call 0 emits a fresh start for it
        assert_eq!(
            tags(&events)
                .iter()
                .filter(|t| **t == "functioncall_start")
                .count(),
            3,
            "call_a, call_b, then call_a reopened"
        );
    }

    /// The cost-correctness pin: `input` and `cache_read` are disjoint splits
    /// of the prompt, so the router's additive `fill_cost_usd` bills every
    /// prompt token exactly once — at the input rate or the ~120x-cheaper
    /// cache rate, never both.
    #[test]
    fn input_is_the_cache_miss_slice_so_cached_tokens_are_billed_once() {
        // DeepSeek-native: the split is reported directly.
        let mut usage = Usage::default();
        merge_usage(
            &json!({"prompt_tokens":100,"completion_tokens":5,
                "prompt_cache_hit_tokens":64,"prompt_cache_miss_tokens":36}),
            &mut usage,
        );
        assert_eq!(usage.input, Some(36), "input is the miss slice");
        assert_eq!(usage.cache_read, Some(64));
        assert_eq!(
            usage.input.unwrap() + usage.cache_read.unwrap(),
            100,
            "the splits must sum to prompt_tokens"
        );

        // OpenAI-compatible endpoint behind an api_url override: prompt_tokens
        // INCLUDES the cached slice, so the miss slice is derived.
        let mut usage = Usage::default();
        merge_usage(
            &json!({"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":40}}),
            &mut usage,
        );
        assert_eq!(usage.cache_read, Some(40));
        assert_eq!(usage.input, Some(60));

        // No cache reported at all: the whole prompt bills at the input rate.
        let mut usage = Usage::default();
        merge_usage(
            &json!({"prompt_tokens":100,"completion_tokens":5}),
            &mut usage,
        );
        assert_eq!(usage.input, Some(100));
        assert_eq!(usage.cache_read, None);

        // Both spellings present: DeepSeek's own fields win.
        let mut usage = Usage::default();
        merge_usage(
            &json!({"prompt_tokens":20,"prompt_cache_hit_tokens":7,"prompt_cache_miss_tokens":13,
                "prompt_tokens_details":{"cached_tokens":9}}),
            &mut usage,
        );
        assert_eq!(usage.cache_read, Some(7));
        assert_eq!(usage.input, Some(13));

        // A cached slice larger than the reported total cannot underflow.
        let mut usage = Usage::default();
        merge_usage(
            &json!({"prompt_tokens":5,"prompt_tokens_details":{"cached_tokens":9}}),
            &mut usage,
        );
        assert_eq!(usage.input, Some(0));
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
        let final_msg = build_final(&state, "deepseek-test");
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
        let final_msg = build_final(&state, "deepseek-test");
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
        let final_msg = build_final(&state, "deepseek-test");
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
        let final_msg = build_final(&state, "deepseek-test");
        assert_eq!(final_msg.content.len(), 2);
    }

    #[test]
    fn content_filter_maps_to_end_with_warning() {
        let (state, _) = run(&[
            json!({"choices":[{"index":0,"delta":{"content":"par"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}]}),
        ]);
        let final_msg = build_final(&state, "deepseek-test");
        assert_eq!(final_msg.stop_reason, StopReason::End);
        assert_eq!(
            final_msg.native_stop_reason.as_deref(),
            Some("content_filter")
        );
        assert!(final_msg.warnings.unwrap()[0].contains("content_filter"));
    }

    #[test]
    fn insufficient_system_resource_is_a_retryable_terminal_error() {
        // Upstream capacity truncated the answer: the router must retry, not
        // hand a silently-short completion to the caller.
        let (_, events) = run(&[
            json!({"choices":[{"index":0,"delta":{"content":"par"}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"insufficient_system_resource"}]}),
        ]);
        assert_eq!(
            tags(&events),
            vec!["text_start", "text_delta", "text_end", "error"]
        );
        let last = events.last().unwrap();
        assert!(last.is_terminal());
        match last {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
                assert!(error.error_kind.unwrap().is_retryable());
                assert_eq!(
                    error.native_stop_reason.as_deref(),
                    Some("insufficient_system_resource")
                );
                // the partial answer rides along
                assert!(matches!(&error.content[0], ContentBlock::Text { text } if text == "par"));
            }
            other => panic!("want error frame, got {other:?}"),
        }
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
        let ev = synthetic_error_event("boom", "deepseek-test", ErrorKind::RateLimited);
        match ev {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
                assert_eq!(error.stop_reason, StopReason::Error);
                assert_eq!(error.provider, "deepseek");
            }
            other => panic!("want error, got {other:?}"),
        }
    }
}
