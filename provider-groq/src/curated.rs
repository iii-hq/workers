//! Defaults for the Groq slice, and the little the live listing cannot say.
//!
//! Groq's `GET /models` is the richest listing of any provider here: it
//! carries the display name, the context window, the output ceiling, the
//! modalities, the supported features, live pricing per token, and even the
//! HuggingFace id of the model's own weights. Nearly everything a hand-kept
//! table would hold is therefore served by the API, and served fresh — so
//! this module holds only the floor a row falls back to when a listing (a
//! gateway behind an `api_url` override, say) says nothing at all.
//!
//! Keeping a local price table alongside a live one would be strictly worse:
//! it would go stale silently, and Groq's own numbers are right here.
use crate::PROVIDER_ID;
use llm_router::types::model::Model;

/// Context window and max output for a row that reports neither. An
/// `api_url` override can point this provider at any OpenAI-compatible
/// server, so the floor is deliberately conservative: a wrong guess should
/// truncate rather than 400 the request.
pub const UNKNOWN_CONTEXT_WINDOW: u64 = 8_192;
pub const UNKNOWN_MAX_OUTPUT_TOKENS: u64 = 4_096;

/// The skeleton every row starts from. Capabilities are left unset rather
/// than assumed: Groq hosts other people's models, and they genuinely differ
/// — `llama-3.1-8b-instant` takes tools, `allam-2-7b` does not, and only the
/// GPT-OSS models do structured output. Discovery fills these in from
/// `supported_features`, and a listing that omits the field leaves them
/// unknown rather than claimed.
pub fn base(id: &str) -> Model {
    Model {
        id: id.into(),
        provider: PROVIDER_ID.into(),
        display_name: None,
        context_window: UNKNOWN_CONTEXT_WINDOW,
        max_output_tokens: UNKNOWN_MAX_OUTPUT_TOKENS,
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: None,
        supports_vision: None,
        supports_cache: None,
        supports_structured_output: None,
        thinking_budgets: None,
        pricing: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_claims_no_capability_it_has_not_been_told_about() {
        // The bug this guards: asserting a provider-wide capability at a host
        // that serves other people's models, where the models disagree.
        let m = base("some-model-shipped-tomorrow");
        assert_eq!(m.id, "some-model-shipped-tomorrow");
        assert_eq!(m.provider, "groq");
        assert_eq!(m.supports_tools, None);
        assert_eq!(m.supports_vision, None);
        assert_eq!(m.supports_thinking, None);
        assert_eq!(m.supports_structured_output, None);
        assert!(m.pricing.is_none());
    }

    #[test]
    fn the_floor_is_conservative_enough_to_truncate_rather_than_fail() {
        let m = base("anything");
        assert_eq!(m.context_window, UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(m.max_output_tokens, UNKNOWN_MAX_OUTPUT_TOKENS);
    }
}
