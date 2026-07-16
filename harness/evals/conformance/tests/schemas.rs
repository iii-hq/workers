//! Golden JSON Schemas for every V1 contract type, checked in under
//! `schemas/` (spec § Verification and acceptance: "JSON Schema and serde
//! round-trip tests for scripts, cassettes, scenarios, recorder events, and
//! results").
//!
//! Regenerate after an intentional type change with:
//! `REGEN_SCHEMAS=1 cargo test --test schemas`

use harness_conformance::canonical::canonical_json_pretty;
use harness_conformance::types::cassette::RouterCassetteV1;
use harness_conformance::types::recorder::{
    RecorderAwaitRequestV1, RecorderAwaitResponseV1, RecorderConfigureRequestV1,
    RecorderConfigureResponseV1, RecorderEventV1, RecorderResetRequestV1, RecorderResetResponseV1,
    RecorderSnapshotRequestV1, RecorderSnapshotResponseV1,
};
use harness_conformance::types::scenario::{ConformanceResultV1, ConformanceScenarioV1};
use harness_conformance::types::script::RouterScriptV1;

fn goldens() -> Vec<(&'static str, serde_json::Value)> {
    fn schema<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
    }
    vec![
        ("router-script.v1", schema::<RouterScriptV1>()),
        ("router-cassette.v1", schema::<RouterCassetteV1>()),
        ("conformance-scenario.v1", schema::<ConformanceScenarioV1>()),
        ("conformance-result.v1", schema::<ConformanceResultV1>()),
        ("recorder-event.v1", schema::<RecorderEventV1>()),
        (
            "recorder-configure.request.v1",
            schema::<RecorderConfigureRequestV1>(),
        ),
        (
            "recorder-configure.response.v1",
            schema::<RecorderConfigureResponseV1>(),
        ),
        (
            "recorder-reset.request.v1",
            schema::<RecorderResetRequestV1>(),
        ),
        (
            "recorder-reset.response.v1",
            schema::<RecorderResetResponseV1>(),
        ),
        (
            "recorder-snapshot.request.v1",
            schema::<RecorderSnapshotRequestV1>(),
        ),
        (
            "recorder-snapshot.response.v1",
            schema::<RecorderSnapshotResponseV1>(),
        ),
        (
            "recorder-await.request.v1",
            schema::<RecorderAwaitRequestV1>(),
        ),
        (
            "recorder-await.response.v1",
            schema::<RecorderAwaitResponseV1>(),
        ),
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

/// The committed fixture scripts round-trip byte-stably through the typed
/// mirrors: parse → serialize → parse yields the identical JSON value, so no
/// authored field is silently dropped or defaulted.
#[test]
fn committed_fixture_scripts_round_trip() {
    let scenarios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    let mut checked = 0;
    for entry in std::fs::read_dir(&scenarios).unwrap() {
        let dir = entry.unwrap().path();
        let script_path = dir.join("router-script.json");
        if !script_path.is_file() {
            continue;
        }
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&script_path).unwrap()).unwrap();
        let typed: RouterScriptV1 = serde_json::from_value(raw.clone()).unwrap();
        let reserialized = serde_json::to_value(&typed).unwrap();
        assert_eq!(
            raw,
            reserialized,
            "{} does not round-trip",
            script_path.display()
        );
        checked += 1;
    }
    assert!(
        checked >= 2,
        "expected at least the two first-slice scenario scripts, found {checked}"
    );
}
