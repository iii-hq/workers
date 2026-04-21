use serde_json::{json, Value};

// Canonical list of router functions and their descriptions. Both
// register_functions() in main.rs and build_manifest() below derive from
// this, so the published manifest and the registered handlers can never drift.
pub const FUNCTIONS: &[(&str, &str)] = &[
    ("router::decide", "Pick a model for a request (hot path)"),
    ("router::policy_create", "Register a routing policy"),
    ("router::policy_update", "Patch a policy"),
    ("router::policy_delete", "Remove a policy"),
    ("router::policy_list", "List all policies"),
    ("router::policy_test", "Dry-run router::decide without logging"),
    ("router::classify", "Run prompt-complexity classifier only"),
    ("router::classifier_config", "Configure the category→model mapping"),
    ("router::ab_create", "Create an A/B test"),
    ("router::ab_record", "Record a quality/latency/cost outcome"),
    ("router::ab_report", "Aggregate A/B samples"),
    ("router::ab_conclude", "Mark an A/B test concluded"),
    ("router::health_update", "Update per-model health + latency"),
    ("router::health_list", "List health for all models"),
    (
        "router::model_register",
        "Register a model (name, quality, pricing)",
    ),
    ("router::model_unregister", "Remove a model registration"),
    ("router::model_list", "List registered models"),
    ("router::stats", "Usage stats over a window"),
];

pub fn build_manifest() -> Value {
    let fns: Vec<Value> = FUNCTIONS
        .iter()
        .map(|(id, desc)| json!({ "id": id, "description": desc }))
        .collect();
    json!({
        "name": "iii-llm-router",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Unopinionated LLM routing brain. Wraps any gateway (LiteLLM/Bifrost/OpenRouter). Models, classifiers, policies, A/B tests, health — all registered at runtime via state.",
        "functions": fns,
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
        assert_eq!(fns.len(), FUNCTIONS.len());
    }

    #[test]
    fn test_manifest_json_output() {
        let s = serde_json::to_string(&build_manifest()).unwrap();
        assert!(s.contains("router::decide"));
        assert!(s.contains("router::model_register"));
    }

    #[test]
    fn functions_const_has_18_entries() {
        assert_eq!(FUNCTIONS.len(), 18);
    }
}
