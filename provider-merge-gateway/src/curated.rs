//! Local catalog metadata for the OpenAI slice. Unlike Anthropic's models
//! API, OpenAI's `GET /v1/models` returns bare ids — no capability tree, no
//! display names, no limits — so live discovery owns the *id list* while
//! this module fills in everything the API cannot provide: per-family
//! metadata for known families, conservative defaults for unknown ones, and
//! the legacy-generation denylist.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};

fn price(input: f64, output: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(input * 0.1),
        cache_write: None, // automatic caching; no write surcharge
    }
}

/// Known legacy families (pre-gpt-5 generations). With no capability data on
/// the wire this has to be a name denylist, unlike provider-anthropic's
/// capability-driven filter — kept permissive on purpose: an unrecognized
/// future family (gpt-6, a new letter series) flows straight into the
/// catalog rather than vanishing.
pub fn is_legacy_generation(model_id: &str) -> bool {
    let base = base_id(model_id).to_ascii_lowercase();
    base.starts_with("gpt-3")
        || base.starts_with("gpt-4")
        || base.starts_with("chatgpt")
        || base.starts_with("o1")
        || base.starts_with("o3")
        || base.starts_with("o4")
}

/// Hand-maintained metadata for the families we know (USD per MTok; verify
/// against openai.com/pricing before release). A missing row only degrades
/// display polish and cost enrichment, never routing.
fn family_meta(base: &str) -> Option<(&'static str, u64, u64, bool, Pricing)> {
    match base {
        "gpt-5.2" => Some(("GPT-5.2", 400_000, 128_000, true, price(1.75, 14.0))),
        "gpt-5.1" => Some(("GPT-5.1", 400_000, 128_000, false, price(1.25, 10.0))),
        "gpt-5-mini" => Some(("GPT-5 Mini", 400_000, 128_000, false, price(0.25, 2.0))),
        "gpt-5-nano" => Some(("GPT-5 Nano", 400_000, 128_000, false, price(0.05, 0.40))),
        _ => None,
    }
}

/// One live id → catalog Model: known-family metadata when the base id
/// matches, conservative defaults otherwise. Tools and automatic caching are
/// uniform across the chat families we admit; vision/thinking/xhigh stay
/// unknown for unrecognized families (reasoning.rs id-patterns decide per
/// request).
pub fn enrich(id: &str) -> Model {
    let base = base_id(id);
    match family_meta(base) {
        Some((display, context_window, max_output_tokens, xhigh, pricing)) => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name: Some(display.into()),
            context_window,
            max_output_tokens,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: Some(xhigh),
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: Some(true),
            supports_structured_output: Some(true), // native json_schema mode
            thinking_budgets: None,                 // effort enum, not token budgets
            pricing: Some(pricing),
        },
        None => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name: None,
            context_window: 128_000,
            max_output_tokens: 16_384,
            input_limit: None,
            supports_thinking: None,
            supports_xhigh: None,
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: None,
            supports_cache: Some(true),
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        },
    }
}

/// Strip a trailing `-YYYY-MM-DD` date suffix
/// (`gpt-5.1-2025-11-13` → `gpt-5.1`). OpenAI dates use hyphenated ISO form,
/// unlike Anthropic's compact `-YYYYMMDD`.
pub fn base_id(id: &str) -> &str {
    if id.len() > 11 {
        let (head, tail) = id.split_at(id.len() - 11);
        let bytes = tail.as_bytes();
        let shape_ok = bytes[0] == b'-'
            && bytes[5] == b'-'
            && bytes[8] == b'-'
            && tail
                .char_indices()
                .all(|(i, c)| matches!(i, 0 | 5 | 8) || c.is_ascii_digit());
        if shape_ok {
            return head;
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_generations_are_flagged_current_ones_are_not() {
        for legacy in [
            "gpt-3.5-turbo",
            "gpt-4",
            "gpt-4-turbo-2024-04-09",
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4.1-nano",
            "chatgpt-4o-latest",
            "o1-pro",
            "o3-mini",
            "o4-mini-2025-04-16",
        ] {
            assert!(is_legacy_generation(legacy), "{legacy} should be legacy");
        }
        for current in [
            "gpt-5",
            "gpt-5-mini",
            "gpt-5.1-2025-11-13",
            "gpt-5.4-pro",
            "gpt-5.5",
            "gpt-6", // future family stays permissive
        ] {
            assert!(!is_legacy_generation(current), "{current} should be kept");
        }
    }

    #[test]
    fn enrich_applies_family_metadata_via_base_id() {
        let m = enrich("gpt-5.1-2025-11-13");
        assert_eq!(m.id, "gpt-5.1-2025-11-13");
        assert_eq!(m.display_name.as_deref(), Some("GPT-5.1"));
        assert_eq!(m.context_window, 400_000);
        assert_eq!(m.supports_structured_output, Some(true));
        assert_eq!(m.pricing.as_ref().unwrap().input, Some(1.25));
        assert!(m.pricing.as_ref().unwrap().cache_write.is_none());
    }

    #[test]
    fn enrich_defaults_conservatively_for_unknown_families() {
        let m = enrich("gpt-5.4-pro");
        assert_eq!(m.display_name, None);
        assert_eq!(m.context_window, 128_000);
        assert_eq!(m.max_output_tokens, 16_384);
        assert_eq!(m.supports_thinking, None);
        assert!(m.pricing.is_none());
    }

    #[test]
    fn base_id_strips_only_iso_date_suffixes() {
        assert_eq!(base_id("gpt-5.1-2025-11-13"), "gpt-5.1");
        assert_eq!(base_id("gpt-5.1"), "gpt-5.1");
        assert_eq!(base_id("gpt-5.1-turbo-extra"), "gpt-5.1-turbo-extra");
    }
}
