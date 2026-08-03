//! Hardcoded curated metadata for the OpenCode Go catalog. Unlike Anthropic's
//! models API, OpenCode Go's `GET /v1/models` returns bare ids — no capability
//! tree, no display names, no limits — so live discovery owns the *id list*
//! while this module fills in everything the API cannot provide: per-model
//! metadata for the maintainer's curated model set, conservative defaults for
//! unknown ids.
//!
//! Source: models.dev api.json — the `opencode-go` ("OpenCode Go") provider
//! entry, fetched 2026-08-03 — plus the maintainer's model list. A missing
//! row only degrades capability enrichment, never routing.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, ReasoningEffort};

/// Hand-maintained metadata for the models we know (from models.dev, fetched
/// 2026-08-03). `reasoning_efforts` holds the effort values the API accepts
/// for the model; empty means the model reasons but publishes no effort
/// levels (toggle-only or undocumented), so the `reasoning_effort` param must
/// be omitted rather than guessed.
pub(crate) struct ModelMeta {
    pub(crate) context_window: u64,
    pub(crate) reasoning: bool,
    pub(crate) reasoning_efforts: &'static [&'static str],
    pub(crate) tool_call: bool,
    pub(crate) structured_output: bool,
}

/// One live id → its curated metadata, when known.
pub(crate) fn meta(id: &str) -> Option<&'static ModelMeta> {
    match id {
        "grok-4.5" => Some(&ModelMeta {
            context_window: 500_000,
            reasoning: true,
            reasoning_efforts: &["low", "medium", "high"],
            tool_call: true,
            structured_output: true,
        }),
        "glm-5.2" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &["high", "max"],
            tool_call: true,
            structured_output: true,
        }),
        "glm-5.1" => Some(&ModelMeta {
            context_window: 202_752,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "kimi-k3" => Some(&ModelMeta {
            context_window: 1_048_576,
            reasoning: true,
            reasoning_efforts: &["max"],
            tool_call: true,
            structured_output: true,
        }),
        "kimi-k2.7-code" => Some(&ModelMeta {
            context_window: 262_144,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: true,
        }),
        "kimi-k2.6" => Some(&ModelMeta {
            context_window: 262_144,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "minimax-m3" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "minimax-m2.7" => Some(&ModelMeta {
            context_window: 204_800,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "qwen3.7-max" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "qwen3.7-plus" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "qwen3.6-plus" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "deepseek-v4-pro" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &["high", "max"],
            tool_call: true,
            structured_output: true,
        }),
        "deepseek-v4-flash" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &["high", "max"],
            tool_call: true,
            structured_output: true,
        }),
        "mimo-v2.5" => Some(&ModelMeta {
            context_window: 1_000_000,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "mimo-v2.5-pro" => Some(&ModelMeta {
            context_window: 1_048_576,
            reasoning: true,
            reasoning_efforts: &[],
            tool_call: true,
            structured_output: false,
        }),
        "hy3" => Some(&ModelMeta {
            context_window: 256_000,
            reasoning: true,
            reasoning_efforts: &["none", "low", "high"],
            tool_call: true,
            structured_output: false,
        }),
        _ => None,
    }
}

/// One live id → catalog Model: curated metadata when the id is known,
/// conservative defaults otherwise. Unknown ids keep the pre-curation
/// defaults (128K context, no thinking) — tools stay on uniformly.
pub fn enrich(id: &str) -> Model {
    match meta(id) {
        Some(m) => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name: Some(id.into()),
            context_window: m.context_window,
            max_output_tokens: 4096,
            input_limit: None,
            supports_thinking: if m.reasoning { Some(true) } else { None },
            supports_xhigh: None,
            reasoning_efforts: if m.reasoning_efforts.is_empty() {
                None
            } else {
                Some(
                    m.reasoning_efforts
                        .iter()
                        .map(|e| ReasoningEffort {
                            effort: (*e).to_string(),
                            description: None,
                        })
                        .collect(),
                )
            },
            supports_tools: if m.tool_call { Some(true) } else { None },
            supports_vision: None,
            supports_cache: None,
            supports_structured_output: if m.structured_output {
                Some(true)
            } else {
                None
            },
            thinking_budgets: None,
            pricing: None,
        },
        None => Model {
            id: id.into(),
            provider: PROVIDER_ID.into(),
            display_name: Some(id.into()),
            context_window: 128_000,
            max_output_tokens: 4096,
            input_limit: None,
            supports_thinking: None,
            supports_xhigh: None,
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: None,
            supports_cache: None,
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The maintainer's curated model set — every id must resolve, or the
    /// catalog silently degrades that model to conservative defaults.
    #[test]
    fn all_curated_model_ids_have_entries() {
        let ids = [
            "grok-4.5",
            "glm-5.2",
            "glm-5.1",
            "kimi-k3",
            "kimi-k2.7-code",
            "kimi-k2.6",
            "minimax-m3",
            "minimax-m2.7",
            "qwen3.7-max",
            "qwen3.7-plus",
            "qwen3.6-plus",
            "deepseek-v4-pro",
            "deepseek-v4-flash",
            "mimo-v2.5",
            "mimo-v2.5-pro",
            "hy3",
        ];
        for id in ids {
            assert!(meta(id).is_some(), "{id} missing from the curated table");
        }
    }

    #[test]
    fn enrich_applies_curated_metadata() {
        let m = enrich("deepseek-v4-flash");
        assert_eq!(m.id, "deepseek-v4-flash");
        assert_eq!(m.display_name.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(m.context_window, 1_000_000);
        assert_eq!(m.supports_thinking, Some(true));
        let efforts: Vec<&str> = m
            .reasoning_efforts
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.effort.as_str())
            .collect();
        assert_eq!(efforts, ["high", "max"]);
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_structured_output, Some(true));

        // grok-4.5: low/medium/high ladder, 500K context.
        let g = enrich("grok-4.5");
        assert_eq!(g.context_window, 500_000);
        let efforts: Vec<&str> = g
            .reasoning_efforts
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.effort.as_str())
            .collect();
        assert_eq!(efforts, ["low", "medium", "high"]);

        // Effort-less reasoning families advertise thinking but no efforts.
        let q = enrich("qwen3.6-plus");
        assert_eq!(q.supports_thinking, Some(true));
        assert!(q.reasoning_efforts.is_none());
        assert!(q.supports_structured_output.is_none());
    }

    #[test]
    fn enrich_defaults_conservatively_for_unknown_ids() {
        for id in ["opencode-go-test-model", "unknown-model", "gpt-4o"] {
            let m = enrich(id);
            assert_eq!(m.display_name.as_deref(), Some(id));
            assert_eq!(m.context_window, 128_000);
            assert_eq!(m.max_output_tokens, 4096);
            assert_eq!(m.supports_thinking, None);
            assert!(m.reasoning_efforts.is_none());
            assert_eq!(m.supports_tools, Some(true));
            assert_eq!(m.supports_structured_output, None);
        }
    }
}
