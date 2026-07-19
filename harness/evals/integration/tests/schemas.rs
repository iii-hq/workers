//! Golden JSON Schemas for every V1 contract type, checked in under
//! `schemas/` (spec § Verification and acceptance: JSON Schema and serde
//! round-trip tests for executed scenario, router, recorder-event, and result
//! contracts).
//!
//! Regenerate after an intentional type change with:
//! `REGEN_SCHEMAS=1 cargo test --test schemas`

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::types::recorder::RecorderEventV1;
use harness_integration::types::scenario::{
    CompiledScenarioV1, ExecutionReportV1, IntegrationResultV1, IntegrationScenarioV1,
};
use harness_integration::types::script::RouterScriptV1;

fn goldens() -> Vec<(&'static str, serde_json::Value)> {
    fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
    }
    vec![
        ("router-script.v1", schema::<RouterScriptV1>()),
        ("authored-scenario.v1", schema::<IntegrationScenarioV1>()),
        ("compiled-scenario.v1", schema::<CompiledScenarioV1>()),
        ("integration-result.v1", schema::<IntegrationResultV1>()),
        ("execution-report.v1", schema::<ExecutionReportV1>()),
        ("recorder-event.v1", schema::<RecorderEventV1>()),
    ]
}

#[test]
fn committed_schemas_match_the_types() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas");
    let regen = std::env::var_os("REGEN_SCHEMAS").is_some();
    let mut expected_files = std::collections::BTreeSet::new();
    for (name, schema) in goldens() {
        let path = dir.join(format!("{name}.json"));
        expected_files.insert(path.clone());
        let rendered = canonical_json_pretty(&schema);
        if regen {
            std::fs::write(&path, &rendered).unwrap();
            continue;
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing golden {}: {e}", path.display()));
        assert_eq!(
            committed, rendered,
            "{name}.json is stale; regenerate with REGEN_SCHEMAS=1 cargo test --test schemas"
        );
    }
    // No orphaned goldens: every committed schema corresponds to a type.
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            assert!(
                expected_files.contains(&path),
                "orphaned golden schema {}",
                path.display()
            );
        }
    }
}

/// Every committed single-file scenario compiles and its authored and strict
/// runtime representations round-trip through their typed mirrors.
#[test]
fn committed_scenarios_compile_and_round_trip() {
    use harness_integration::fixtures::ScenarioFixture;

    let scenarios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    let mut checked = 0;
    for entry in std::fs::read_dir(&scenarios).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.join("scenario.yaml").is_file() {
            continue;
        }
        let fixture = ScenarioFixture::load(&dir).unwrap();
        let authored_value = serde_json::to_value(&fixture.authored).unwrap();
        let authored_again: IntegrationScenarioV1 = serde_json::from_value(authored_value).unwrap();
        assert_eq!(fixture.authored, authored_again);

        let compiled_value = serde_json::to_value(&fixture.scenario).unwrap();
        let compiled_again: CompiledScenarioV1 = serde_json::from_value(compiled_value).unwrap();
        assert_eq!(fixture.scenario, compiled_again);

        let script_value = serde_json::to_value(&fixture.script).unwrap();
        let script_again: RouterScriptV1 = serde_json::from_value(script_value).unwrap();
        assert_eq!(fixture.script, script_again);
        checked += 1;
    }
    assert!(checked > 0, "expected at least one committed scenario");
}

#[test]
fn authored_schema_matches_compiler_safety_constraints() {
    use harness_integration::expand::{scenario_template, ScenarioTemplateKind};

    let schema = serde_json::to_value(schemars::schema_for!(IntegrationScenarioV1)).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let valid = serde_json::to_value(scenario_template(
        "C-E2E-SCHEMA",
        "Validate authored schema constraints.",
        ScenarioTemplateKind::Crash,
    ))
    .unwrap();
    assert!(validator.is_valid(&valid));

    let mut unsafe_id = valid.clone();
    unsafe_id["id"] = serde_json::json!("../../escape");
    assert!(!validator.is_valid(&unsafe_id));

    let mut unsafe_alias = valid.clone();
    let functions = unsafe_alias["functions"].as_object_mut().unwrap();
    let function = functions.remove("record").unwrap();
    functions.insert("../record".to_string(), function);
    assert!(!validator.is_valid(&unsafe_alias));

    let mut zero_timeout = valid.clone();
    zero_timeout["timeouts"]["teardown_ms"] = serde_json::json!(0);
    assert!(!validator.is_valid(&zero_timeout));

    let mut zero_fault_threshold = valid;
    zero_fault_threshold["fault"]["after_target_calls"] = serde_json::json!(0);
    assert!(!validator.is_valid(&zero_fault_threshold));
}

#[test]
fn compiled_schema_matches_runtime_safety_constraints() {
    use harness_integration::expand::{compile_scenario, scenario_template, ScenarioTemplateKind};

    let schema = serde_json::to_value(schemars::schema_for!(CompiledScenarioV1)).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let authored = scenario_template(
        "C-E2E-COMPILED-SCHEMA",
        "Validate compiled schema constraints.",
        ScenarioTemplateKind::Crash,
    );
    let valid =
        serde_json::to_value(compile_scenario(&authored, "prompt").unwrap().scenario).unwrap();
    assert!(validator.is_valid(&valid));

    let mut unsafe_id = valid.clone();
    unsafe_id["id"] = serde_json::json!("../../escape");
    assert!(!validator.is_valid(&unsafe_id));

    let mut zero_fault_threshold = valid;
    zero_fault_threshold["fault"]["after_target_calls"] = serde_json::json!(0);
    assert!(!validator.is_valid(&zero_fault_threshold));
}
