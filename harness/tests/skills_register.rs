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

// TEMP(iii-skill): delete this test when the dedicated iii skill worker lands.
#[test]
fn build_iii_skill_payload_matches_skills_register_contract() {
    let payload: Value = harness::build_iii_skill_register_payload();
    assert_eq!(payload["id"], "iii");
    let skill = payload["skill"].as_str().expect("skill must be a string");
    assert!(!skill.trim().is_empty(), "skill body must be non-empty");
    assert!(
        skill.starts_with("# iii"),
        "skill must lead with an H1 title"
    );
}

// TEMP(sandbox-skill): delete this test when iii-sandbox publishes its own.
#[test]
fn build_sandbox_skill_payload_matches_skills_register_contract() {
    let payload: Value = harness::build_sandbox_skill_register_payload();
    assert_eq!(payload["id"], "sandbox");
    let skill = payload["skill"].as_str().expect("skill must be a string");
    assert!(!skill.trim().is_empty(), "skill body must be non-empty");
    assert!(
        skill.starts_with("# sandbox"),
        "skill must lead with an H1 title"
    );
    assert!(
        skill.contains("sandbox::create") && skill.contains("sandbox::fs::read"),
        "skill must mention the lifecycle and at least one fs:: function"
    );
}
