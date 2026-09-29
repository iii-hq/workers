//! The only hand-maintained catalog data: pricing. Everything else — model
//! ids, display names, context/output limits, capability flags — comes live
//! from `GET /v1/models` (see discovery.rs); the API does not expose pricing,
//! so this table fills that one gap.
//!
//! Missing prices leave cost enrichment unavailable and prevent
//! `max_cost_usd` budgets; they never hide a model from the live catalog.
use llm_router::chat::pricing::fill_cost_usd;
use llm_router::types::events::Usage;
use llm_router::types::model::Pricing;

/// Cache reads are priced per model (0.1x input on most, deeper on newer
/// ones); cache writes use the 5-minute rate, 1.25x input.
fn price(input: f64, output: f64, cache_read: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(cache_read),
        cache_write: Some(input * 1.25),
    }
}

/// USD per MTok by base model id. Verify against anthropic.com/pricing
/// before release; a missing row leaves `cost_usd` unset, which nulls cost
/// rollups and refuses `max_cost_usd` sends, but never affects routing or
/// capability gating.
pub fn pricing_for(model_id: &str) -> Option<Pricing> {
    match base_id(model_id) {
        "claude-opus-5-5" => Some(price(4.0, 20.0, 0.20)),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6"
        | "claude-opus-4-5" => Some(price(5.0, 25.0, 0.50)),
        "claude-sonnet-5" => Some(price(2.0, 10.0, 0.20)),
        "claude-sonnet-4-6" | "claude-sonnet-4-5" => Some(price(3.0, 15.0, 0.30)),
        "claude-haiku-4-5" => Some(price(1.0, 5.0, 0.10)),
        // The Mythos 5.1 cache-read rate was unannounced at launch; it is
        // assumed to match Fable 5.1.
        "claude-fable-5-1" | "claude-mythos-5-1" => Some(price(10.0, 50.0, 0.25)),
        "claude-fable-5" | "claude-mythos-5" => Some(price(10.0, 50.0, 1.0)),
        _ => None,
    }
}

/// `cost_usd` for usage whose cache writes include `cache_write_1h` tokens
/// written with the 1-hour TTL (the shared prefix, see wire/cache.rs). Those
/// bill at 2x input, which the catalog's single 5-minute `cache_write` rate
/// cannot express. `None` when there were no 1-hour writes or the model has
/// no price, leaving the router to price the usage from the catalog.
pub fn cost_with_1h_cache_writes(
    model_id: &str,
    usage: &Usage,
    cache_write_1h: u64,
) -> Option<f64> {
    if cache_write_1h == 0 {
        return None;
    }
    let pricing = pricing_for(model_id)?;
    let input = pricing.input?;
    let five_minute = Usage {
        cache_write: usage.cache_write.map(|w| w.saturating_sub(cache_write_1h)),
        cost_usd: None,
        ..usage.clone()
    };
    let rest = fill_cost_usd(&five_minute, Some(&pricing))
        .cost_usd
        .unwrap_or(0.0);
    Some(rest + cache_write_1h as f64 * input * 2.0 / 1_000_000.0)
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
    fn current_models_carry_their_own_cache_read_rate() {
        let expect = |input, output, cache_read, cache_write| Pricing {
            input: Some(input),
            output: Some(output),
            cache_read: Some(cache_read),
            cache_write: Some(cache_write),
        };
        for (id, pricing) in [
            ("claude-opus-5-5", expect(4.0, 20.0, 0.20, 5.0)),
            ("claude-opus-5-5-20260901", expect(4.0, 20.0, 0.20, 5.0)),
            ("claude-opus-5", expect(5.0, 25.0, 0.50, 6.25)),
            ("claude-opus-4-5", expect(5.0, 25.0, 0.50, 6.25)),
            ("claude-sonnet-4-6", expect(3.0, 15.0, 0.30, 3.75)),
            ("claude-sonnet-5", expect(2.0, 10.0, 0.20, 2.5)),
            ("claude-fable-5-1", expect(10.0, 50.0, 0.25, 12.5)),
            ("claude-fable-5", expect(10.0, 50.0, 1.0, 12.5)),
            ("claude-mythos-5-1", expect(10.0, 50.0, 0.25, 12.5)),
            ("claude-mythos-5", expect(10.0, 50.0, 1.0, 12.5)),
        ] {
            assert_eq!(pricing_for(id), Some(pricing), "{id}");
        }
    }

    fn opus_usage() -> Usage {
        Usage {
            input: Some(10),
            output: Some(20),
            cache_read: Some(1_000),
            cache_write: Some(1_100),
            ..Usage::default()
        }
    }

    #[test]
    fn one_hour_cache_writes_bill_at_twice_input() {
        // 10*4 + 20*20 + 1000*0.20 + 100*5 (5m) + 1000*8 (1h), per MTok.
        let cost = cost_with_1h_cache_writes("claude-opus-5-5", &opus_usage(), 1_000).unwrap();
        assert!((cost - 9_140.0 / 1_000_000.0).abs() < 1e-12, "{cost}");
    }

    #[test]
    fn without_one_hour_writes_or_a_price_the_router_prices_it() {
        assert_eq!(
            cost_with_1h_cache_writes("claude-opus-5-5", &opus_usage(), 0),
            None
        );
        assert_eq!(
            cost_with_1h_cache_writes("claude-unknown", &opus_usage(), 1_000),
            None
        );
    }

    #[test]
    fn base_id_strips_only_date_suffixes() {
        assert_eq!(base_id("claude-sonnet-4-6-20260115"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-sonnet-4-6"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-haiku-4-5"), "claude-haiku-4-5");
        assert_eq!(base_id("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(base_id("claude-opus-5-5-20260901"), "claude-opus-5-5");
    }
}
