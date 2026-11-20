//! Local catalog metadata for the Gemini slice. `GET /models` gives the ids,
//! limits and the thinking flag (discovery.rs); this module adds what the
//! list cannot give: pricing, and the thinking levels from `reasoning.rs`.
//!
//! Prices: USD per million tokens, from ai.google.dev pricing, read
//! 2026-11-19. The catalog has one price per model and no time dimension, so
//! each row ships one figure; rows are updated in a normal PR when a price
//! changes.
use crate::PROVIDER_ID;
use crate::reasoning::{longest_row, supported_levels, wire_level};
use llm_router::types::model::{Model, Pricing, ReasoningEffort, ThinkingLevel};

/// Context window and max output for ids the live list does not describe.
const UNKNOWN_CONTEXT_WINDOW: u64 = 1_048_576;
const UNKNOWN_MAX_OUTPUT_TOKENS: u64 = 65_536;

/// (input, cached input, output). A row matches its id and any id that
/// continues it with `-`; the longest row wins.
// shortcut: gemini-3.1-pro-preview ships its up-to-200k tier only; the catalog
// has no per-tier price.
const PRICES: &[(&str, (f64, f64, f64))] = &[
    // The rate from 2026-12-01; 2.00 / 0.10 / 10.00 through 2026-11-30.
    // The later figure does not under-report a bill.
    ("gemini-4-carbon", (4.00, 0.20, 20.00)),
    ("gemini-4-argon", (4.00, 0.20, 20.00)),
    ("gemini-4-neon", (3.00, 0.15, 15.00)),
    ("gemini-4-hydrogen", (1.50, 0.075, 7.50)),
    // Rises to 1.50 / 0.15 / 7.50 on 2027-01-01; update the rows then.
    ("gemini-3.8-flash", (0.75, 0.075, 3.75)),
    ("gemini-3.6-flash", (0.75, 0.075, 3.75)),
    ("gemini-3.5-flash-lite", (0.30, 0.03, 2.50)),
    // Also matches gemini-3.1-flash-lite-preview.
    ("gemini-3.1-flash-lite", (0.25, 0.025, 1.50)),
    // Also matches gemini-3.1-pro-preview-customtools.
    ("gemini-3.1-pro-preview", (2.00, 0.20, 12.00)),
    ("gemini-3-flash-preview", (0.50, 0.05, 3.00)),
];

/// Auto-routed ids → the model that answers and bills. Pricing only.
fn canonical(id: &str) -> &str {
    match id {
        "gemini-3.5-flash" => "gemini-3.6-flash",
        "gemini-3.7-flash" => "gemini-3.8-flash",
        other => other,
    }
}

fn pricing(id: &str) -> Option<Pricing> {
    longest_row(PRICES, canonical(id)).map(|&(input, cached, output)| Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(cached),
        cache_write: None, // implicit caching has no write charge
    })
}

/// One id → catalog Model with the defaults `discovery` overrides from the
/// live row.
pub fn enrich(id: &str) -> Model {
    let levels = supported_levels(id);
    Model {
        id: id.into(),
        provider: PROVIDER_ID.into(),
        display_name: None,
        context_window: UNKNOWN_CONTEXT_WINDOW,
        max_output_tokens: UNKNOWN_MAX_OUTPUT_TOKENS,
        input_limit: None,
        supports_thinking: Some(true),
        supports_xhigh: Some(levels.contains(&ThinkingLevel::Xhigh)),
        // No off switch exists in the Interactions schema.
        supports_thinking_off: Some(false),
        reasoning_efforts: Some(
            levels
                .iter()
                .map(|&l| ReasoningEffort {
                    effort: wire_level(l).into(),
                    description: None,
                })
                .collect(),
        ),
        supports_tools: Some(true),
        supports_vision: Some(true),
        supports_cache: Some(true),
        supports_structured_output: Some(true),
        thinking_budgets: None,
        pricing: pricing(id),
        speech: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(id: &str) -> Option<(f64, f64, f64)> {
        pricing(id).map(|p| (p.input.unwrap(), p.cache_read.unwrap(), p.output.unwrap()))
    }

    #[test]
    fn price_match_by_prefix_and_canonical_ids() {
        assert_eq!(price("gemini-4-carbon"), Some((4.00, 0.20, 20.00)));
        assert_eq!(
            price("gemini-3.1-flash-lite-preview"),
            Some((0.25, 0.025, 1.50))
        );
        assert_eq!(
            price("gemini-3.1-pro-preview-customtools"),
            Some((2.00, 0.20, 12.00))
        );
        assert_eq!(price("gemini-3.5-flash"), price("gemini-3.6-flash"));
        assert_eq!(price("gemini-3.7-flash"), price("gemini-3.8-flash"));
        assert_eq!(price("gemini-3.5-flash-lite"), Some((0.30, 0.03, 2.50)));
        assert_eq!(price("gemini-flash-latest"), None);
        assert_eq!(price("gemini-3.8-flashy"), None);
    }

    #[test]
    fn thinking_flags_follow_the_level_table() {
        let m = enrich("gemini-4-hydrogen");
        assert_eq!(m.supports_xhigh, Some(true));
        assert_eq!(m.supports_thinking_off, Some(false));
        let efforts: Vec<String> = m
            .reasoning_efforts
            .unwrap()
            .into_iter()
            .map(|e| e.effort)
            .collect();
        assert_eq!(efforts, ["low", "medium", "high", "xhigh"]);
        let m = enrich("gemini-3.6-flash");
        assert_eq!(m.supports_xhigh, Some(false));
        assert_eq!(m.reasoning_efforts.unwrap()[0].effort, "minimal");
    }
}
