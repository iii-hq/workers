//! `provider::openai-codex::count_tokens` — local prompt token estimation
//! with the tiktoken tokenizers (vocabularies embedded in the binary). The
//! Codex backend exposes no metering endpoint, so the count is computed from
//! the text the wire mappers would send plus the published chat-framing
//! constants; it never runs the model, costs nothing, and needs no network.

use iii_sdk::errors::Error;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, CoreBPE};

use crate::wire::names::encode_tool_name;

/// The count is a local tokenizer estimate, not a provider-metered value.
const ESTIMATOR_TIKTOKEN: &str = "tiktoken";

/// Chat-framing overhead per wire message row (role tag + separators),
/// OpenAI's published ~4-tokens-per-message heuristic for ChatML-framed
/// conversations. The system prompt is one such row.
const TOKENS_PER_MESSAGE: u64 = 4;

/// Reply priming the API appends to every prompt (the assistant start tag).
const TOKENS_REPLY_PRIMING: u64 = 2;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; selects the tokenizer (o200k_base for
    /// gpt-4o/gpt-5/o-series and anything unknown-modern, cl100k_base for
    /// gpt-3.5 and non-o gpt-4 families).
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward
    /// the total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::openai-codex::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Estimated prompt tokens for the assembled request.
    pub tokens: u64,
    /// Always `tiktoken`: a local tokenizer produced the estimate.
    pub estimator: String,
}

/// Tokenizer for a model id. This catalog namespaces ids (`codex/gpt-5.2`),
/// so selection runs on the bare model id after the namespace. cl100k_base
/// covers the gpt-3.5 and non-o gpt-4 generations (`gpt-4`, `gpt-4-turbo`,
/// …); everything else — gpt-4o, gpt-4.1, gpt-5, the o-series, and
/// unknown-modern ids — is o200k_base.
fn encoder_for(model: &str) -> &'static CoreBPE {
    let bare = model.rsplit('/').next().unwrap_or(model);
    if wants_cl100k(bare) {
        cl100k_base_singleton()
    } else {
        o200k_base_singleton()
    }
}

fn wants_cl100k(bare: &str) -> bool {
    if bare.starts_with("gpt-3.5") {
        return true;
    }
    match bare.strip_prefix("gpt-4") {
        Some(rest) => rest.is_empty() || rest.starts_with('-'),
        None => false,
    }
}

/// The text a message contributes to the wire: text blocks, plus each
/// function call's encoded name and serialized arguments, plus function
/// result bodies. Thinking blocks are dropped (never replayed on this wire),
/// images are skipped (their token cost is model-specific, not textual), and
/// custom messages never reach the provider.
fn message_text(message: &AgentMessage) -> Option<String> {
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

fn count(bpe: &CoreBPE, text: &str) -> u64 {
    bpe.encode_ordinary(text).len() as u64
}

pub fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let bpe = encoder_for(&req.model);
    let mut tokens = TOKENS_REPLY_PRIMING;
    if let Some(system) = req.system_prompt.as_deref().filter(|s| !s.is_empty()) {
        tokens += TOKENS_PER_MESSAGE + count(bpe, system);
    }
    for message in &req.messages {
        if let Some(text) = message_text(message) {
            tokens += TOKENS_PER_MESSAGE + count(bpe, &text);
        }
    }
    for tool in req.tools.as_deref().unwrap_or(&[]) {
        let schema = json!({
            "name": encode_tool_name(&tool.name),
            "description": tool.description,
            "parameters": tool.parameters,
        });
        tokens += count(bpe, &schema.to_string());
    }
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TIKTOKEN.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    fn request(model: &str, messages: Vec<AgentMessage>) -> CountTokensRequest {
        CountTokensRequest {
            model: model.into(),
            system_prompt: None,
            tools: None,
            messages,
        }
    }

    #[test]
    fn encoder_selection_strips_the_codex_namespace() {
        assert!(wants_cl100k("gpt-4-turbo"));
        assert!(!wants_cl100k("gpt-4o"));
        assert!(!wants_cl100k("gpt-5.2"));
        // Namespaced ids select on the bare model id.
        let namespaced = handle(request("codex/gpt-5.2", vec![user("hi")])).unwrap();
        let bare = handle(request("gpt-5.2", vec![user("hi")])).unwrap();
        assert_eq!(namespaced.tokens, bare.tokens);
        assert_eq!(namespaced.model, "codex/gpt-5.2");
    }

    #[test]
    fn empty_messages_are_rejected() {
        assert!(handle(request("codex/gpt-5.2", vec![])).is_err());
    }

    #[test]
    fn framing_constants_apply_per_message_plus_priming() {
        let bpe = o200k_base_singleton();
        let hello = count(bpe, "hello world");
        let resp = handle(request("codex/gpt-5.2", vec![user("hello world")])).unwrap();
        assert_eq!(
            resp.tokens,
            TOKENS_REPLY_PRIMING + TOKENS_PER_MESSAGE + hello
        );
        assert_eq!(resp.estimator, "tiktoken");
    }

    #[test]
    fn system_prompt_and_tools_count_toward_the_total() {
        let base = handle(request("codex/gpt-5.2", vec![user("hi")]))
            .unwrap()
            .tokens;
        let mut req = request("codex/gpt-5.2", vec![user("hi")]);
        req.system_prompt = Some("be brief".into());
        req.tools = Some(vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }]);
        assert!(handle(req).unwrap().tokens > base);
    }
}
