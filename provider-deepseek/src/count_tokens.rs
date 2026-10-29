//! `provider::deepseek::count_tokens` — prompt token counting with DeepSeek's
//! own vocabulary.
//!
//! DeepSeek publishes no metering endpoint, so the count is computed locally.
//! It is computed with DeepSeek's published tokenizer rather than a borrowed
//! one: the OpenAI-compatible wire shape does not imply an OpenAI vocabulary,
//! and counting these models with tiktoken would produce a number that looks
//! authoritative while being wrong by whatever the two vocabularies disagree
//! about. The vocabulary is fetched once and cached on disk, so a model
//! DeepSeek ships tomorrow counts correctly today.
//!
//! What gets counted is the prompt DeepSeek builds from this provider's wire
//! rows, not the rows' text: the tool block, the DSML each call becomes and
//! the replayed `reasoning_content` are billed as prompt tokens too. Counting
//! the text alone left `context_pressure` ~7% under its bill. The turn's
//! thinking level picks the encoder's mode: chat mode bills no effort line
//! and no reasoning replay.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::deepseek_v41::{
    effort_budget, prompt, vocabulary, Mode, DEFAULT_EFFORT_BUDGET,
};
use llm_router::provider_scaffold::vocabulary_count::{resolve, ESTIMATOR_TOKENIZER};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::{AgentFunction, ThinkingLevel};
use llm_router::types::router::ResponseFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::reasoning::{is_reasoning_model, reasoning_effort_for};
use crate::request::build_response_format;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets. Reported back unchanged; every DeepSeek
    /// chat model shares the one vocabulary.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::deepseek::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
    /// The turn's thinking level. `off` counts the chat-mode prompt; any
    /// other level the thinking prompt at the effort it maps to; none the
    /// API default (thinking, `high`).
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    /// The turn's response format. DeepSeek renders the `json_object` body
    /// it becomes into the system message, so it is billed as prompt.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request, by DeepSeek's vocabulary.
    pub tokens: u64,
    /// Always `tokenizer`: the model's own vocabulary produced the count.
    pub estimator: String,
}

pub async fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    // A cold cache with no network is the one case with no honest answer
    // here. Reporting the typed no_token_counter error leaves the caller on
    // its own estimate, which is the same place it would have been without
    // this function — better than a number computed from the wrong
    // vocabulary.
    let tokenizer = resolve(&vocabulary()).await.ok_or_else(|| {
        Error::Handler(
            "router/no_token_counter: DeepSeek's vocabulary is not cached and could \
             not be fetched"
                .into(),
        )
    })?;
    let rows = to_wire_messages(
        &req.messages,
        req.system_prompt.as_deref().unwrap_or_default(),
    );
    let tools = functions_to_wire(req.tools.as_deref().unwrap_or(&[]));
    let response_format = req.response_format.as_ref().map(build_response_format);
    let tokens = tokenizer
        .encode(
            prompt(
                &rows,
                &tools,
                response_format.as_ref(),
                mode(&req.model, req.thinking_level),
            ),
            false,
        )
        .map_err(|err| Error::Handler(format!("router/no_token_counter: {err}")))?
        .len() as u64;
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}

/// The encoder mode the request runs in, read off the level as `stream`
/// resolves it: `off` disables thinking, every other level is the effort it
/// maps to, and no level, or a model that does not reason, is the API
/// default.
///
/// shortcut: the count sees neither `provider_options.thinking` nor the
/// catalog's `supports_thinking`, so a native `"disabled"` switch, or a
/// catalog record that overrides the id pattern, still counts thinking mode
/// (over by ~26 tokens plus the replayed reasoning). Forward them on
/// `router::count_tokens` if a caller starts sending the native switch.
fn mode(model: &str, level: Option<ThinkingLevel>) -> Mode {
    let level = level.filter(|_| is_reasoning_model(model, None));
    if level == Some(ThinkingLevel::Off) {
        return Mode::Chat;
    }
    Mode::Thinking {
        budget: reasoning_effort_for(level)
            .and_then(effort_budget)
            .unwrap_or(DEFAULT_EFFORT_BUDGET),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLASH: &str = "deepseek-v4-flash";

    #[test]
    fn a_model_that_does_not_reason_counts_the_default_whatever_the_level() {
        // stream sends no thinking params to such a model, so `off` does not
        // switch it to chat mode.
        let other = "qwen2.5-coder-7b-instruct";
        for level in [ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::Xhigh] {
            assert_eq!(mode(other, Some(level)), mode(other, None), "{level:?}");
        }
    }

    #[test]
    fn off_renders_chat_mode_and_levels_render_their_effort_budget() {
        assert_eq!(mode(FLASH, Some(ThinkingLevel::Off)), Mode::Chat);
        // No level: the API's own default, thinking at `high`.
        assert_eq!(mode(FLASH, None), Mode::Thinking { budget: 75 });
        for (level, budget) in [
            (ThinkingLevel::Minimal, 50),
            (ThinkingLevel::Low, 50),
            (ThinkingLevel::Medium, 75),
            (ThinkingLevel::High, 75),
            (ThinkingLevel::Xhigh, 100),
        ] {
            assert_eq!(
                mode(FLASH, Some(level)),
                Mode::Thinking { budget },
                "{level:?}"
            );
        }
    }

    #[test]
    fn the_request_takes_the_turns_level_and_format() {
        let req: CountTokensRequest = serde_json::from_value(serde_json::json!({
            "model": "deepseek-v4-flash",
            "messages": [],
            "thinking_level": "off",
            "response_format": {"type": "json"},
        }))
        .unwrap();
        assert_eq!(req.thinking_level, Some(ThinkingLevel::Off));
        assert_eq!(req.response_format.unwrap().r#type, "json");
    }
}
