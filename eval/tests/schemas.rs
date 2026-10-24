mod support;

use eval::surface::catalog;

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

#[test]
fn catalog_matches_the_registered_surface() {
    let ids: Vec<_> = catalog().iter().map(|spec| spec.function_id).collect();
    assert_eq!(
        ids,
        [
            "eval::configure",
            "eval::config",
            "eval::start",
            "eval::list",
            "eval::status",
            "eval::result",
            "eval::cancel",
            "eval::step",
            "eval::sweep",
            "eval::execution-changed",
        ]
    );
}

#[test]
fn schemas_are_typed_and_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        support::assert_typed_schema(
            &format!("{} request", spec.function_id),
            &spec.request_schema,
        );
        support::assert_typed_schema(
            &format!("{} response", spec.function_id),
            &spec.response_schema,
        );
        let value = serde_json::json!({
            "function_id": spec.function_id,
            "request_schema": spec.request_schema,
            "response_schema": spec.response_schema,
        });
        let mut actual = serde_json::to_string_pretty(&value).unwrap();
        actual.push('\n');
        if let Err(error) = support::check_golden(&golden_file_name(spec.function_id), &actual) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn configure_accepts_engine_metadata_but_no_credentials_or_unknown_fields() {
    let configure = catalog()
        .into_iter()
        .find(|spec| spec.function_id == "eval::configure")
        .unwrap();
    let schema = serde_json::to_value(&configure.request_schema).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"].get("_caller_worker_id").is_some());
    let config = serde_json::json!({
        "subject": {"provider": "anthropic", "model": "claude"},
        "where": "harness", "runs_per_side": 3, "technical_retries": 1,
        "max_scenarios": 1, "estimated_usd_per_run": 0.5,
        "min_effect": {"tokens": 1000.0, "pass_rate_pp": 10.0},
        "budget": {"max_spend_usd": 10.0, "max_duration_ms": 7200000, "max_calls": 20,
                   "max_candidates": 2}
    });
    let request: eval::contract::ConfigureRequestV1 = serde_json::from_value(
        serde_json::json!({"config": config, "_caller_worker_id": "console"}),
    )
    .unwrap();
    request.config.validate().unwrap();
    let mut with_key = config.clone();
    with_key["api_key"] = "secret".into();
    assert!(
        serde_json::from_value::<eval::contract::ConfigureRequestV1>(
            serde_json::json!({"config": with_key})
        )
        .is_err()
    );
    // A stack on the E2E's own harness, or docker without one, cannot run.
    let mut harness_stack = request.config.clone();
    harness_stack.stack = Some(eval::contract::StackV1 {
        name: "s".into(),
        yaml: "y".into(),
    });
    assert!(harness_stack.validate().is_err());
    let mut docker = request.config.clone();
    docker.where_ = eval::contract::WhereV1::Docker;
    assert!(docker.validate().is_err());
}
