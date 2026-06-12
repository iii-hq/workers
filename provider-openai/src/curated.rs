//! Curated capability snapshot for the OpenAI catalog slice, and the
//! live-discovery merge. The static slice doubles as the declaration's
//! `models` so the catalog has no cold hole before first discovery.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};
use std::collections::HashSet;

/// One live `GET /v1/models` row (OpenAI rows carry no display name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStub {
    pub id: String,
}

fn model(
    id: &str,
    display_name: &str,
    context_window: u64,
    max_output_tokens: u64,
    xhigh: bool,
    pricing: Pricing,
) -> Model {
    Model {
        id: id.into(),
        provider: PROVIDER_ID.into(),
        display_name: Some(display_name.into()),
        context_window,
        max_output_tokens,
        input_limit: None,
        supports_thinking: Some(true),
        supports_xhigh: Some(xhigh),
        supports_tools: Some(true),
        supports_vision: Some(true),
        supports_cache: Some(true),
        supports_structured_output: Some(true), // native json_schema mode
        thinking_budgets: None,                 // effort enum, not token budgets
        pricing: Some(pricing),
    }
}

fn price(input: f64, output: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(input * 0.1),
        cache_write: None, // automatic caching; no write surcharge
    }
}

/// The hand-maintained snapshot (USD per MTok). Verify against models.dev
/// before release; stale records degrade preflight sizing and cost fill.
pub fn static_models() -> Vec<Model> {
    vec![
        model(
            "gpt-5.2",
            "GPT-5.2",
            400_000,
            128_000,
            true,
            price(1.75, 14.0),
        ),
        model(
            "gpt-5.1",
            "GPT-5.1",
            400_000,
            128_000,
            false,
            price(1.25, 10.0),
        ),
        model(
            "gpt-5-mini",
            "GPT-5 Mini",
            400_000,
            128_000,
            false,
            price(0.25, 2.0),
        ),
        model(
            "gpt-5-nano",
            "GPT-5 Nano",
            400_000,
            128_000,
            false,
            price(0.05, 0.40),
        ),
    ]
}

/// Curated lookup for request shaping when the router forwarded no
/// model_meta and the catalog has no record.
pub fn find(model_id: &str) -> Option<Model> {
    let base = base_id(model_id);
    static_models()
        .into_iter()
        .find(|m| m.id == model_id || m.id == base)
}

/// Strip a trailing `-YYYY-MM-DD` date suffix
/// (`gpt-5.1-2025-11-13` → `gpt-5.1`). OpenAI dates use hyphenated ISO form,
/// unlike Anthropic's compact `-YYYYMMDD`.
fn base_id(id: &str) -> &str {
    if id.len() > 11 {
        let (head, tail) = id.split_at(id.len() - 11);
        let bytes = tail.as_bytes();
        let shape_ok = bytes[0] == b'-'
            && bytes[5] == b'-'
            && bytes[8] == b'-'
            && tail
                .char_indices()
                .all(|(i, c)| matches!(i, 0 | 5 | 8) || c.is_ascii_digit());
        if shape_ok {
            return head;
        }
    }
    id
}

/// Live ids ∪ curated metadata (spec § Catalog metadata sources): every live
/// id gets curated capabilities when its base id matches, conservative
/// defaults otherwise; curated entries missing from the live list are kept
/// (aliases the API doesn't enumerate).
pub fn merge_with_live(live: &[LiveStub]) -> Vec<Model> {
    let curated = static_models();
    let mut out: Vec<Model> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();

    for stub in live {
        let base = base_id(&stub.id);
        match curated.iter().find(|m| m.id == stub.id || m.id == base) {
            Some(c) => {
                covered.insert(c.id.clone());
                let mut m = c.clone();
                m.id = stub.id.clone();
                out.push(m);
            }
            None => out.push(unknown_model(stub)),
        }
    }
    for c in curated {
        if !covered.contains(&c.id) && !out.iter().any(|m| m.id == c.id) {
            out.push(c);
        }
    }
    out
}

/// Conservative record for a live id with no curated metadata. Tools and
/// automatic caching are uniform across the chat families we admit;
/// vision/thinking/structured-output stay unknown.
fn unknown_model(stub: &LiveStub) -> Model {
    Model {
        id: stub.id.clone(),
        provider: PROVIDER_ID.into(),
        display_name: None,
        context_window: 128_000,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_well_formed() {
        let models = static_models();
        assert_eq!(models.len(), 4);
        for m in &models {
            assert_eq!(m.provider, "openai");
            assert_eq!(m.supports_structured_output, Some(true));
            assert_eq!(m.supports_thinking, Some(true));
            assert!(
                m.thinking_budgets.is_none(),
                "OpenAI uses efforts, not budgets"
            );
            let pricing = m.pricing.as_ref().unwrap();
            assert!(pricing.input.is_some() && pricing.output.is_some());
            assert!(pricing.cache_write.is_none(), "no cache-write surcharge");
            assert!(m.context_window > 0 && m.max_output_tokens > 0);
        }
        // xhigh is the gpt-5.2+ tier only (matches the Task 6 effort ladder)
        assert_eq!(
            models
                .iter()
                .filter(|m| m.supports_xhigh == Some(true))
                .count(),
            1
        );
    }

    #[test]
    fn find_matches_dated_ids_via_base() {
        assert_eq!(find("gpt-5.1").unwrap().id, "gpt-5.1");
        assert_eq!(find("gpt-5.1-2025-11-13").unwrap().id, "gpt-5.1");
        assert!(find("claude-sonnet-4-6").is_none());
        assert!(
            find("gpt-5.1-turbo-extra").is_none(),
            "non-date suffixes don't match"
        );
    }

    #[test]
    fn merge_enriches_live_ids_and_keeps_curated_aliases() {
        let live = vec![
            LiveStub {
                id: "gpt-5.1-2025-11-13".into(),
            },
            LiveStub {
                id: "o3-mini".into(),
            },
        ];
        let merged = merge_with_live(&live);
        // live dated id carries curated caps under the live id
        let dated = merged
            .iter()
            .find(|m| m.id == "gpt-5.1-2025-11-13")
            .unwrap();
        assert_eq!(dated.context_window, 400_000);
        assert_eq!(dated.supports_structured_output, Some(true));
        // unknown live id gets conservative defaults
        let mystery = merged.iter().find(|m| m.id == "o3-mini").unwrap();
        assert_eq!(mystery.max_output_tokens, 16_384);
        assert!(mystery.pricing.is_none());
        // curated entries not in the live list are kept
        assert!(merged.iter().any(|m| m.id == "gpt-5.2"));
        assert!(merged.iter().any(|m| m.id == "gpt-5-mini"));
        // the covered base id is NOT duplicated
        assert!(!merged.iter().any(|m| m.id == "gpt-5.1"));
    }

    #[test]
    fn merge_with_empty_live_list_returns_full_snapshot() {
        assert_eq!(merge_with_live(&[]).len(), 4);
    }
}
