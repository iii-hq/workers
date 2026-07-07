//! The hand-maintained Z.AI catalog. Z.AI's OpenAI-compatible API exposes no
//! models-listing endpoint (`GET /models` is absent from docs.z.ai's OpenAPI
//! spec), so this table IS the catalog: ids, limits, capabilities, and
//! pricing, all from docs.z.ai (guides/overview/pricing, guides/llm/*,
//! guides/vlm/*), snapshot 2026-07. A stale row degrades cost display and
//! limits, never routing correctness.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing};

struct Row {
    id: &'static str,
    display: &'static str,
    context_window: u64,
    max_output_tokens: u64,
    vision: bool,
    /// USD per MTok: (input, cached input, output). All zeros = free tier.
    price: (f64, f64, f64),
}

/// Current GLM lineup (4.5 and newer). Older generations (glm-4-32b, glm-3)
/// and non-chat modalities (glm-ocr, glm-image, cogvideox, autoglm) are
/// deliberately absent.
const ROWS: &[Row] = &[
    Row {
        id: "glm-5.2",
        display: "GLM 5.2",
        context_window: 1_000_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (1.40, 0.26, 4.40),
    },
    Row {
        id: "glm-5.1",
        display: "GLM 5.1",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (1.40, 0.26, 4.40),
    },
    Row {
        id: "glm-5",
        display: "GLM 5",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (1.00, 0.20, 3.20),
    },
    Row {
        id: "glm-5-turbo",
        display: "GLM 5 Turbo",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (1.20, 0.24, 4.00),
    },
    Row {
        id: "glm-4.7",
        display: "GLM 4.7",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (0.60, 0.11, 2.20),
    },
    Row {
        id: "glm-4.7-flashx",
        display: "GLM 4.7 FlashX",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (0.07, 0.01, 0.40),
    },
    Row {
        id: "glm-4.7-flash",
        display: "GLM 4.7 Flash",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (0.0, 0.0, 0.0),
    },
    Row {
        id: "glm-4.6",
        display: "GLM 4.6",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: false,
        price: (0.60, 0.11, 2.20),
    },
    Row {
        id: "glm-4.5",
        display: "GLM 4.5",
        context_window: 128_000,
        max_output_tokens: 96_000,
        vision: false,
        price: (0.60, 0.11, 2.20),
    },
    Row {
        id: "glm-4.5-air",
        display: "GLM 4.5 Air",
        context_window: 128_000,
        max_output_tokens: 96_000,
        vision: false,
        price: (0.20, 0.03, 1.10),
    },
    Row {
        id: "glm-4.5-x",
        display: "GLM 4.5 X",
        context_window: 128_000,
        max_output_tokens: 96_000,
        vision: false,
        price: (2.20, 0.45, 8.90),
    },
    Row {
        id: "glm-4.5-airx",
        display: "GLM 4.5 AirX",
        context_window: 128_000,
        max_output_tokens: 96_000,
        vision: false,
        price: (1.10, 0.22, 4.50),
    },
    Row {
        id: "glm-4.5-flash",
        display: "GLM 4.5 Flash",
        context_window: 128_000,
        max_output_tokens: 96_000,
        vision: false,
        price: (0.0, 0.0, 0.0),
    },
    Row {
        id: "glm-5v-turbo",
        display: "GLM 5V Turbo",
        context_window: 200_000,
        max_output_tokens: 128_000,
        vision: true,
        price: (1.20, 0.24, 4.00),
    },
    // ponytail: 64K context on the 4.x vision rows is a conservative floor —
    // docs.z.ai documents only their max output; raise when documented.
    Row {
        id: "glm-4.6v",
        display: "GLM 4.6V",
        context_window: 64_000,
        max_output_tokens: 32_000,
        vision: true,
        price: (0.30, 0.05, 0.90),
    },
    Row {
        id: "glm-4.5v",
        display: "GLM 4.5V",
        context_window: 64_000,
        max_output_tokens: 16_000,
        vision: true,
        price: (0.60, 0.11, 1.80),
    },
];

