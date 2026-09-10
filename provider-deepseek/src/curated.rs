//! Local catalog metadata for the DeepSeek slice. `GET /models` returns bare
//! ids — no limits, no capabilities, no pricing (api-docs.deepseek.com,
//! "List Models") — so live discovery owns the *id list* and this module
//! fills in everything the API cannot provide: per-model metadata for the
//! models DeepSeek documents, conservative defaults for anything else.
//!
//! Figures from api-docs.deepseek.com (quick_start/pricing,
//! guides/thinking_mode), snapshot 2026-09-10. The live ids are
//! `deepseek-flash` (DeepSeek-V4.1-Flash) and `deepseek-v4-pro`
//! (DeepSeek-V4-Pro-0813); `deepseek-v4-flash` is a retired alias the API
//! still answers with Flash at the Flash price. DeepSeek has scheduled V4
//! Pro's retirement for 2026-09-14 12:00 Beijing time, after which
//! `deepseek-v4-pro` requests are answered by V4.1 Flash and billed at the
//! Flash price until V4.1 Pro ships; the row stays until the id leaves
//! `GET /models`. A stale row degrades cost display and limits, never routing
//! correctness.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing, ReasoningEffort};

/// Context window and max output for unknown ids — an `api_url` override can
/// point this provider at any OpenAI-compatible server. Deliberately the
/// floor of DeepSeek's own history (V3.x: 64K context, 8K output) so a wrong
/// guess truncates rather than 400s the request.
const UNKNOWN_CONTEXT_WINDOW: u64 = 65_536;
const UNKNOWN_MAX_OUTPUT_TOKENS: u64 = 8_192;

/// USD per MTok: (input on cache miss, input on cache hit, output), at the
/// PEAK rate — DeepSeek's list price. Off-peak is half of every figure and
/// applies outside 01:00–04:00 and 06:00–10:00 UTC Monday–Friday
/// (quick_start/pricing, note 3). The catalog has no time-of-day dimension,
/// so the one figure published is the one that never under-reports a bill.
// ponytail: peak price as the single figure; a time-of-day multiplier needs a
// catalog extension.
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
        price: (1.32, 0.044, 3.96),
    },
    Row {
        id: "deepseek-flash",
        display: "DeepSeek V4.1 Flash",
        context_window: 1_000_000,
        max_output_tokens: 384_000,
        price: (0.30, 0.006, 1.20),
    },
];

/// Legacy flash names the API still accepts but answers with V4.1 Flash at
/// the Flash price (quick_start/pricing, note 1). Metadata follows the model
/// that answers; the row keeps the id the caller used, because that is what
/// routes.
fn canonical(id: &str) -> &str {
    match id {
        "deepseek-v4-flash" | "deepseek-v4-flash-vision-exp" => "deepseek-flash",
        other => other,
    }
}

/// The wire vocabulary `reasoning_effort` takes, published so
/// `router::models::get` shows what a `thinking_level` lands on
/// (`reasoning::reasoning_effort_for`) instead of a bare `supports_thinking`.
fn efforts() -> Vec<ReasoningEffort> {
    [
        ("low", "thinking_level minimal / low"),
        ("high", "thinking_level medium / high; the API default"),
        ("max", "thinking_level xhigh"),
    ]
    .into_iter()
    .map(|(effort, description)| ReasoningEffort {
        effort: effort.into(),
        description: Some(description.into()),
    })
    .collect()
}

/// One live id → catalog Model: documented metadata when the id is known,
/// conservative defaults otherwise.
pub fn enrich(id: &str) -> Model {
    match ROWS.iter().find(|r| r.id == canonical(id)) {
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
                reasoning_efforts: Some(efforts()),
                // Flash documents vision, but this provider's wire is text
                // only (image blocks degrade to a marker — see
                // `wire::messages`), so advertising it would route images
                // into a turn that cannot carry them. Stays false until the
                // wire grows content parts.
                supports_vision: Some(false),
                pricing: Some(Pricing {
                    input: Some(input),
                    output: Some(output),
                    cache_read: Some(cached),
                    cache_write: None, // automatic caching, no write surcharge
                }),
                speech: None,
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
        speech: None,
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
        assert_eq!(p.input, Some(1.32));
        assert_eq!(p.cache_read, Some(0.044));
        assert_eq!(p.output, Some(3.96));
        assert!(p.cache_write.is_none());
        let efforts: Vec<&str> = m
            .reasoning_efforts
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.effort.as_str())
            .collect();
        assert_eq!(efforts, ["low", "high", "max"]);
    }

    #[test]
    fn flash_is_the_live_id_and_the_cheaper_row() {
        let m = enrich("deepseek-flash");
        assert_eq!(m.display_name.as_deref(), Some("DeepSeek V4.1 Flash"));
        assert_eq!(m.context_window, 1_000_000, "flash is not a 65K model");
        assert_eq!(m.max_output_tokens, 384_000);
        let flash = m.pricing.unwrap();
        let pro = enrich("deepseek-v4-pro").pricing.unwrap();
        assert!(flash.input < pro.input);
        assert!(flash.output < pro.output);
        assert_eq!(flash.input, Some(0.30));
        assert_eq!(flash.cache_read, Some(0.006));
        assert_eq!(flash.output, Some(1.20));
    }

    #[test]
    fn legacy_flash_aliases_get_flash_metadata_under_their_own_id() {
        for alias in ["deepseek-v4-flash", "deepseek-v4-flash-vision-exp"] {
            let m = enrich(alias);
            assert_eq!(m.id, alias, "the caller's id is what routes");
            assert_eq!(m.context_window, 1_000_000, "{alias} must not degrade");
            assert_eq!(m.pricing, enrich("deepseek-flash").pricing);
        }
    }

    #[test]
    fn advertised_efforts_cover_every_level_the_mapper_emits() {
        use crate::reasoning::reasoning_effort_for;
        use llm_router::types::model::ThinkingLevel;
        let advertised: Vec<String> = enrich("deepseek-v4-pro")
            .reasoning_efforts
            .unwrap()
            .into_iter()
            .map(|e| e.effort)
            .collect();
        for level in [
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::Xhigh,
        ] {
            let effort = reasoning_effort_for(Some(level)).unwrap();
            assert!(
                advertised.iter().any(|a| a == effort),
                "{level:?} → {effort} is not advertised in reasoning_efforts"
            );
        }
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
        assert!(m.reasoning_efforts.is_none());
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
