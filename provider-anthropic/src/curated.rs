//! Curated capability snapshot for the Anthropic catalog slice, and the
//! live-discovery merge. The static slice doubles as the declaration's
//! `models` so the catalog has no cold hole before first discovery.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing, ThinkingLevel};
use std::collections::{BTreeMap, HashSet};

/// One live `GET /v1/models` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStub {
    pub id: String,
    pub display_name: Option<String>,
}

fn budgets(xhigh: bool) -> BTreeMap<ThinkingLevel, u64> {
    let mut b = BTreeMap::from([
        (ThinkingLevel::Minimal, 1024),
        (ThinkingLevel::Low, 4096),
        (ThinkingLevel::Medium, 8192),
        (ThinkingLevel::High, 16_384),
    ]);
    if xhigh {
        b.insert(ThinkingLevel::Xhigh, 31_999);
    }
    b
}

fn model(
    id: &str,
    display_name: &str,
    context_window: u64,
    max_output_tokens: u64,
    thinking: bool,
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
        supports_thinking: Some(thinking),
        supports_xhigh: Some(xhigh),
        supports_tools: Some(true),
        supports_vision: Some(true),
        supports_cache: Some(true),
        supports_structured_output: Some(false), // no native JSON mode
        thinking_budgets: thinking.then(|| budgets(xhigh)),
        pricing: Some(pricing),
    }
}

fn price(input: f64, output: f64) -> Pricing {
    Pricing {
        input: Some(input),
        output: Some(output),
        cache_read: Some(input * 0.1),
        cache_write: Some(input * 1.25),
    }
}

/// The hand-maintained snapshot (USD per MTok). Verify against models.dev
/// before release; stale records degrade preflight sizing and cost fill.
pub fn static_models() -> Vec<Model> {
    vec![
        model(
            "claude-opus-4-7",
            "Claude Opus 4.7",
            1_000_000,
            32_000,
            true,
            true,
            price(5.0, 25.0),
        ),
        model(
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            200_000,
            64_000,
            true,
            false,
            price(3.0, 15.0),
        ),
        model(
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            200_000,
            16_000,
            false,
            false,
            price(1.0, 5.0),
        ),
    ]
}

/// Curated lookup for request shaping (thinking budgets) when the router
/// forwarded no model_meta and the catalog has no record.
pub fn find(model_id: &str) -> Option<Model> {
    let base = base_id(model_id);
    static_models()
        .into_iter()
        .find(|m| m.id == model_id || m.id == base)
}

/// Strip a trailing `-YYYYMMDD` date suffix
/// (`claude-sonnet-4-6-20260115` → `claude-sonnet-4-6`).
fn base_id(id: &str) -> &str {
    if id.len() > 9 {
        let (head, tail) = id.split_at(id.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
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
                if stub.display_name.is_some() {
                    m.display_name = stub.display_name.clone();
                }
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

/// Conservative record for a live id with no curated metadata. Capability
/// flags stay permissive-but-honest: tools/cache yes (uniform across the
/// Messages API), vision unknown, structured output never.
fn unknown_model(stub: &LiveStub) -> Model {
    Model {
        id: stub.id.clone(),
        provider: PROVIDER_ID.into(),
        display_name: stub.display_name.clone(),
        context_window: 200_000,
        max_output_tokens: 8192,
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        supports_tools: Some(true),
        supports_vision: None,
        supports_cache: Some(true),
        supports_structured_output: Some(false),
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
        assert_eq!(models.len(), 3);
        for m in &models {
            assert_eq!(m.provider, "anthropic");
            assert_eq!(m.supports_structured_output, Some(false));
            assert!(m.pricing.as_ref().unwrap().input.is_some());
            assert!(m.context_window > 0 && m.max_output_tokens > 0);
            if m.supports_thinking == Some(true) {
                assert!(m.thinking_budgets.is_some());
            }
        }
        assert!(models.iter().any(|m| m.supports_xhigh == Some(true)));
    }

    #[test]
    fn find_matches_dated_ids_via_base() {
        assert_eq!(find("claude-sonnet-4-6").unwrap().id, "claude-sonnet-4-6");
        assert_eq!(
            find("claude-sonnet-4-6-20260115").unwrap().id,
            "claude-sonnet-4-6"
        );
        assert!(find("gpt-4o").is_none());
    }

    #[test]
    fn merge_enriches_live_ids_and_keeps_curated_aliases() {
        let live = vec![
            LiveStub {
                id: "claude-sonnet-4-6-20260115".into(),
                display_name: Some("Sonnet live".into()),
            },
            LiveStub {
                id: "claude-mystery-9".into(),
                display_name: None,
            },
        ];
        let merged = merge_with_live(&live);
        // live dated id carries curated caps under the live id
        let sonnet = merged
            .iter()
            .find(|m| m.id == "claude-sonnet-4-6-20260115")
            .unwrap();
        assert_eq!(sonnet.context_window, 200_000);
        assert_eq!(sonnet.display_name.as_deref(), Some("Sonnet live"));
        // unknown live id gets conservative defaults
        let mystery = merged.iter().find(|m| m.id == "claude-mystery-9").unwrap();
        assert_eq!(mystery.max_output_tokens, 8192);
        assert!(mystery.pricing.is_none());
        // curated entries not in the live list are kept (opus, haiku)
        assert!(merged.iter().any(|m| m.id == "claude-opus-4-7"));
        assert!(merged.iter().any(|m| m.id == "claude-haiku-4-5"));
        // the covered base id is NOT duplicated
        assert!(!merged.iter().any(|m| m.id == "claude-sonnet-4-6"));
    }

    #[test]
    fn merge_with_empty_live_list_returns_full_snapshot() {
        assert_eq!(merge_with_live(&[]).len(), 3);
    }
}
