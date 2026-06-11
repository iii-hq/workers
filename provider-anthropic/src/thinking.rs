//! thinking_level → Messages API `thinking` field, with budgets from the
//! catalog (`thinking_budgets`) when present and a formula on the model's
//! max output tokens as fallback.
use llm_router::types::model::{Model, ThinkingLevel};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub mode: &'static str, // always "enabled"
    pub budget_tokens: u64,
}

/// Anthropic's documented minimum thinking budget.
pub const MIN_THINKING_BUDGET: u64 = 1024;

/// Tokens reserved for the visible answer: the budget counts toward
/// max_tokens, and an xhigh budget without a reserve can consume the whole
/// request budget and silently return an empty completion.
pub const OUTPUT_RESERVE_TOKENS: u64 = 1024;

/// Unlike the TS port (whose `ThinkingBudgets` had no xhigh field), an
/// `Xhigh` catalog entry is honored when present.
fn budget_from_catalog(
    level: ThinkingLevel,
    budgets: Option<&BTreeMap<ThinkingLevel, u64>>,
) -> Option<u64> {
    budgets?.get(&level).copied()
}

/// Budget formula on the model's max output tokens, used when the catalog
/// carries no thinking_budgets for the tier.
fn budget_from_formula(level: ThinkingLevel, output: u64) -> u64 {
    match level {
        ThinkingLevel::Xhigh => 31_999.min(output.saturating_sub(1)),
        ThinkingLevel::High => 16_000.min((output / 2).saturating_sub(1)),
        ThinkingLevel::Medium => 8_000.min(output / 4),
        ThinkingLevel::Low | ThinkingLevel::Minimal => 4_000.min(output / 8),
    }
}

pub fn build_thinking_config(
    level: Option<ThinkingLevel>,
    max_tokens: u64,
    model: Option<&Model>,
) -> Option<ThinkingConfig> {
    let level = level?;
    // An explicit `supports_thinking: false` would 400; unknown stays permissive.
    if model.and_then(|m| m.supports_thinking) == Some(false) {
        return None;
    }
    let effective =
        if level == ThinkingLevel::Xhigh && model.and_then(|m| m.supports_xhigh) == Some(false) {
            ThinkingLevel::High
        } else {
            level
        };

    let mut budget =
        budget_from_catalog(effective, model.and_then(|m| m.thinking_budgets.as_ref()));
    if budget.is_none_or(|b| b == 0) {
        // The formula needs the model's output ceiling; unknown model → no thinking.
        let output = model.map(|m| m.max_output_tokens).filter(|&o| o > 0)?;
        budget = Some(budget_from_formula(effective, output));
    }
    let budget = budget.filter(|&b| b > 0)?;

    // Keep the budget below max_tokens with room for the visible answer.
    let budget = budget.min(max_tokens.saturating_sub(OUTPUT_RESERVE_TOKENS));
    if budget < MIN_THINKING_BUDGET {
        return None;
    }
    Some(ThinkingConfig {
        mode: "enabled",
        budget_tokens: budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(budgets: Option<BTreeMap<ThinkingLevel, u64>>, xhigh: Option<bool>) -> Model {
        Model {
            id: "claude-test".into(),
            provider: "anthropic".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: xhigh,
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: Some(true),
            supports_structured_output: Some(false),
            thinking_budgets: budgets,
            pricing: None,
        }
    }

    #[test]
    fn absent_level_means_off() {
        assert_eq!(
            build_thinking_config(None, 32_000, Some(&model(None, None))),
            None
        );
    }

    #[test]
    fn catalog_budget_wins_over_formula() {
        let budgets = BTreeMap::from([(ThinkingLevel::High, 12_345u64)]);
        let cfg = build_thinking_config(
            Some(ThinkingLevel::High),
            32_000,
            Some(&model(Some(budgets), None)),
        )
        .unwrap();
        assert_eq!(cfg.budget_tokens, 12_345);
        assert_eq!(cfg.mode, "enabled");
    }

    #[test]
    fn formula_fallback_per_tier() {
        let m = model(None, None); // max_output_tokens 64k
        let high = build_thinking_config(Some(ThinkingLevel::High), 64_000, Some(&m)).unwrap();
        assert_eq!(high.budget_tokens, 16_000); // min(16k, 64k/2-1)
        let med = build_thinking_config(Some(ThinkingLevel::Medium), 64_000, Some(&m)).unwrap();
        assert_eq!(med.budget_tokens, 8_000);
        let low = build_thinking_config(Some(ThinkingLevel::Low), 64_000, Some(&m)).unwrap();
        assert_eq!(low.budget_tokens, 4_000);
        let xhigh = build_thinking_config(Some(ThinkingLevel::Xhigh), 64_000, Some(&m)).unwrap();
        assert_eq!(xhigh.budget_tokens, 31_999);
    }

    #[test]
    fn xhigh_degrades_to_high_when_unsupported() {
        let m = model(None, Some(false));
        let cfg = build_thinking_config(Some(ThinkingLevel::Xhigh), 64_000, Some(&m)).unwrap();
        assert_eq!(cfg.budget_tokens, 16_000); // the high tier
    }

    #[test]
    fn clamped_below_minimum_drops_thinking() {
        let m = model(None, None);
        // max_tokens 2000 → budget clamp to 2000-1024=976 < 1024 → dropped
        assert_eq!(
            build_thinking_config(Some(ThinkingLevel::High), 2_000, Some(&m)),
            None
        );
    }

    #[test]
    fn explicit_no_thinking_support_drops() {
        let mut m = model(None, None);
        m.supports_thinking = Some(false);
        assert_eq!(
            build_thinking_config(Some(ThinkingLevel::High), 64_000, Some(&m)),
            None
        );
    }

    #[test]
    fn unknown_model_drops_thinking() {
        assert_eq!(
            build_thinking_config(Some(ThinkingLevel::High), 64_000, None),
            None
        );
    }

    #[test]
    fn xhigh_catalog_entry_is_honored_when_supported() {
        let budgets = BTreeMap::from([(ThinkingLevel::Xhigh, 20_000u64)]);
        // supports_xhigh unset (None) → not degraded; catalog value beats the formula
        let cfg = build_thinking_config(
            Some(ThinkingLevel::Xhigh),
            64_000,
            Some(&model(Some(budgets), None)),
        )
        .unwrap();
        assert_eq!(cfg.budget_tokens, 20_000);
    }

    #[test]
    fn budget_exactly_at_minimum_is_kept() {
        let m = model(None, None);
        // max_tokens 2048 → formula high = 16000, clamped to 2048-1024 = 1024 == MIN → kept
        let cfg = build_thinking_config(Some(ThinkingLevel::High), 2_048, Some(&m)).unwrap();
        assert_eq!(cfg.budget_tokens, MIN_THINKING_BUDGET);
    }
}
