use serde_json::Value;

#[test]
fn build_skills_payload_has_expected_shape() {
    let payload: Value = harness::build_skills_register_payload();
    assert_eq!(payload["id"], "harness");
    assert_eq!(payload["skill_version"], env!("CARGO_PKG_VERSION"));
    assert!(payload["min_console_version"].is_string());
    assert!(payload["expected_workers"].is_array());
    let workers = payload["expected_workers"].as_array().unwrap();
    assert!(workers.iter().all(serde_json::Value::is_string));
    assert!(workers.contains(&Value::String("turn-orchestrator".to_string())));
}
