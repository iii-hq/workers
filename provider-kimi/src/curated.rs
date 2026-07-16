//! Local catalog metadata for the Kimi (Moonshot) slice. Like OpenAI's,
//! Moonshot's `GET /v1/models` returns bare ids — no capability tree, no
//! display names, no limits — so live discovery owns the *id list* while this
//! module fills in everything the API cannot provide: a per-id display name,
//! per-family capability/pricing metadata, and conservative defaults.
//!
//! Display names are derived from the id itself (`pretty_name`) so every model
//! gets a distinct, readable label — `kimi-k2.5`, `kimi-k2.6`, and
//! `kimi-k2.7-code` must not all collapse to one "Kimi K2" row in the picker.
//! Capabilities/pricing stay keyed by family prefix.
//!
//! Moonshot model ids embed dates *mid-string* (`kimi-k2-0905-preview`), not as
//! a trailing ISO suffix like OpenAI, so family detection is prefix/substring
//! based and `base_id` is the identity (no dated-alias dedup to do).
//!
//! NOTE: context windows and pricing below are best-effort PLACEHOLDERS —
//! verify against platform.moonshot.ai before any release. A wrong row only
//! degrades display polish and cost enrichment, never routing.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};

fn price(input: f64, output: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        // Moonshot context caching discounts cache hits; no write surcharge.
        cache_read: Some(input * 0.1),
        cache_write: None,
    }
}

/// Title-case one id segment: `kimi`→`Kimi`, `k2.5`→`K2.5`, `v1`→`v1`,
/// `128k`→`128K`, `code`→`Code`, …
fn prettify_segment(seg: &str) -> String {
    let lower = seg.to_ascii_lowercase();
    // version tokens like `k2`, `k2.5`, `k2.7` → `K2`, `K2.5`, `K2.7`
    if let Some(rest) = lower.strip_prefix('k') {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return format!("K{rest}");
        }
    }
    // `v1`, `v2` keep the lowercase `v`
    if let Some(rest) = lower.strip_prefix('v') {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return lower;
        }
    }
    // window tokens `8k`, `32k`, `128k` → `8K`, `32K`, `128K`
    if let Some(num) = lower.strip_suffix('k') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return lower.to_ascii_uppercase();
        }
    }
    // generic: capitalize the first letter
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Human-readable display name derived from the model id, so every distinct id
/// gets a distinct label. Drops the noisy `preview` marker.
/// `moonshot-v1-128k-vision-preview` → `Moonshot v1 128K Vision`.
pub fn pretty_name(id: &str) -> String {
    id.split('-')
        .filter(|seg| !seg.is_empty() && !seg.eq_ignore_ascii_case("preview"))
        .map(prettify_segment)
        .collect::<Vec<_>>()
        .join(" ")
}

struct KimiMeta {
    context_window: u64,
    max_output_tokens: u64,
    supports_thinking: bool,
    supports_vision: bool,
    pricing: Pricing,
}

/// Prefix/substring family detection for capabilities + pricing over the
/// lowercased id. Returns None for ids we don't recognize (enrich falls back to
/// conservative defaults). The display name is derived separately, per id.
fn family_meta(id: &str) -> Option<KimiMeta> {
    let lower = id.to_ascii_lowercase();
    let has_vision = lower.contains("vision");
    let has_thinking = lower.contains("thinking");

    if lower.starts_with("kimi-thinking") {
        return Some(KimiMeta {
            context_window: 131_072,
            max_output_tokens: 32_768,
            supports_thinking: true,
            supports_vision: true,
            pricing: price(0.50, 2.50),
        });
    }
    if lower.starts_with("kimi-k2") {
        return Some(KimiMeta {
            context_window: 262_144,
            max_output_tokens: 32_768,
            supports_thinking: has_thinking,
            supports_vision: has_vision,
            pricing: price(0.60, 2.50),
        });
    }
    if lower.starts_with("kimi-latest") {
        return Some(KimiMeta {
            context_window: 131_072,
            max_output_tokens: 32_768,
            supports_thinking: false,
            supports_vision: true,
            pricing: price(0.60, 2.50),
        });
    }
    // moonshot-v1-{8k,32k,128k} (and their -vision-preview variants).
    if lower.starts_with("moonshot-v1-8k") {
        return Some(KimiMeta {
            context_window: 8_192,
            max_output_tokens: 4_096,
            supports_thinking: false,
            supports_vision: has_vision,
            pricing: price(0.20, 2.00),
        });
    }
    if lower.starts_with("moonshot-v1-32k") {
        return Some(KimiMeta {
            context_window: 32_768,
            max_output_tokens: 4_096,
            supports_thinking: false,
            supports_vision: has_vision,
            pricing: price(0.40, 2.50),
        });
    }
    if lower.starts_with("moonshot-v1-128k") {
        return Some(KimiMeta {
            context_window: 131_072,
            max_output_tokens: 4_096,
            supports_thinking: false,
            supports_vision: has_vision,
            pricing: price(0.60, 3.00),
        });
    }
    None
}

