use crate::errors::classify;
use crate::request::WireDialect;
use crate::{now_ms, PROVIDER_ID};
use llm_router::provider_scaffold::names::decode_tool_name;
use llm_router::provider_scaffold::sse_transport::{arguments_incomplete, StreamEndView};
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
use serde_json::Value;

fn data_line(block: &str) -> Option<&str> {
    block
        .lines()
        .filter_map(|line| {
            let data = line.trim_end_matches('\r').strip_prefix("data:")?;
            Some(data.strip_prefix(' ').unwrap_or(data))
        })
        .next_back()
}

fn message(
    model: &str,
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    warnings: &[String],
    usage: Option<Usage>,
) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content,
        stop_reason,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: (!warnings.is_empty()).then(|| warnings.to_vec()),
        usage,
        model: model.to_string(),
        provider: PROVIDER_ID.to_string(),
        timestamp: now_ms(),
    }
}

pub fn synthetic_error(
    model: &str,
    error_message: impl Into<String>,
    kind: ErrorKind,
) -> AssistantMessageEvent {
    let error_message = error_message.into();
    let mut error = message(
        model,
        vec![ContentBlock::Text {
            text: error_message.clone(),
        }],
        StopReason::Error,
        &[],
        None,
    );
    error.error_message = Some(error_message);
    error.error_kind = Some(kind);
    AssistantMessageEvent::Error { error }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatOpen {
    Thinking,
    Text,
    Call(usize),
}

#[derive(Debug, Default)]
struct ChatCall {
    id: String,
    function_id: String,
    arguments: String,
}

pub struct ChatState {
    thinking: String,
    text: String,
    calls: Vec<ChatCall>,
    open: Option<ChatOpen>,
    usage: Usage,
    usage_seen: bool,
    stop_reason: StopReason,
    native_stop_reason: Option<String>,
    warnings: Vec<String>,
    saw_finish_reason: bool,
}

impl ChatState {
    fn new(warnings: Vec<String>) -> Self {
        Self {
            thinking: String::new(),
            text: String::new(),
            calls: Vec::new(),
            open: None,
            usage: Usage::default(),
            usage_seen: false,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            warnings,
            saw_finish_reason: false,
        }
    }

    fn content(&self) -> Vec<ContentBlock> {
        let mut content = Vec::new();
        if !self.thinking.is_empty() {
            content.push(ContentBlock::Thinking {
                text: self.thinking.clone(),
                signature: None,
            });
        }
        if !self.text.is_empty() {
            content.push(ContentBlock::Text {
                text: self.text.clone(),
            });
        }
        content.extend(self.calls.iter().filter_map(|call| {
            if call.function_id.is_empty() {
                return None;
            }
            let arguments = if call.arguments.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&call.arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| {
                        llm_router::types::messages::degraded_arguments(&call.arguments)
                    })
            };
            Some(ContentBlock::FunctionCall {
                id: call.id.clone(),
                function_id: call.function_id.clone(),
                arguments,
            })
        }));
        content
    }

    fn partial(&self, model: &str) -> AssistantMessage {
        let mut message = message(
            model,
            self.content(),
            self.stop_reason,
            &self.warnings,
            self.usage_seen.then(|| self.usage.clone()),
        );
        message.native_stop_reason = self.native_stop_reason.clone();
        message
    }

    fn close_open(&mut self, model: &str, events: &mut Vec<AssistantMessageEvent>) {
        match self.open.take() {
            Some(ChatOpen::Thinking) => events.push(AssistantMessageEvent::ThinkingEnd {
                partial: self.partial(model),
            }),
            Some(ChatOpen::Text) => events.push(AssistantMessageEvent::TextEnd {
                partial: self.partial(model),
            }),
            Some(ChatOpen::Call(_)) => events.push(AssistantMessageEvent::FunctioncallEnd {
                partial: self.partial(model),
            }),
            None => {}
        }
    }
}

