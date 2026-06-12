//! The only hand-maintained catalog data: pricing. Everything else — model
//! ids, display names, context/output limits, capability flags — comes live
//! from `GET /v1/models` (see discovery.rs); the API does not expose pricing,
//! so this table fills that one gap.
use llm_router::types::model::Pricing;

fn price(input: f64, output: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(input * 0.1),
        cache_write: Some(input * 1.25),
    }
}

/// USD per MTok by base model id. Verify against anthropic.com/pricing
/// before release; a missing row only degrades cost enrichment, never
/// routing or capability gating.
pub fn pricing_for(model_id: &str) -> Option<Pricing> {
    match base_id(model_id) {
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some(price(5.0, 25.0)),
        "claude-sonnet-4-6" | "claude-sonnet-4-5" => Some(price(3.0, 15.0)),
        "claude-haiku-4-5" => Some(price(1.0, 5.0)),
        "claude-fable-5" | "claude-mythos-5" => Some(price(10.0, 50.0)),
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
    fn base_id_strips_only_date_suffixes() {
        assert_eq!(base_id("claude-sonnet-4-6-20260115"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-sonnet-4-6"), "claude-sonnet-4-6");
        assert_eq!(base_id("claude-haiku-4-5"), "claude-haiku-4-5");
    }
}