/// Models the GLM Coding Plan endpoint serves: the featured tier models
/// (docs.z.ai devpack/overview: GLM-5.2, GLM-5-Turbo, GLM-4.7) plus the
/// additional codes the coding-tools guide documents for that endpoint
/// (docs.z.ai scenario-example/develop-tools/others: GLM-5.1, GLM-5,
/// GLM-4.5-air). The catalog shrinks to these when the resolved endpoint
/// is the coding one.
const CODING_PLAN_IDS: [&str; 6] = [
    "glm-5.2",
    "glm-5.1",
    "glm-5",
    "glm-5-turbo",
    "glm-4.7",
    "glm-4.5-air",
];

/// The full catalog slice reconciled into the router.
pub fn models() -> Vec<Model> {
    ROWS.iter().map(to_model).collect()
}

/// The Coding Plan subset of the catalog.
pub fn coding_models() -> Vec<Model> {
    ROWS.iter()
        .filter(|r| CODING_PLAN_IDS.contains(&r.id))
        .map(to_model)
        .collect()
}

fn to_model(r: &Row) -> Model {
    let (input, cached, output) = r.price;
    Model {
        id: r.id.into(),
        provider: PROVIDER_ID.into(),
        display_name: Some(r.display.into()),
        context_window: r.context_window,
        max_output_tokens: r.max_output_tokens,
        input_limit: None,
        // Every current GLM chat model takes the `thinking` toggle
        // (docs.z.ai guides/capabilities/thinking: GLM-4.5 and newer).
        supports_thinking: Some(true),
        // `reasoning_effort: xhigh` exists on GLM-5.2+ only.
        supports_xhigh: Some(r.id.starts_with("glm-5.2")),
        supports_tools: Some(true),
        supports_vision: Some(r.vision),
        // Implicit prompt caching, billed at the cached-input rate.
        supports_cache: Some(true),
        // Z.AI documents `json_object` only — no strict json_schema mode.
        supports_structured_output: Some(false),
        thinking_budgets: None, // toggle + effort enum, not token budgets
        pricing: Some(Pricing {
            input: Some(input),
            output: Some(output),
            cache_read: Some(cached),
            cache_write: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_ids_are_unique_and_owned_by_zai() {
        let models = models();
        assert!(!models.is_empty());
        let ids: HashSet<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), models.len(), "duplicate ids in ROWS");
        assert!(models.iter().all(|m| m.provider == "zai"));
    }

    #[test]
    fn flagship_row_matches_docs() {
        let m = models().into_iter().find(|m| m.id == "glm-5.2").unwrap();
        assert_eq!(m.display_name.as_deref(), Some("GLM 5.2"));
        assert_eq!(m.context_window, 1_000_000);
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_xhigh, Some(true));
        assert_eq!(m.supports_structured_output, Some(false));
        let p = m.pricing.unwrap();
        assert_eq!(p.input, Some(1.40));
        assert_eq!(p.cache_read, Some(0.26));
        assert_eq!(p.output, Some(4.40));
    }

    #[test]
    fn only_glm_52_supports_xhigh_and_only_v_models_see_images() {
        for m in models() {
            assert_eq!(
                m.supports_xhigh,
                Some(m.id.starts_with("glm-5.2")),
                "{}",
                m.id
            );
            assert_eq!(m.supports_vision, Some(m.id.contains('v')), "{}", m.id);
        }
    }

    #[test]
    fn free_tier_rows_price_at_zero() {
        for id in ["glm-4.7-flash", "glm-4.5-flash"] {
            let m = models().into_iter().find(|m| m.id == id).unwrap();
            let p = m.pricing.unwrap();
            assert_eq!(p.input, Some(0.0));
            assert_eq!(p.output, Some(0.0));
        }
    }
}
