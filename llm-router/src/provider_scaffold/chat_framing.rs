//! How an assembled chat request is walked for counting, independent of which
//! tokenizer does the counting.
//!
//! Every local counter — tiktoken for the OpenAI families, a model's own
//! vocabulary for everything else — has to agree on the same three things:
//! which text a message contributes to the wire, what the chat framing costs
//! on top of that text, and how a tool schema is serialized. Only the
//! encoding differs. Keeping the walk here means a new tokenizer is one
//! closure rather than a second copy of these rules, and a fix to the walk
//! reaches every counter at once.

use serde_json::json;

use crate::provider_scaffold::names::encode_tool_name;
use crate::types::content::ContentBlock;
use crate::types::messages::AgentMessage;
use crate::types::model::AgentFunction;

/// Chat-framing overhead per wire message row (role tag + separators),
/// OpenAI's published ~4-tokens-per-message heuristic for ChatML-framed
/// conversations. The system prompt is one such row.
pub const TOKENS_PER_MESSAGE: u64 = 4;

/// Reply priming the API appends to every prompt (the assistant start tag).
pub const TOKENS_REPLY_PRIMING: u64 = 2;

/// The text a message contributes to the wire: text blocks, plus each
/// function call's encoded name and serialized arguments, plus function
/// result bodies. Thinking blocks are dropped (never replayed on this wire),
/// images are skipped (their token cost is model-specific, not textual), and
/// custom messages never reach the provider.
pub fn message_text(message: &AgentMessage) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let content = match message {
        AgentMessage::User(m) => &m.content,
        AgentMessage::Assistant(m) => &m.content,
        AgentMessage::FunctionResult(m) => &m.content,
        AgentMessage::Custom(_) => return None,
    };
    for block in content {
        match block {
            ContentBlock::Text { text } => parts.push(text.clone()),
            ContentBlock::FunctionCall {
                function_id,
                arguments,
                ..
            } => {
                parts.push(encode_tool_name(function_id));
                parts.push(arguments.to_string());
            }
            _ => {}
        }
    }
    Some(parts.join("\n"))
}

/// The serialized form of one tool schema, as it reaches the wire.
pub fn tool_schema_text(tool: &AgentFunction) -> String {
    json!({
        "name": encode_tool_name(&tool.name),
        "description": tool.description,
        "parameters": tool.parameters,
    })
    .to_string()
}

/// Count an assembled chat request with `count_text` as the encoder: reply
/// priming, plus one framed row per system prompt and message, plus each
/// tool's serialized schema. The framing constants are the caller-independent
/// part; `count_text` is the only thing a tokenizer changes.
pub fn count_framed_chat(
    system_prompt: Option<&str>,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
    count_text: impl Fn(&str) -> u64,
) -> u64 {
    let mut tokens = TOKENS_REPLY_PRIMING;
    if let Some(system) = system_prompt.filter(|s| !s.is_empty()) {
        tokens += TOKENS_PER_MESSAGE + count_text(system);
    }
    for message in messages {
        if let Some(text) = message_text(message) {
            tokens += TOKENS_PER_MESSAGE + count_text(&text);
        }
    }
    for tool in tools {
        tokens += count_text(&tool_schema_text(tool));
    }
    tokens
}
