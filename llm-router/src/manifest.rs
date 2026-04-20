use serde_json::{json, Value};

pub fn build_manifest() -> Value {
    json!({
        "name": "iii-llm-router",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Unopinionated LLM routing brain. Wraps any gateway (LiteLLM/Bifrost/OpenRouter). Models, classifiers, policies, A/B tests, health — all registered at runtime via state.",
        "functions": [
            { "id": "router::decide", "description": "Pick a model for a request (hot path)" },
            { "id": "router::policy_create", "description": "Register a routing policy" },
            { "id": "router::policy_update", "description": "Patch a policy" },
            { "id": "router::policy_delete", "description": "Remove a policy" },
            { "id": "router::policy_list", "description": "List all policies" },
            { "id": "router::policy_test", "description": "Dry-run router::decide without logging" },
            { "id": "router::classify", "description": "Run prompt-complexity classifier only" },
            { "id": "router::classifier_config", "description": "Configure the category→model mapping" },
            { "id": "router::ab_create", "description": "Create an A/B test" },
            { "id": "router::ab_record", "description": "Record a quality/latency/cost outcome" },
            { "id": "router::ab_report", "description": "Aggregate A/B samples" },
            { "id": "router::ab_conclude", "description": "Mark an A/B test concluded" },
            { "id": "router::health_update", "description": "Update per-model health + latency" },
            { "id": "router::health_list", "description": "List health for all models" },
            { "id": "router::model_register", "description": "Register a model (name, quality, pricing)" },
            { "id": "router::model_unregister", "description": "Remove a model registration" },
            { "id": "router::model_list", "description": "List registered models" },
            { "id": "router::stats", "description": "Usage stats over a window" },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_has_required_fields() {
        let m = build_manifest();
        assert!(m.get("name").is_some());
        assert!(m.get("version").is_some());
        let fns = m.get("functions").unwrap().as_array().unwrap();
        assert_eq!(fns.len(), 18);
    }

    #[test]
    fn test_manifest_json_output() {
        let s = serde_json::to_string(&build_manifest()).unwrap();
        assert!(s.contains("router::decide"));
        assert!(s.contains("router::model_register"));
    }
}
