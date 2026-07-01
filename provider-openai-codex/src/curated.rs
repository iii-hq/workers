//! Static, namespaced model catalog for the Codex/ChatGPT-subscription
//! backend. The ChatGPT backend has no usable `GET /v1/models`, so the slice
//! is declared statically. Router ids are namespaced (`codex/…`) so they can
//! never collide with `provider-openai`'s dynamic `gpt-5.*` catalog (a shared
//! id makes a model unroutable via `AmbiguousModel`); the upstream model id
//! sent on the wire is the un-namespaced form.
use crate::PROVIDER_ID;
use llm_router::types::model::Model;

/// `(router id, upstream id, display, context_window, max_output, xhigh)`.
/// Subscription billing → no pricing (cost is the ChatGPT plan, not per-token).
const CATALOG: &[(&str, &str, &str, u64, u64, bool)] = &[
    (
        "codex/gpt-5.5",
        "gpt-5.5",
        "GPT-5.5 (Codex)",
        400_000,
        128_000,
        true,
    ),
    (
        "codex/gpt-5.4",
        "gpt-5.4",
        "GPT-5.4 (Codex)",
        400_000,
        128_000,
        true,
    ),
    (
        "codex/gpt-5.4-mini",
        "gpt-5.4-mini",
        "GPT-5.4 Mini (Codex)",
        400_000,
        128_000,
        true,
    ),
    (
        "codex/gpt-5-codex",
        "gpt-5-codex",
        "GPT-5 Codex",
        400_000,
        128_000,
        true,
    ),
];

/// Map a router model id (`codex/gpt-5.5`) to the upstream id (`gpt-5.5`).
/// Falls back to stripping a leading `codex/`, else the id verbatim.
pub fn upstream_model_id(router_id: &str) -> String {
    for (rid, upstream, ..) in CATALOG {
        if *rid == router_id {
            return (*upstream).to_string();
        }
    }
    router_id
        .strip_prefix("codex/")
        .unwrap_or(router_id)
        .to_string()
}

/// The static catalog slice declared to the router.
pub fn static_models() -> Vec<Model> {
    CATALOG
        .iter()
        .map(
            |(rid, _upstream, display, context_window, max_output_tokens, xhigh)| Model {
                id: (*rid).to_string(),
                provider: PROVIDER_ID.to_string(),
                display_name: Some((*display).to_string()),
                context_window: *context_window,
                max_output_tokens: *max_output_tokens,
                input_limit: None,
                supports_thinking: Some(true),
                supports_xhigh: Some(*xhigh),
                supports_tools: Some(true),
                supports_vision: Some(true),
                supports_cache: Some(true),
                supports_structured_output: None,
                thinking_budgets: None,
                pricing: None, // subscription billing, not per-token
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_ids_are_namespaced_and_map_to_upstream() {
        let models = static_models();
        assert!(models.iter().all(|m| m.id.starts_with("codex/")));
        assert_eq!(upstream_model_id("codex/gpt-5.5"), "gpt-5.5");
        assert_eq!(upstream_model_id("codex/unknown"), "unknown");
        assert_eq!(upstream_model_id("gpt-5.5"), "gpt-5.5");
    }

    #[test]
    fn no_overlap_with_bare_gpt_ids() {
        // Namespacing guarantees the router never sees a shared id.
        for m in static_models() {
            assert!(m.id.contains('/'), "{} must be namespaced", m.id);
        }
    }
}
