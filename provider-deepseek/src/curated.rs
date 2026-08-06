//! Local catalog metadata for the DeepSeek slice. `GET /models` returns bare
//! ids — no limits, no capabilities, no pricing (api-docs.deepseek.com,
//! "List Models") — so live discovery owns the *id list* and this module
//! fills in everything the API cannot provide: per-model metadata for the
//! models DeepSeek documents, conservative defaults for anything else.
//!
//! Figures from api-docs.deepseek.com (quick_start/pricing,
//! guides/thinking_mode, guides/json_mode), snapshot 2026-08. A stale row
//! degrades cost display and limits, never routing correctness.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};

/// Context window and max output for unknown ids — an `api_url` override can
/// point this provider at any OpenAI-compatible server. Deliberately the
/// floor of DeepSeek's own history (V3.x: 64K context, 8K output) so a wrong
/// guess truncates rather than 400s the request.
const UNKNOWN_CONTEXT_WINDOW: u64 = 65_536;
const UNKNOWN_MAX_OUTPUT_TOKENS: u64 = 8_192;

/// USD per MTok: (input on cache miss, input on cache hit, output).
/// The regular list price, which is what is billed today. DeepSeek has
/// announced — but not yet scheduled — a peak/off-peak policy charging 2x
/// the regular price during 09:00–12:00 and 14:00–18:00 Beijing time
/// ("the effective date will be subject to the official announcement").
/// These rows stay the regular price when that lands; only a time-of-day
/// multiplier the catalog has no way to express would need adding.
struct Row {
    id: &'static str,
    display: &'static str,
    context_window: u64,
    max_output_tokens: u64,
    price: (f64, f64, f64),
}

const ROWS: &[Row] = &[
    Row {
        id: "deepseek-v4-pro",
        display: "DeepSeek V4 Pro",
        context_window: 1_000_000,
        max_output_tokens: 384_000,
        price: (0.435, 0.003625, 0.87),
    },
    Row {
        id: "deepseek-v4-flash",
        display: "DeepSeek V4 Flash",
        context_window: 1_000_000,
        max_output_tokens: 384_000,
        price: (0.14, 0.0028, 0.28),
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
                // Hybrid reasoning: `thinking: {type}` toggles it on every
                // V4 model, and `reasoning_effort: max` is accepted by both.
                supports_thinking: Some(true),
                supports_xhigh: Some(true),
                supports_vision: Some(false),
                pricing: Some(Pricing {
                    input: Some(input),
                    output: Some(output),
                    cache_read: Some(cached),
                    cache_write: None, // automatic caching, no write surcharge
                }),
                ..base(id)
            }
        }
        None => base(id),
    }
}

/// The shared skeleton: what holds for every id this provider serves.
/// Unknown families leave thinking/vision unset — `reasoning.rs` id-patterns
/// decide per request rather than the catalog asserting a capability.
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
        // Context caching on disk is on by default for every account and
        // needs no request markers (guides/kv_cache).
        supports_cache: Some(true),
        // DeepSeek documents `json_object` only — no strict json_schema mode.
        supports_structured_output: Some(false),
        thinking_budgets: None, // toggle + effort enum, not token budgets
        pricing: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_models_carry_their_metadata() {
        let m = enrich("deepseek-v4-pro");
        assert_eq!(m.display_name.as_deref(), Some("DeepSeek V4 Pro"));
        assert_eq!(m.provider, "deepseek");
        assert_eq!(m.context_window, 1_000_000);
        assert_eq!(m.max_output_tokens, 384_000);
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_xhigh, Some(true));
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_vision, Some(false));
        assert_eq!(m.supports_cache, Some(true));
        assert_eq!(m.supports_structured_output, Some(false));
        let p = m.pricing.unwrap();
        assert_eq!(p.input, Some(0.435));
        assert_eq!(p.cache_read, Some(0.003625));
        assert_eq!(p.output, Some(0.87));
        assert!(p.cache_write.is_none());
    }

    #[test]
    fn flash_is_the_cheaper_row() {
        let flash = enrich("deepseek-v4-flash").pricing.unwrap();
        let pro = enrich("deepseek-v4-pro").pricing.unwrap();
        assert!(flash.input < pro.input);
        assert!(flash.output < pro.output);
        assert_eq!(flash.input, Some(0.14));
        assert_eq!(flash.output, Some(0.28));
    }

    #[test]
    fn unknown_ids_get_conservative_defaults_and_never_vanish() {
        let m = enrich("deepseek-v9-unreleased");
        assert_eq!(m.id, "deepseek-v9-unreleased");
        assert_eq!(m.display_name, None);
        assert_eq!(m.context_window, 65_536);
        assert_eq!(m.max_output_tokens, 8_192);
        assert_eq!(m.supports_thinking, None);
        assert_eq!(m.supports_xhigh, None);
        assert!(m.pricing.is_none());
        // structured output and tools hold for the whole OpenAI-compatible
        // surface, known model or not.
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_structured_output, Some(false));
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
            // Cache hits are billed at a discount, never above list input.
            assert!(p.cache_read < p.input, "{}", r.id);
        }
    }
}
