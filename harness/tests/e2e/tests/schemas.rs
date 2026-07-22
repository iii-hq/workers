//! Serde round-trip tests for the serialized V1 contract types (scenario,
//! router, trace-evidence, and result), plus validation of compiled payloads
//! against the producer-owned contracts. There are no generated goldens:
//! wire shape is pinned by the round trips, and silent compiler weakening is
//! caught by focused fixture tests.

use harness_integration::expand::CompiledFixtureV1;
use harness_integration::types::scenario::{
    Classification, CompiledScenarioV1, ExecutionReportV1, IntegrationResultV1,
};
use harness_integration::types::script::{RouterScriptV1, SchemaVersion1};
use harness_integration::types::trace::TraceEvidenceV1;

/// Every strict checked-in fixture round-trips through its typed mirror.
#[test]
fn registered_scenarios_compile_and_round_trip() {
    let fixtures = harness_integration::scenarios::all();
    assert!(!fixtures.is_empty(), "expected at least one scenario");
    for fixture in &fixtures {
        fixture.validate().unwrap();

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

    round_trip(TraceEvidenceV1::new(Vec::new()));
    round_trip(IntegrationResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: "E2E-ROUND-TRIP".into(),
        classification: Classification::Pass,
        failure: None,
        artifacts: vec!["teardown.json".into()],
    });
    round_trip(IntegrationResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: "E2E-ROUND-TRIP".into(),
        classification: Classification::ContractFailure,
        failure: Some("verify: {{run_id}}::record ran 0 times".into()),
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
    let golden: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../harness/tests/golden/schemas/harness.send.json"
    )))
    .unwrap();
    let validator = jsonschema::JSONSchema::compile(&golden["request_schema"]).unwrap();
    for fixture in &harness_integration::scenarios::all() {
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
    let schema = serde_json::to_value(schemars::schema_for!(CompiledScenarioV1)).unwrap();
    let validator = jsonschema::JSONSchema::compile(&schema).unwrap();
    let valid = serde_json::to_value(
        harness_integration::scenarios::all()
            .into_iter()
            .next()
            .unwrap()
            .scenario,
    )
    .unwrap();
    assert!(validator.is_valid(&valid));

    let mut unsafe_id = valid.clone();
    unsafe_id["id"] = serde_json::json!("../../escape");
    assert!(!validator.is_valid(&unsafe_id));

    let mut zero_deadline = valid;
    zero_deadline["deadlines"]["scenario_ms"] = serde_json::json!(0);
    assert!(!validator.is_valid(&zero_deadline));
}