fn merge_chat_usage(raw: &Value, usage: &mut Usage) -> bool {
    let number = |key: &str| raw.get(key).and_then(Value::as_u64);
    let mut reported = false;
    let cached = ["prompt_tokens_details", "input_tokens_details"]
        .iter()
        .filter_map(|parent| {
            raw.pointer(&format!("/{parent}/cached_tokens"))
                .and_then(Value::as_u64)
        })
        .next_back();
    if let Some(value) = cached {
        usage.cache_read = Some(value);
        reported = true;
    }
    if let Some(value) = number("prompt_tokens").or_else(|| number("input_tokens")) {
        usage.input = Some(value.saturating_sub(cached.unwrap_or(0)));
        reported = true;
    }
    if let Some(value) = number("completion_tokens").or_else(|| number("output_tokens")) {
        usage.output = Some(value);
        reported = true;
    }
    if let Some(value) = raw
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
    {
        usage.reasoning = Some(value);
        reported = true;
    }
    reported
}

fn chat_finish_reason(reason: &str) -> StopReason {
    match reason {
        "length" => StopReason::Length,
        "tool_calls" | "function_call" => StopReason::FunctionCall,
        _ => StopReason::End,
    }
}

fn handle_chat_block(
    block: &str,
    state: &mut ChatState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let Some(data) = data_line(block) else {
        return Vec::new();
    };
    if data == "[DONE]" {
        let mut events = Vec::new();
        state.close_open(model, &mut events);
        events.push(AssistantMessageEvent::Stop {
            stop_reason: state.stop_reason,
            error_message: None,
            error_kind: None,
        });
        events.push(AssistantMessageEvent::Done {
            message: state.partial(model),
        });
        return events;
    }
    let Ok(chunk) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    if let Some(error) = chunk.get("error") {
        let error_message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("upstream error")
            .to_string();
        let mut final_message = state.partial(model);
        final_message.stop_reason = StopReason::Error;
        final_message.error_message = Some(error_message);
        final_message.error_kind = Some(classify(None, data));
        return vec![AssistantMessageEvent::Error {
            error: final_message,
        }];
    }

    let mut events = Vec::new();
    if let Some(usage) = chunk.get("usage").filter(|usage| usage.is_object()) {
        if merge_chat_usage(usage, &mut state.usage) {
            state.usage_seen = true;
            events.push(AssistantMessageEvent::Usage {
                usage: state.usage.clone(),
            });
        }
    }
    let Some(choice) = chunk.pointer("/choices/0") else {
        return events;
    };
    if let Some(delta) = choice.get("delta") {
        let reasoning = delta
            .get("reasoning")
            .or_else(|| delta.get("reasoning_content"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !reasoning.is_empty() {
            if state.open != Some(ChatOpen::Thinking) {
                state.close_open(model, &mut events);
                state.open = Some(ChatOpen::Thinking);
                events.push(AssistantMessageEvent::ThinkingStart {
                    partial: state.partial(model),
                });
            }
            state.thinking.push_str(reasoning);
            events.push(AssistantMessageEvent::ThinkingDelta {
                partial: None,
                delta: reasoning.to_string(),
            });
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                if state.open != Some(ChatOpen::Text) {
                    state.close_open(model, &mut events);
                    state.open = Some(ChatOpen::Text);
                    events.push(AssistantMessageEvent::TextStart {
                        partial: state.partial(model),
                    });
                }
                state.text.push_str(text);
                events.push(AssistantMessageEvent::TextDelta {
                    partial: None,
                    delta: text.to_string(),
                });
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if index >= 256 {
                    continue;
                }
                while state.calls.len() <= index {
                    state.calls.push(ChatCall::default());
                }
                if state.open != Some(ChatOpen::Call(index)) {
                    state.close_open(model, &mut events);
                    state.open = Some(ChatOpen::Call(index));
                    events.push(AssistantMessageEvent::FunctioncallStart {
                        partial: state.partial(model),
                    });
                }
                let current = &mut state.calls[index];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    current.id = id.to_string();
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    current.function_id = decode_tool_name(name);
                }
                if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str)
                {
                    current.arguments.push_str(arguments);
                    events.push(AssistantMessageEvent::FunctioncallDelta {
                        partial: None,
                        delta: arguments.to_string(),
                        id: current.id.clone(),
                        arguments_preview: None,
                    });
                }
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        state.stop_reason = chat_finish_reason(reason);
        state.native_stop_reason = Some(reason.to_string());
        state.saw_finish_reason = true;
        state.close_open(model, &mut events);
    }
    events
}

enum AnthropicBlock {
    Empty,
    Text(String),
    Thinking {
        text: String,
        signature: Option<String>,
    },
    Redacted(String),
    Tool {
        id: String,
        function_id: String,
        arguments: String,
    },
    Unknown,
}

pub struct AnthropicState {
    blocks: Vec<AnthropicBlock>,
    usage: Usage,
    usage_seen: bool,
    stop_reason: StopReason,
    native_stop_reason: Option<String>,
    warnings: Vec<String>,
}

impl AnthropicState {
    fn new(warnings: Vec<String>) -> Self {
        Self {
            blocks: Vec::new(),
            usage: Usage::default(),
            usage_seen: false,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            warnings,
        }
    }

    fn ensure(&mut self, index: usize) {
        while self.blocks.len() <= index {
            self.blocks.push(AnthropicBlock::Empty);
        }
    }

    fn content(&self) -> Vec<ContentBlock> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                AnthropicBlock::Text(text) if !text.is_empty() => {
                    Some(ContentBlock::Text { text: text.clone() })
                }
                AnthropicBlock::Thinking { text, signature }
                    if !text.is_empty() || signature.is_some() =>
                {
                    Some(ContentBlock::Thinking {
                        text: text.clone(),
                        signature: signature.clone(),
                    })
                }
                AnthropicBlock::Redacted(data) if !data.is_empty() => {
                    Some(ContentBlock::RedactedThinking { data: data.clone() })
                }
                AnthropicBlock::Tool {
                    id,
                    function_id,
                    arguments,
                } if !function_id.is_empty() => Some(ContentBlock::FunctionCall {
                    id: id.clone(),
                    function_id: function_id.clone(),
                    arguments: if arguments.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(arguments)
                            .ok()
                            .filter(Value::is_object)
                            .unwrap_or_else(|| {
                                llm_router::types::messages::degraded_arguments(arguments)
                            })
                    },
                }),
                _ => None,
            })
            .collect()
    }

    fn partial(&self, model: &str) -> AssistantMessage {
        let mut message = message(
            model,
            self.content(),
            self.stop_reason,
            &self.warnings,
            self.usage_seen.then(|| self.usage.clone()),
        );
        message.native_stop_reason = self.native_stop_reason.clone();
        message
    }
}