/// One live id → catalog Model: a per-id display name plus known-family
/// capability/pricing metadata when a prefix matches, conservative defaults
/// otherwise. Tools, context caching, and json_object structured output are
/// uniform across the Kimi families we admit.
pub fn enrich(id: &str) -> Model {
    let display_name = Some(pretty_name(id));
    match family_meta(id) {
        Some(meta) => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name,
            context_window: meta.context_window,
            max_output_tokens: meta.max_output_tokens,
            input_limit: None,
            supports_thinking: Some(meta.supports_thinking),
            supports_xhigh: None, // Moonshot has no effort ladder
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: Some(meta.supports_vision),
            supports_cache: Some(true),
            supports_structured_output: Some(true), // json_object mode
            thinking_budgets: None,
            pricing: Some(meta.pricing),
        },
        None => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name,
            context_window: 131_072,
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

/// Moonshot ids carry no trailing ISO-date suffix to strip, so the base id is
/// the id itself. Kept so discovery's dated-alias dedup stays a harmless no-op.
pub fn base_id(id: &str) -> &str {
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_name_is_distinct_and_readable_per_id() {
        assert_eq!(pretty_name("kimi-k2.5"), "Kimi K2.5");
        assert_eq!(pretty_name("kimi-k2.6"), "Kimi K2.6");
        assert_eq!(pretty_name("kimi-k2.7-code"), "Kimi K2.7 Code");
        assert_eq!(
            pretty_name("kimi-k2.7-code-highspeed"),
            "Kimi K2.7 Code Highspeed"
        );
        assert_eq!(pretty_name("moonshot-v1-128k"), "Moonshot v1 128K");
        assert_eq!(
            pretty_name("moonshot-v1-128k-vision-preview"),
            "Moonshot v1 128K Vision"
        );
        assert_eq!(pretty_name("moonshot-v1-auto"), "Moonshot v1 Auto");
        assert_eq!(pretty_name("kimi-thinking-preview"), "Kimi Thinking");
    }

    #[test]
    fn enrich_gives_distinct_names_within_the_k2_family() {
        // same capability profile, distinct labels — the picker bug fix
        let a = enrich("kimi-k2.5");
        let b = enrich("kimi-k2.7-code");
        assert_eq!(a.display_name.as_deref(), Some("Kimi K2.5"));
        assert_eq!(b.display_name.as_deref(), Some("Kimi K2.7 Code"));
        assert_eq!(a.context_window, 262_144);
        assert_eq!(b.context_window, 262_144);
        assert_ne!(a.display_name, b.display_name);
    }

    #[test]
    fn enrich_applies_family_metadata() {
        let m = enrich("kimi-k2.6");
        assert_eq!(m.id, "kimi-k2.6");
        assert_eq!(m.display_name.as_deref(), Some("Kimi K2.6"));
        assert_eq!(m.context_window, 262_144);
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_thinking, Some(false));
        assert!(m.pricing.is_some());

        let v = enrich("moonshot-v1-8k-vision-preview");
        assert_eq!(v.display_name.as_deref(), Some("Moonshot v1 8K Vision"));
        assert_eq!(v.context_window, 8_192);
        assert_eq!(v.supports_vision, Some(true));
    }

    #[test]
    fn thinking_families_flag_supports_thinking() {
        assert_eq!(
            enrich("kimi-thinking-preview").supports_thinking,
            Some(true)
        );
        assert_eq!(enrich("kimi-k2-thinking").supports_thinking, Some(true));
    }

    #[test]
    fn enrich_defaults_conservatively_for_unknown_families_but_still_names_them() {
        let m = enrich("kimi-future-9000");
        // unknown family still gets a derived display name (not None)
        assert_eq!(m.display_name.as_deref(), Some("Kimi Future 9000"));
        assert_eq!(m.context_window, 131_072);
        assert_eq!(m.max_output_tokens, 16_384);
        assert_eq!(m.supports_thinking, None);
        assert!(m.pricing.is_none());
        // unknown families still route and accept tools
        assert_eq!(m.supports_tools, Some(true));
    }

    #[test]
    fn base_id_is_identity() {
        assert_eq!(base_id("kimi-k2-0905-preview"), "kimi-k2-0905-preview");
        assert_eq!(base_id("moonshot-v1-128k"), "moonshot-v1-128k");
    }
}
