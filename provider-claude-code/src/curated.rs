//! Static fallback catalog. Live `GET /v1/models` is the source of truth (see
//! discovery.rs); this slice is reconciled only when the live endpoint rejects
//! the subscription OAuth bearer (401/403), so the picker still shows usable
//! models. Pricing is intentionally absent — a Pro/Max subscription is flat-fee,
//! not per-token.
use llm_router::types::model::Model;

/// Base model ids offered on the Claude Pro/Max subscription. Keep roughly in
/// step with the live catalog; a stale entry only affects the fallback path
/// (when live discovery is unavailable), never normal operation. `adaptive`
/// marks the adaptive-thinking generation (thinking gated off otherwise).
const FALLBACK: &[(&str, &str, bool)] = &[
    ("claude-opus-4-8", "Claude Opus 4.8 (Claude Code)", true),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6 (Claude Code)", true),
    ("claude-fable-5", "Claude Fable 5 (Claude Code)", true),
    ("claude-mythos-5", "Claude Mythos 5 (Claude Code)", true),
    ("claude-haiku-4-5", "Claude Haiku 4.5 (Claude Code)", false),
];

fn fallback_model(base_id: &str, display_name: &str, adaptive: bool) -> Model {
    Model {
        id: format!("claude-code/{base_id}"),
        provider: crate::PROVIDER_ID.into(),
        display_name: Some(display_name.to_string()),
        context_window: 200_000,
        max_output_tokens: 8192,
        input_limit: None,
        supports_thinking: Some(adaptive),
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: Some(true),
        supports_vision: Some(true),
        supports_cache: Some(true),
        supports_structured_output: Some(false),
        thinking_budgets: None,
        // Subscription billing: no per-token pricing to enrich with.
        pricing: None,
    }
}

/// The static fallback slice, namespaced `claude-code/<id>`.
pub fn curated_models() -> Vec<Model> {
    FALLBACK
        .iter()
        .map(|(id, name, adaptive)| fallback_model(id, name, *adaptive))
        .collect()
}

/// Strip a trailing `-YYYYMMDD` date suffix
/// (`claude-sonnet-4-6-20260115` → `claude-sonnet-4-6`).
pub fn base_id(id: &str) -> &str {
    if id.len() > 9 {
        let (head, tail) = id.split_at(id.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return head;
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_slice_is_namespaced_and_unpriced() {
        let models = curated_models();
        assert!(!models.is_empty());
        for m in &models {
            assert!(m.id.starts_with("claude-code/"), "id namespaced: {}", m.id);
            assert_eq!(m.provider, "claude-code");
            assert!(m.pricing.is_none(), "subscription has no pricing");
        }
        let haiku = models
            .iter()
            .find(|m| m.id == "claude-code/claude-haiku-4-5")
            .unwrap();
        assert_eq!(haiku.supports_thinking, Some(false));
        let opus = models
            .iter()
            .find(|m| m.id == "claude-code/claude-opus-4-8")
            .unwrap();
        assert_eq!(opus.supports_thinking, Some(true));
    }

    #[test]
    fn base_id_strips_only_date_suffixes() {
        assert_eq!(base_id("claude-sonnet-4-6-20260115"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-sonnet-4-6"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-haiku-4-5"), "claude-haiku-4-5");
    }
}