fn merge_anthropic_usage(raw: &Value, usage: &mut Usage) -> bool {
    let number = |key: &str| raw.get(key).and_then(Value::as_u64);
    let mut reported = false;
    if let Some(value) = number("input_tokens") {
        usage.input = Some(value);
        reported = true;
    }
    if let Some(value) = number("output_tokens") {
        usage.output = Some(value);
        reported = true;
    }
    if let Some(value) = number("cache_read_input_tokens") {
        usage.cache_read = Some(value);
        reported = true;
    }
    if let Some(value) = number("cache_creation_input_tokens") {
        usage.cache_write = Some(value);
        reported = true;
    }
    reported
}

fn anthropic_stop_reason(reason: &str) -> StopReason {
    match reason {
        "max_tokens" => StopReason::Length,
        "tool_use" => StopReason::FunctionCall,
        _ => StopReason::End,
    }
}

fn handle_anthropic_block(
    block: &str,
    state: &mut AnthropicState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let Some(data) = data_line(block) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let event_type = parsed
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut events = Vec::new();
    match event_type {
        "message_start" => {
            if let Some(usage) = parsed.pointer("/message/usage") {
                if merge_anthropic_usage(usage, &mut state.usage) {
                    state.usage_seen = true;
                    events.push(AssistantMessageEvent::Usage {
                        usage: state.usage.clone(),
                    });
                }
            }
        }
        "content_block_start" => {
            let Some(index) = parsed
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i as usize)
            else {
                return events;
            };
            if index >= 256 {
                return events;
            }
            state.ensure(index);
            let content_type = parsed
                .pointer("/content_block/type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            state.blocks[index] = match content_type {
                "text" => {
                    events.push(AssistantMessageEvent::TextStart {
                        partial: state.partial(model),
                    });
                    AnthropicBlock::Text(String::new())
                }
                "thinking" => {
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: state.partial(model),
                    });
                    AnthropicBlock::Thinking {
                        text: String::new(),
                        signature: None,
                    }
                }
                "redacted_thinking" => {
                    events.push(AssistantMessageEvent::ThinkingStart {
                        partial: state.partial(model),
                    });
                    AnthropicBlock::Redacted(
                        parsed
                            .pointer("/content_block/data")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    )
                }
                "tool_use" => {
                    events.push(AssistantMessageEvent::FunctioncallStart {
                        partial: state.partial(model),
                    });
                    AnthropicBlock::Tool {
                        id: parsed
                            .pointer("/content_block/id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        function_id: decode_tool_name(
                            parsed
                                .pointer("/content_block/name")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                        arguments: String::new(),
                    }
                }
                _ => AnthropicBlock::Unknown,
            };
        }
        "content_block_delta" => {
            let Some(index) = parsed
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i as usize)
            else {
                return events;
            };
            let delta_type = parsed
                .pointer("/delta/type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match state.blocks.get_mut(index) {
                Some(AnthropicBlock::Text(text)) if delta_type == "text_delta" => {
                    let delta = parsed
                        .pointer("/delta/text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    text.push_str(delta);
                    events.push(AssistantMessageEvent::TextDelta {
                        partial: None,
                        delta: delta.to_string(),
                    });
                }
                Some(AnthropicBlock::Thinking { text, .. }) if delta_type == "thinking_delta" => {
                    let delta = parsed
                        .pointer("/delta/thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    text.push_str(delta);
                    events.push(AssistantMessageEvent::ThinkingDelta {
                        partial: None,
                        delta: delta.to_string(),
                    });
                }
                Some(AnthropicBlock::Thinking { signature, .. })
                    if delta_type == "signature_delta" =>
                {
                    let delta = parsed
                        .pointer("/delta/signature")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    signature.get_or_insert_with(String::new).push_str(delta);
                }
                Some(AnthropicBlock::Tool { id, arguments, .. })
                    if delta_type == "input_json_delta" =>
                {
                    let delta = parsed
                        .pointer("/delta/partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    arguments.push_str(delta);
                    events.push(AssistantMessageEvent::FunctioncallDelta {
                        partial: None,
                        delta: delta.to_string(),
                        id: id.clone(),
                        arguments_preview: None,
                    });
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let Some(index) = parsed
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i as usize)
            else {
                return events;
            };
            match state.blocks.get(index) {
                Some(AnthropicBlock::Text(_)) => events.push(AssistantMessageEvent::TextEnd {
                    partial: state.partial(model),
                }),
                Some(AnthropicBlock::Thinking { .. }) | Some(AnthropicBlock::Redacted(_)) => events
                    .push(AssistantMessageEvent::ThinkingEnd {
                        partial: state.partial(model),
                    }),
                Some(AnthropicBlock::Tool { .. }) => {
                    events.push(AssistantMessageEvent::FunctioncallEnd {
                        partial: state.partial(model),
                    })
                }
                _ => {}
            }
        }
        "message_delta" => {
            if let Some(reason) = parsed.pointer("/delta/stop_reason").and_then(Value::as_str) {
                state.stop_reason = anthropic_stop_reason(reason);
                state.native_stop_reason = Some(reason.to_string());
            }
            if let Some(usage) = parsed.get("usage") {
                if merge_anthropic_usage(usage, &mut state.usage) {
                    state.usage_seen = true;
                    events.push(AssistantMessageEvent::Usage {
                        usage: state.usage.clone(),
                    });
                }
            }
        }
        "message_stop" => {
            events.push(AssistantMessageEvent::Stop {
                stop_reason: state.stop_reason,
                error_message: None,
                error_kind: None,
            });
            events.push(AssistantMessageEvent::Done {
                message: state.partial(model),
            });
        }
        "error" => {
            let error_message = parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("upstream error")
                .to_string();
            let mut final_message = state.partial(model);
            final_message.stop_reason = StopReason::Error;
            final_message.error_message = Some(error_message);
            final_message.error_kind = Some(classify(None, data));
            events.push(AssistantMessageEvent::Error {
                error: final_message,
            });
        }
        _ => {}
    }
    events
}

pub enum DecoderState {
    Chat(ChatState),
    Anthropic(AnthropicState),
}

impl DecoderState {
    pub fn new(dialect: WireDialect, warnings: Vec<String>) -> Self {
        match dialect {
            WireDialect::ChatCompletions => Self::Chat(ChatState::new(warnings)),
            WireDialect::AnthropicMessages => Self::Anthropic(AnthropicState::new(warnings)),
        }
    }

    pub fn partial(&self, model: &str) -> AssistantMessage {
        match self {
            Self::Chat(state) => state.partial(model),
            Self::Anthropic(state) => state.partial(model),
        }
    }

    pub fn handle_block(&mut self, block: &str, model: &str) -> Vec<AssistantMessageEvent> {
        match self {
            Self::Chat(state) => handle_chat_block(block, state, model),
            Self::Anthropic(state) => handle_anthropic_block(block, state, model),
        }
    }

    pub fn error_event(
        &self,
        model: &str,
        error_message: impl Into<String>,
        kind: ErrorKind,
    ) -> AssistantMessageEvent {
        let mut error = self.partial(model);
        error.stop_reason = StopReason::Error;
        error.error_message = Some(error_message.into());
        error.error_kind = Some(kind);
        AssistantMessageEvent::Error { error }
    }
}

impl StreamEndView for DecoderState {
    fn saw_terminator(&self) -> bool {
        matches!(self, Self::Chat(state) if state.saw_finish_reason)
    }

    fn has_content(&self) -> bool {
        match self {
            Self::Chat(state) => {
                !state.thinking.is_empty()
                    || !state.text.is_empty()
                    || state.calls.iter().any(|call| !call.function_id.is_empty())
            }
            Self::Anthropic(state) => !state.content().is_empty(),
        }
    }

    fn has_unfinished_call(&self) -> bool {
        match self {
            Self::Chat(state) => {
                matches!(state.open, Some(ChatOpen::Call(_)))
                    || state
                        .calls
                        .iter()
                        .any(|call| arguments_incomplete(&call.arguments))
            }
            Self::Anthropic(state) => state.blocks.iter().any(|block| {
                matches!(
                    block,
                    AnthropicBlock::Tool { arguments, .. } if arguments_incomplete(arguments)
                )
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_usage_stays_absent_for_both_dialects() {
        for dialect in [WireDialect::ChatCompletions, WireDialect::AnthropicMessages] {
            assert!(DecoderState::new(dialect, vec![])
                .partial("m")
                .usage
                .is_none());
        }
    }

    #[test]
    fn empty_usage_objects_stay_absent_for_both_dialects() {
        let mut chat = DecoderState::new(WireDialect::ChatCompletions, vec![]);
        assert!(chat
            .handle_block("data: {\"choices\":[],\"usage\":{}}\n\n", "m")
            .is_empty());
        assert!(chat.partial("m").usage.is_none());

        let mut anthropic = DecoderState::new(WireDialect::AnthropicMessages, vec![]);
        assert!(anthropic
            .handle_block(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\n",
                "m",
            )
            .is_empty());
        assert!(anthropic.partial("m").usage.is_none());
    }

    #[test]
    fn chat_usage_maps_only_values_the_upstream_returns() {
        let mut state = DecoderState::new(WireDialect::ChatCompletions, vec![]);
        let events = state.handle_block(
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            "m",
        );
        assert!(matches!(events[0], AssistantMessageEvent::Usage { .. }));
        let usage = state.partial("m").usage.unwrap();
        assert_eq!(usage.input, Some(12));
        assert_eq!(usage.output, Some(3));
        assert_eq!(usage.cost_usd, None);
        assert_eq!(usage.cache_read, None);
    }

    #[test]
    fn chat_cached_tokens_are_not_double_counted_as_input() {
        let mut state = DecoderState::new(WireDialect::ChatCompletions, vec![]);
        state.handle_block(
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":5}}}\n\n",
            "m",
        );
        let usage = state.partial("m").usage.unwrap();
        assert_eq!(usage.input, Some(7));
        assert_eq!(usage.cache_read, Some(5));
        assert_eq!(usage.output, Some(3));
    }
}
