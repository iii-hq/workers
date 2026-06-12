//! cost_usd fill from catalog pricing (spec § router::chat). Never mutates.
use crate::types::events::Usage;
use crate::types::model::Pricing;

const PER_TOKENS: f64 = 1_000_000.0; // pricing fields are USD per million tokens

pub fn fill_cost_usd(usage: &Usage, pricing: Option<&Pricing>) -> Usage {
    let mut out = usage.clone();
    let Some(p) = pricing else { return out };
    if out.cost_usd.is_some() {
        return out; // the provider's own figure wins
    }
    let parts = [
        (usage.input, p.input),
        (usage.output, p.output),
        (usage.cache_read, p.cache_read),
        (usage.cache_write, p.cache_write),
    ];
    let mut cost = 0.0;
    let mut any = false;
    for (tokens, rate) in parts {
        if let (Some(t), Some(r)) = (tokens, rate) {
            cost += (t as f64) * r / PER_TOKENS;
            any = true;
        }
    }
    if any {
        out.cost_usd = Some(cost);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::events::Usage;
    use crate::types::model::Pricing;

    fn pricing() -> Pricing {
        Pricing {
            input: Some(3.0),
            output: Some(15.0),
            cache_read: Some(0.3),
            cache_write: Some(3.75),
        }
    }

    #[test]
    fn computes_cost_from_counts_and_per_million_rates() {
        let usage = fill_cost_usd(
            &Usage {
                input: Some(1_000_000),
                output: Some(100_000),
                ..Default::default()
            },
            Some(&pricing()),
        );
        assert!((usage.cost_usd.unwrap() - 4.5).abs() < 1e-9);
    }

    #[test]
    fn never_overwrites_provider_cost_and_tolerates_missing_pricing() {
        let reported = Usage {
            input: Some(100),
            cost_usd: Some(1.23),
            ..Default::default()
        };
        assert_eq!(
            fill_cost_usd(&reported, Some(&pricing())).cost_usd,
            Some(1.23)
        );
        assert_eq!(fill_cost_usd(&reported, None).cost_usd, Some(1.23));
        assert_eq!(
            fill_cost_usd(&Usage::default(), Some(&pricing())).cost_usd,
            None
        );
    }
}
