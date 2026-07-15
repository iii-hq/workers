//! The model-adaptive token budget (context-manager.md § Token budget
//! model):
//!
//! ```text
//! usable = max(0, (input_limit ?? (context_window - max_output_tokens))
//!              - reserved - thinking_budget)
//! ```
//!
//! `reserved` defaults to `min(cap, pct% of context_window)`; the
//! thinking budget comes from the model's declared budget for the
//! requested tier (0 when the caller passes no tier or the model
//! declares none).

use crate::config::WorkerConfig;
use crate::types::{Model, ModelLimits, ThinkingLevel};

/// How the limits in a [`ResolvedModel`] were obtained — echoed to
/// callers as `model_resolved` so a silent fallback is detectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelResolved {
    Inline,
    Router,
    Fallback,
}

impl ModelResolved {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelResolved::Inline => "inline",
            ModelResolved::Router => "router",
            ModelResolved::Fallback => "fallback",
        }
    }
}

/// Limits plus the metadata budget math needs, with provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModel {
    pub limits: ModelLimits,
    /// Declared per-tier reasoning budgets (router-resolved models
    /// only; inline limits carry none).
    pub thinking_budgets: Option<std::collections::BTreeMap<ThinkingLevel, u64>>,
    pub resolved: ModelResolved,
}

/// The conservative fallback used when neither inline limits nor the
/// router can resolve the model.
pub fn fallback_model() -> ResolvedModel {
    ResolvedModel {
        limits: ModelLimits {
            context_window: 8_192,
            max_output_tokens: 1_024,
            input_limit: None,
        },
        thinking_budgets: None,
        resolved: ModelResolved::Fallback,
    }
}

impl ResolvedModel {
    pub fn from_inline(limits: ModelLimits) -> Self {
        Self {
            limits,
            thinking_budgets: None,
            resolved: ModelResolved::Inline,
        }
    }

    pub fn from_router(model: &Model, effective_max_output_tokens: u64) -> Self {
        Self {
            limits: ModelLimits {
                context_window: model.context_window,
                max_output_tokens: effective_max_output_tokens.min(model.max_output_tokens),
                input_limit: model.input_limit,
            },
            thinking_budgets: model.thinking_budgets.clone(),
            resolved: ModelResolved::Router,
        }
    }

    /// The model's declared budget for `level`, else 0.
    pub fn thinking_budget(&self, level: Option<ThinkingLevel>) -> u64 {
        match (level, &self.thinking_budgets) {
            (Some(level), Some(budgets)) => budgets.get(&level).copied().unwrap_or(0),
            _ => 0,
        }
    }
}

/// Default reserve: `min(cap, context_window * pct / 100)`.
pub fn default_reserved(cfg: &WorkerConfig, context_window: u64) -> u64 {
    cfg.reserved_tokens_cap
        .min(context_window.saturating_mul(cfg.reserved_pct) / 100)
}

/// The usable input budget for one call. Saturating throughout — a
/// tiny model with a large reserve clamps to 0 instead of wrapping.
pub fn usable(limits: &ModelLimits, reserved: u64, thinking_budget: u64) -> u64 {
    let base = limits.input_limit.unwrap_or_else(|| {
        limits
            .context_window
            .saturating_sub(limits.max_output_tokens)
    });
    base.saturating_sub(reserved)
        .saturating_sub(thinking_budget)
}

