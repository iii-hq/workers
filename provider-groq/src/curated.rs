//! Local catalog metadata for the Groq slice.
//!
//! Groq's `GET /models` is unusually generous — it carries `context_window`
//! and `active` alongside each id — so live discovery owns the id list and
//! this module fills in what the listing cannot say: display names, output
//! ceilings, capabilities, and pricing.
//!
//! Prices are USD per MTok, snapshot 2026-08. Groq's own pricing page renders
//! its figures client-side and ships none in the document, so these come from
//! published third-party tracking rather than from Groq directly, and are
//! worth re-checking before anyone leans on the cost display. A stale row
//! degrades cost, never routing correctness.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};

/// Context window and max output for an id no row knows. Groq serves several
/// model families and adds to them often, and an `api_url` override can point
/// this provider at any OpenAI-compatible server, so the floor is deliberately
/// conservative: a wrong guess should truncate rather than 400 the request.
const UNKNOWN_CONTEXT_WINDOW: u64 = 8_192;
const UNKNOWN_MAX_OUTPUT_TOKENS: u64 = 4_096;

struct Row {
    id: &'static str,
    display: &'static str,
    context_window: u64,
    max_output_tokens: u64,
    /// (input, cached input, output) per MTok. Cached is `None` for a model
    /// Groq publishes no cache rate for.
    price: (f64, Option<f64>, f64),
    /// Whether the model exposes reasoning through `reasoning_effort`.
    thinking: bool,
}

const ROWS: &[Row] = &[
    Row {
        id: "llama-3.1-8b-instant",
        display: "Llama 3.1 8B Instant",
        context_window: 131_072,
        max_output_tokens: 131_072,
        price: (0.05, None, 0.08),
        thinking: false,
    },
    Row {
        id: "llama-3.3-70b-versatile",
        display: "Llama 3.3 70B Versatile",
        context_window: 131_072,
        max_output_tokens: 32_768,
        price: (0.59, None, 0.79),
        thinking: false,
    },
    Row {
        id: "openai/gpt-oss-20b",
        display: "GPT-OSS 20B",
        context_window: 131_072,
        max_output_tokens: 65_536,
        price: (0.075, Some(0.0375), 0.30),
        thinking: true,
    },
    Row {
        id: "openai/gpt-oss-120b",
        display: "GPT-OSS 120B",
        context_window: 131_072,
        max_output_tokens: 65_536,
        price: (0.15, Some(0.075), 0.60),
        thinking: true,
    },
];

/// One live id → catalog Model: documented metadata when the id is known,
/// conservative defaults otherwise.
pub fn enrich(id: &str) -> Model {
    match ROWS.iter().find(|r| r.id == id) {
        Some(r) => {
            let (input, cached, output) = r.price;
            Model {
                display_name: Some(r.display.into()),
                context_window: r.context_window,
                max_output_tokens: r.max_output_tokens,
                supports_thinking: Some(r.thinking),
                // `reasoning_effort` is an enum here, and no model documents
                // a tier above high.
                supports_xhigh: Some(false),
                supports_vision: Some(false),
                pricing: Some(Pricing {
                    input: Some(input),
                    output: Some(output),
                    cache_read: cached,
                    cache_write: None,
                }),
                ..base(id)
            }
        }
        None => base(id),
    }
}

/// The shared skeleton: what holds for every id this provider serves.
/// Unknown families leave thinking and vision unset — `reasoning.rs` decides
/// per request rather than the catalog asserting a capability it cannot know
/// for a model Groq added after this snapshot.
fn base(id: &str) -> Model {
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
        supports_tools: Some(true),
        supports_vision: None,
        // Prompt caching needs no request markers where it applies.
        supports_cache: Some(true),
        // The OpenAI-compatible surface takes `response_format`, json_schema
        // included.
        supports_structured_output: Some(true),
        thinking_budgets: None,
        pricing: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_models_carry_their_metadata() {
        let m = enrich("llama-3.3-70b-versatile");
        assert_eq!(m.display_name.as_deref(), Some("Llama 3.3 70B Versatile"));
        assert_eq!(m.provider, "groq");
        assert_eq!(m.context_window, 131_072);
        assert_eq!(m.max_output_tokens, 32_768);
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_vision, Some(false));
        let p = m.pricing.unwrap();
        assert_eq!(p.input, Some(0.59));
        assert_eq!(p.output, Some(0.79));
        assert!(p.cache_write.is_none());
    }

    #[test]
    fn a_reasoning_model_is_marked_and_carries_its_cache_rate() {
        let m = enrich("openai/gpt-oss-120b");
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_xhigh, Some(false));
        assert_eq!(m.pricing.and_then(|p| p.cache_read), Some(0.075));
    }

    #[test]
    fn a_non_reasoning_row_is_marked_as_such() {
        // The families differ here in a way they do not at a single-family
        // provider: Llama does not reason, GPT-OSS does.
        assert_eq!(
            enrich("llama-3.1-8b-instant").supports_thinking,
            Some(false)
        );
    }

    #[test]
    fn unknown_ids_get_conservative_defaults_and_never_vanish() {
        let m = enrich("some-model-shipped-tomorrow");
        assert_eq!(m.id, "some-model-shipped-tomorrow");
        assert_eq!(m.display_name, None);
        assert_eq!(m.context_window, UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(m.max_output_tokens, UNKNOWN_MAX_OUTPUT_TOKENS);
        assert_eq!(m.supports_thinking, None);
        assert!(m.pricing.is_none());
        // Tools and structured output hold for the whole OpenAI-compatible
        // surface, known model or not.
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_structured_output, Some(true));
    }

    #[test]
    fn rows_are_unique_and_every_row_prices_out() {
        let mut ids: Vec<&str> = ROWS.iter().map(|r| r.id).collect();
        let len = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate ids in ROWS");
        for r in ROWS {
            let p = enrich(r.id).pricing.expect("documented row prices out");
            assert!(p.input.is_some_and(|v| v > 0.0), "{}", r.id);
            assert!(p.output.is_some_and(|v| v > 0.0), "{}", r.id);
            // A cache hit is billed at a discount when it is billed at all.
            if let Some(cached) = p.cache_read {
                assert!(Some(cached) < p.input, "{}", r.id);
            }
        }
    }
}
