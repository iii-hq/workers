//! Golden JSON Schemas for every V1 contract type, checked in under
//! `schemas/` (spec § Verification and acceptance: JSON Schema and serde
//! round-trip tests for executed scenario, router, recorder-event, and result
//! contracts).
//!
//! Regenerate after an intentional type change with:
//! `REGEN_SCHEMAS=1 cargo test --test schemas`

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::expand::CompiledFixtureV1;
use harness_integration::types::recorder::{RecorderEventKind, RecorderEventV1};
use harness_integration::types::scenario::{
    AuthoredScenarioV1, Classification, CompiledScenarioV1, ExecutionReportV1, IntegrationResultV1,
};
use harness_integration::types::script::{RouterScriptV1, SchemaVersion1};

fn goldens() -> Vec<(&'static str, serde_json::Value)> {
    fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
    }
    vec![
        ("router-script.v1", schema::<RouterScriptV1>()),
        ("authored-scenario.v1", schema::<AuthoredScenarioV1>()),
        ("compiled-scenario.v1", schema::<CompiledScenarioV1>()),
        ("compiled-fixture.v1", schema::<CompiledFixtureV1>()),
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
        let authored_again: AuthoredScenarioV1 = serde_json::from_value(authored_value).unwrap();
        assert_eq!(fixture.authored, authored_again);

        let compiled_value = serde_json::to_value(&fixture.scenario).unwrap();
        let compiled_again: CompiledScenarioV1 = serde_json::from_value(compiled_value).unwrap();
        assert_eq!(fixture.scenario, compiled_again);

        let script_value = serde_json::to_value(&fixture.script).unwrap();
        let script_again: RouterScriptV1 = serde_json::from_value(script_value).unwrap();
        assert_eq!(fixture.script, script_again);

        let compiled_fixture = fixture.compiled();
        let fixture_value = serde_json::to_value(&compiled_fixture).unwrap();
        let fixture_again: CompiledFixtureV1 = serde_json::from_value(fixture_value).unwrap();
        assert_eq!(compiled_fixture, fixture_again);
        checked += 1;
    }
    assert!(checked > 0, "expected at least one committed scenario");
}

#[test]
fn evidence_and_report_contracts_round_trip() {
    fn round_trip<T>(value: T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let encoded = serde_json::to_value(&value).unwrap();
        let decoded: T = serde_json::from_value(encoded).unwrap();
        assert_eq!(value, decoded);
    }

    round_trip(RecorderEventV1 {
        schema_version: SchemaVersion1::V1,
        run_id: "run-1".into(),
        sequence: 1,
        kind: RecorderEventKind::TargetCall,
        function_id: "run-1::target".into(),
        payload: serde_json::json!({ "value": "expected" }),
        received_at: "2026-07-19T00:00:00Z".into(),
    });
    round_trip(IntegrationResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: "C-E2E-ROUND-TRIP".into(),
        classification: Classification::Pass,
        invariants: Vec::new(),
        artifacts: vec!["teardown.json".into()],
    });
    round_trip(ExecutionReportV1 {
        schema_version: SchemaVersion1::V1,
        run_id: "run-1".into(),
        scenario_id: "C-E2E-ROUND-TRIP".into(),
        started_at: "2026-07-19T00:00:00Z".into(),
        duration_ms: 1,
        result_path: "result.json".into(),
        result_sha256: "0".repeat(64),
    });
}

#[test]
fn compiled_send_is_accepted_by_the_authoritative_harness_contract() {
    use harness_integration::fixtures::ScenarioFixture;

    let golden: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../harness/tests/golden/schemas/harness.send.json"
    )))
    .unwrap();
    let validator = jsonschema::JSONSchema::compile(&golden["request_schema"]).unwrap();
    let scenarios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    for entry in std::fs::read_dir(scenarios).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.join("scenario.yaml").is_file() {
            continue;
        }
        let fixture = ScenarioFixture::load(&dir).unwrap();
        let send = serde_json::to_value(&fixture.scenario.send).unwrap();
        let errors = validator
            .validate(&send)
            .err()
            .map(|errors| errors.map(|error| error.to_string()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(
            errors.is_empty(),
            "{} compiled an invalid harness::send request: {errors:?}",
            fixture.scenario.id
        );
    }
}

#[test]
fn authored_schema_matches_compiler_safety_constraints() {
    use harness_integration::expand::{scenario_template, ScenarioTemplateKind};

    let schema = serde_json::to_value(schemars::schema_for!(AuthoredScenarioV1)).unwrap();
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