/// Adaptive verbatim-tail budget for compaction: 25% of `usable`,
/// clamped to [2_000, 8_000]; the caller's `preserve_recent_tokens`
/// overrides it (same policy as the harness prior art).
pub fn preserve_recent_budget(usable: u64, override_tokens: Option<u64>) -> u64 {
    const MIN_PRESERVE_RECENT_TOKENS: u64 = 2_000;
    const MAX_PRESERVE_RECENT_TOKENS: u64 = 8_000;
    if let Some(ovr) = override_tokens {
        return ovr;
    }
    (usable / 4).clamp(MIN_PRESERVE_RECENT_TOKENS, MAX_PRESERVE_RECENT_TOKENS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(
        context_window: u64,
        max_output_tokens: u64,
        input_limit: Option<u64>,
    ) -> ModelLimits {
        ModelLimits {
            context_window,
            max_output_tokens,
            input_limit,
        }
    }

    #[test]
    fn reserved_defaults_to_ten_pct_capped_at_20k() {
        let cfg = WorkerConfig::default();
        // 10% of 200k = 20k, equal to the cap.
        assert_eq!(default_reserved(&cfg, 200_000), 20_000);
        // 10% of 32k = 3.2k, under the cap.
        assert_eq!(default_reserved(&cfg, 32_000), 3_200);
        // 10% of 1M = 100k, capped at 20k.
        assert_eq!(default_reserved(&cfg, 1_000_000), 20_000);
    }

    #[test]
    fn spec_examples_hold() {
        let cfg = WorkerConfig::default();
        // "A 200k model with defaults yields ~180k usable"
        let l = limits(200_000, 8_000, None);
        let u = usable(&l, default_reserved(&cfg, l.context_window), 0);
        assert_eq!(u, 172_000); // 200k - 8k output - 20k reserved

        // "a 32k model yields ~12k" (with a 16k output budget)
        let l = limits(32_000, 16_000, None);
        let u = usable(&l, default_reserved(&cfg, l.context_window), 0);
        assert_eq!(u, 12_800); // 32k - 16k - 3.2k
    }

    #[test]
    fn input_limit_takes_precedence_over_derivation() {
        let l = limits(200_000, 8_000, Some(100_000));
        assert_eq!(usable(&l, 20_000, 0), 80_000);
    }

    #[test]
    fn thinking_budget_subtracts() {
        let l = limits(200_000, 8_000, None);
        assert_eq!(usable(&l, 20_000, 16_000), 156_000);
    }

    #[test]
    fn tiny_models_clamp_to_zero() {
        let l = limits(1_000, 2_000, None); // output exceeds window
        assert_eq!(usable(&l, 0, 0), 0);
        let l = limits(8_192, 1_024, None);
        assert_eq!(usable(&l, 10_000, 0), 0); // reserve exceeds base
    }

    #[test]
    fn fallback_is_8192_1024() {
        let f = fallback_model();
        assert_eq!(f.limits.context_window, 8_192);
        assert_eq!(f.limits.max_output_tokens, 1_024);
        assert_eq!(f.resolved, ModelResolved::Fallback);
    }

    #[test]
    fn router_effective_output_cap_prevents_premature_compaction() {
        let model = Model {
            id: "gpt-5.4".to_string(),
            provider: "codex".to_string(),
            context_window: 272_000,
            max_output_tokens: 128_000,
            input_limit: None,
            thinking_budgets: None,
        };

        // The model accepts up to 128k output, but the router's configured
        // request cap is 32k. Context assembly must reserve the amount the
        // router will actually request, not the catalog ceiling.
        let resolved = ResolvedModel::from_router(&model, 32_000);
        assert_eq!(resolved.limits.max_output_tokens, 32_000);
        assert_eq!(usable(&resolved.limits, 20_000, 0), 220_000);
    }

    #[test]
    fn thinking_budget_requires_both_level_and_declaration() {
        let inline = ResolvedModel::from_inline(limits(100, 10, None));
        assert_eq!(inline.thinking_budget(Some(ThinkingLevel::High)), 0);

        let mut budgets = std::collections::BTreeMap::new();
        budgets.insert(ThinkingLevel::High, 9_000);
        let routed = ResolvedModel {
            limits: limits(100, 10, None),
            thinking_budgets: Some(budgets),
            resolved: ModelResolved::Router,
        };
        assert_eq!(routed.thinking_budget(Some(ThinkingLevel::High)), 9_000);
        assert_eq!(routed.thinking_budget(Some(ThinkingLevel::Low)), 0);
        assert_eq!(routed.thinking_budget(None), 0);
    }

    #[test]
    fn preserve_recent_budget_clamps_and_overrides() {
        assert_eq!(preserve_recent_budget(40_000, None), 8_000); // 10k capped
        assert_eq!(preserve_recent_budget(20_000, None), 5_000); // within band
        assert_eq!(preserve_recent_budget(1_000, None), 2_000); // floor
        assert_eq!(preserve_recent_budget(40_000, Some(123)), 123);
    }
}
