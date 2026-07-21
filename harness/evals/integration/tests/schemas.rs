//! Golden JSON Schemas for every serialized V1 contract type, checked in
//! under `schemas/`, plus serde round-trip tests for the scenario, router,
//! recorder-event, and result contracts.
//!
//! Regenerate after an intentional type change with:
//! `REGEN_SCHEMAS=1 cargo test --test schemas`

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::expand::CompiledFixtureV1;
use harness_integration::types::recorder::{RecorderEventKind, RecorderEventV1};
use harness_integration::types::scenario::{
    Classification, CompiledScenarioV1, ExecutionReportV1, IntegrationResultV1,
};
use harness_integration::types::script::{RouterScriptV1, SchemaVersion1};

fn goldens() -> Vec<(&'static str, serde_json::Value)> {
    fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
    }
    // The authored layer is code and never serialized, so it has no golden.
    vec![
        ("router-script.v1", schema::<RouterScriptV1>()),
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

/// Every registered scenario compiles and its strict runtime representation
/// round-trips through its typed mirror. The authored layer is code and has
/// no round trip.
#[test]
fn registered_scenarios_compile_and_round_trip() {
    use harness_integration::fixtures::ScenarioFixture;

    let registered = harness_integration::scenarios::all();
    assert!(!registered.is_empty(), "expected at least one scenario");
    for entry in &registered {
        let fixture = ScenarioFixture::from_registered(entry).unwrap();

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
    }
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
        scenario_id: "E2E-ROUND-TRIP".into(),
        classification: Classification::Pass,
        invariants: Vec::new(),
        artifacts: vec!["teardown.json".into()],
    });
    round_trip(ExecutionReportV1 {
        schema_version: SchemaVersion1::V1,
        run_id: "run-1".into(),
        scenario_id: "E2E-ROUND-TRIP".into(),
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
    for entry in &harness_integration::scenarios::all() {
        let fixture = ScenarioFixture::from_registered(entry).unwrap();
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
fn compiled_schema_matches_runtime_safety_constraints() {
    use harness_integration::expand::{compile_scenario, scenario_template, ScenarioTemplateKind};

    let schema = serde_json::to_value(schemars::schema_for!(CompiledScenarioV1)).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let authored = scenario_template(
        "E2E-COMPILED-SCHEMA",
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
