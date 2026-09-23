//! The only hand-maintained catalog data: pricing. Everything else — model
//! ids, display names, context/output limits, capability flags — comes live
//! from `GET /v1/models` (see discovery.rs); the API does not expose pricing,
//! so this table fills that one gap.
//!
//! A model missing here still routes and appears in the picker; it only
//! loses cost enrichment, and the harness refuses `max_cost_usd` budgets on
//! it ("no pricing is configured in the model catalog"). Add a row whenever
//! Anthropic ships a model.
use llm_router::types::model::Pricing;

/// Standard cache-read multiplier ("cache hits and refreshes" at 0.1x input).
const STANDARD_CACHE_READ: f64 = 0.1;
/// 5-minute cache write is 1.25x input on every model.
const CACHE_WRITE_5M: f64 = 1.25;

fn price(input: f64, output: f64) -> Pricing {
    price_with_cache_read(input, output, STANDARD_CACHE_READ)
}

/// Some generations discount cache reads further than the standard 0.1x
/// (Opus 5.5: 0.05x; Fable 5.1 / Mythos 5.1: 0.025x).
fn price_with_cache_read(input: f64, output: f64, cache_read_multiplier: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(input * cache_read_multiplier),
        cache_write: Some(input * CACHE_WRITE_5M),
    }
}

/// USD per MTok by base model id. Source of truth:
/// <https://platform.claude.com/docs/en/about-claude/pricing#model-pricing>
/// (verified 2026-09). A missing row only degrades cost enrichment, never
/// routing or capability gating.
pub fn pricing_for(model_id: &str) -> Option<Pricing> {
    match base_id(model_id) {
        // Frontier tier. The 5.1 generation reads cache at 0.025x.
        "claude-fable-5-1" | "claude-mythos-5-1" => Some(price_with_cache_read(10.0, 50.0, 0.025)),
        "claude-fable-5" | "claude-mythos-5" => Some(price(10.0, 50.0)),
        // Opus 5.5 dropped to $4/$20 and reads cache at 0.05x.
        "claude-opus-5-5" => Some(price_with_cache_read(4.0, 20.0, 0.05)),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6"
        | "claude-opus-4-5" => Some(price(5.0, 25.0)),
        // Sonnet 5 launched at $2/$10 introductory pricing, now standard.
        "claude-sonnet-5" => Some(price(2.0, 10.0)),
        "claude-sonnet-4-6" | "claude-sonnet-4-5" => Some(price(3.0, 15.0)),
        "claude-haiku-4-5" => Some(price(1.0, 5.0)),
        _ => None,
    }
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

    fn approx(actual: Option<f64>, expected: f64) -> bool {
        actual.is_some_and(|v| (v - expected).abs() < 1e-9)
    }

    #[test]
    fn pricing_matches_dated_ids_via_base() {
        assert_eq!(pricing_for("claude-sonnet-4-6").unwrap().input, Some(3.0));
        assert_eq!(
            pricing_for("claude-sonnet-4-6-20260115").unwrap().output,
            Some(15.0)
        );
        assert_eq!(pricing_for("claude-opus-4-8").unwrap().input, Some(5.0));
        assert!(pricing_for("claude-mystery-9").is_none());
    }

    #[test]
    fn generation_5_rows_are_priced() {
        // Every row the live catalog currently returns must price out;
        // a hole here silently disables max_cost_usd for that model.
        for id in [
            "claude-opus-5-5",
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5-1",
            "claude-fable-5",
            "claude-mythos-5-1",
            "claude-mythos-5",
            "claude-opus-4-5",
        ] {
            assert!(pricing_for(id).is_some(), "{id} has no pricing row");
        }
        let opus_5_5 = pricing_for("claude-opus-5-5").unwrap();
        assert_eq!(opus_5_5.input, Some(4.0));
        assert_eq!(opus_5_5.output, Some(20.0));
        let opus_5 = pricing_for("claude-opus-5").unwrap();
        assert_eq!((opus_5.input, opus_5.output), (Some(5.0), Some(25.0)));
        let sonnet_5 = pricing_for("claude-sonnet-5").unwrap();
        assert_eq!((sonnet_5.input, sonnet_5.output), (Some(2.0), Some(10.0)));
        let fable_5_1 = pricing_for("claude-fable-5-1").unwrap();
        assert_eq!(
            (fable_5_1.input, fable_5_1.output),
            (Some(10.0), Some(50.0))
        );
    }

    #[test]
    fn cache_multipliers_follow_the_published_table() {
        // Standard: 0.1x read, 1.25x 5-minute write.
        let sonnet = pricing_for("claude-sonnet-4-6").unwrap();
        assert!(approx(sonnet.cache_read, 0.30));
        assert!(approx(sonnet.cache_write, 3.75));
        // Opus 5.5: 0.05x read ($0.20), write still 1.25x ($5).
        let opus_5_5 = pricing_for("claude-opus-5-5").unwrap();
        assert!(approx(opus_5_5.cache_read, 0.20));
        assert!(approx(opus_5_5.cache_write, 5.0));
        // Fable 5.1 / Mythos 5.1: 0.025x read ($0.25); Fable 5 stays 0.1x ($1).
        assert!(approx(
            pricing_for("claude-fable-5-1").unwrap().cache_read,
            0.25
        ));
        assert!(approx(
            pricing_for("claude-mythos-5-1").unwrap().cache_read,
            0.25
        ));
        assert!(approx(
            pricing_for("claude-fable-5").unwrap().cache_read,
            1.0
        ));
    }

    #[test]
    fn base_id_strips_only_date_suffixes() {
        assert_eq!(base_id("claude-sonnet-4-6-20260115"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-sonnet-4-6"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-haiku-4-5"), "claude-haiku-4-5");
        // A minor-version tail is not a date: `-5-5` must survive.
        assert_eq!(base_id("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(base_id("claude-opus-5-5-20260901"), "claude-opus-5-5");
    }
}
