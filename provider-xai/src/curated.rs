//! Local catalog metadata for the xAI slice. Like OpenAI's, xAI's
//! `GET /v1/models` returns bare ids — no capability tree, no display names,
//! no limits — so live discovery owns the *id list* while this module fills
//! in everything the API cannot provide: per-family metadata for known grok
//! families, conservative defaults for unknown ones, and the legacy denylist.
//!
//! Pricing for grok-4.3 and grok-build is taken from docs.x.ai; other rows
//! are best-effort and only affect cost display, never routing. A missing row
//! degrades display polish, not behaviour.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};

fn price(input: f64, output: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(input * 0.25), // xAI cached-input discount
        cache_write: None,
    }
}

/// Legacy grok generations (grok-1, grok-2). Kept permissive: an unrecognized
/// future family (grok-5, a new variant) flows straight into the catalog
/// rather than vanishing.
pub fn is_legacy_generation(model_id: &str) -> bool {
    let base = base_id(model_id).to_ascii_lowercase();
    base.starts_with("grok-1") || base.starts_with("grok-2")
}

/// Per-family metadata: (display, context_window, max_output_tokens,
/// reasoning, vision, pricing). Matched on the lowercased base id, most
/// specific family first.
fn family_meta(base: &str) -> Option<(&'static str, u64, u64, bool, bool, Pricing)> {
    let b = base.to_ascii_lowercase();
    if b.starts_with("grok-build") {
        return Some(("Grok Build", 256_000, 16_384, false, false, price(1.0, 2.0)));
    }
    if b.contains("code-fast") {
        return Some((
            "Grok Code Fast",
            256_000,
            16_384,
            true,
            false,
            price(0.20, 1.50),
        ));
    }
    if b.starts_with("grok-4.3") {
        return Some(("Grok 4.3", 1_000_000, 64_000, true, false, price(1.25, 2.5)));
    }
    if b.contains("grok-4") && b.contains("fast") {
        return Some((
            "Grok 4 Fast",
            2_000_000,
            32_768,
            true,
            true,
            price(0.20, 0.50),
        ));
    }
    if b.starts_with("grok-4") {
        return Some(("Grok 4", 256_000, 32_768, true, true, price(3.0, 15.0)));
    }
    if b.starts_with("grok-3-mini") {
        return Some((
            "Grok 3 Mini",
            131_072,
            16_384,
            true,
            false,
            price(0.30, 0.50),
        ));
    }
    if b.starts_with("grok-3") {
        return Some(("Grok 3", 131_072, 16_384, false, false, price(3.0, 15.0)));
    }
    None
}

/// One live id → catalog Model: known-family metadata when the base id
/// matches a grok family, conservative defaults otherwise. xAI's chat models
/// accept tools; reasoning/vision vary by family (both stay unknown for
/// unrecognized families — reasoning.rs id-patterns decide per request).
pub fn enrich(id: &str) -> Model {
    let base = base_id(id);
    match family_meta(base) {
        Some((display, context_window, max_output_tokens, reasoning, vision, pricing)) => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name: Some(display.into()),
            context_window,
            max_output_tokens,
            input_limit: None,
            supports_thinking: Some(reasoning),
            supports_xhigh: Some(false), // xAI reasoning_effort is low/high only
            supports_tools: Some(true),
            supports_vision: Some(vision),
            supports_cache: Some(true),
            supports_structured_output: Some(true), // native json_schema response_format
            thinking_budgets: None,                 // effort enum, not token budgets
            pricing: Some(pricing),
        },
        None => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name: None,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_limit: None,
            supports_thinking: None,
            supports_xhigh: None,
            supports_tools: Some(true),
            supports_vision: None,
            supports_cache: Some(true),
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        },
    }
}

/// Strip a trailing `-MMDD` snapshot suffix (`grok-4-0709` → `grok-4`,
/// `grok-2-1212` → `grok-2`). xAI pins dated snapshots with a 4-digit
/// month-day tail; undated aliases carry no suffix.
pub fn base_id(id: &str) -> &str {
    if id.len() > 5 {
        let (head, tail) = id.split_at(id.len() - 5);
        let bytes = tail.as_bytes();
        if bytes[0] == b'-' && tail[1..].chars().all(|c| c.is_ascii_digit()) {
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
        for legacy in ["grok-1", "grok-2", "grok-2-1212", "grok-2-vision-1212"] {
            assert!(is_legacy_generation(legacy), "{legacy} should be legacy");
        }
        for current in ["grok-3", "grok-3-mini", "grok-4", "grok-4.3", "grok-5"] {
            assert!(!is_legacy_generation(current), "{current} should be kept");
        }
    }

    #[test]
    fn enrich_applies_family_metadata_via_base_id() {
        let m = enrich("grok-4.3");
        assert_eq!(m.display_name.as_deref(), Some("Grok 4.3"));
        assert_eq!(m.context_window, 1_000_000);
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_structured_output, Some(true));
        assert_eq!(m.pricing.as_ref().unwrap().input, Some(1.25));

        let build = enrich("grok-build-0.1");
        assert_eq!(build.display_name.as_deref(), Some("Grok Build"));
        assert_eq!(build.supports_thinking, Some(false));
        assert_eq!(build.pricing.as_ref().unwrap().output, Some(2.0));

        let mini = enrich("grok-3-mini");
        assert_eq!(mini.display_name.as_deref(), Some("Grok 3 Mini"));
        assert_eq!(mini.supports_thinking, Some(true));
    }

    #[test]
    fn enrich_defaults_conservatively_for_unknown_families() {
        let m = enrich("grok-9-experimental");
        assert_eq!(m.display_name, None);
        assert_eq!(m.context_window, 131_072);
        assert_eq!(m.supports_thinking, None);
        assert_eq!(m.supports_tools, Some(true));
        assert!(m.pricing.is_none());
    }

    #[test]
    fn base_id_strips_only_mmdd_snapshot_suffixes() {
        assert_eq!(base_id("grok-4-0709"), "grok-4");
        assert_eq!(base_id("grok-2-1212"), "grok-2");
        assert_eq!(base_id("grok-4.3"), "grok-4.3");
        assert_eq!(base_id("grok-3-mini"), "grok-3-mini");
    }
}
