use std::collections::BTreeSet;
use std::path::Path;

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::fixtures::ScenarioFixture;
use harness_integration::scenarios::RegisteredScenario;
use harness_integration::types::script::JsonMatcherV1;
use serde_json::{json, Value};

fn snapshot(fixture: &ScenarioFixture) -> Value {
    json!({
        "scenario": serde_json::to_value(&fixture.scenario).unwrap(),
        "router_script": serde_json::to_value(&fixture.script).unwrap(),
        "system_prompt_template_sha256":
            harness_integration::canonical::sha256_of_bytes(
                fixture.system_prompt_template.as_bytes()
            )
    })
}

fn scenarios_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios")
}

fn load(entry: &RegisteredScenario) -> ScenarioFixture {
    ScenarioFixture::from_registered(entry, &scenarios_root())
        .unwrap_or_else(|error| panic!("compiling {}: {error:#}", entry.slug))
}

fn snapshot_slugs(snapshots: &Path) -> BTreeSet<String> {
    const SUFFIX: &str = ".compiled.json";

    std::fs::read_dir(snapshots)
        .unwrap_or_else(|error| panic!("reading {}: {error}", snapshots.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(SUFFIX))
                .map(str::to_owned)
        })
        .collect()
}

fn assert_snapshot_inventory(scenarios: &BTreeSet<String>, snapshots: &BTreeSet<String>) {
    let missing = scenarios.difference(snapshots).collect::<Vec<_>>();
    let orphaned = snapshots.difference(scenarios).collect::<Vec<_>>();
    assert!(
        missing.is_empty() && orphaned.is_empty(),
        "compiled snapshot inventory must match registered scenarios 1:1; \
         missing snapshots: {missing:?}; orphaned snapshots: {orphaned:?}"
    );
}

#[test]
fn all_compiled_scenarios_match_snapshots() {
    let snapshots = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots");
    let regenerate = std::env::var_os("REGEN_SCENARIO_SNAPSHOTS").is_some();

    let registered = harness_integration::scenarios::all();
    if regenerate {
        for entry in &registered {
            let actual = canonical_json_pretty(&snapshot(&load(entry)));
            let path = snapshots.join(format!("{}.compiled.json", entry.slug));
            std::fs::write(&path, &actual).unwrap();
        }
    }

    let registered_slugs: BTreeSet<String> =
        registered.iter().map(|entry| entry.slug.clone()).collect();
    assert_snapshot_inventory(&registered_slugs, &snapshot_slugs(&snapshots));

    for entry in &registered {
        let actual = canonical_json_pretty(&snapshot(&load(entry)));
        let path = snapshots.join(format!("{}.compiled.json", entry.slug));
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        assert_eq!(
            actual, expected,
            "compiled snapshot for {}; regenerate with \
             REGEN_SCENARIO_SNAPSHOTS=1 cargo test --test scenario_compilation",
            entry.slug
        );
    }
}

#[test]
fn inferred_function_history_contains_call_and_result() {
    let registered = harness_integration::scenarios::all();
    let entry = registered
        .iter()
        .find(|entry| entry.slug == "exactly-once-function")
        .expect("exactly-once-function is registered");
    let fixture = load(entry);
    let matcher = &fixture.script.generations[1].match_.messages;
    let JsonMatcherV1::Exact { expected, .. } = matcher else {
        panic!("function history should use an exact matcher");
    };
    let messages = expected.as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["content"][0]["type"], "function_call");
    assert_eq!(messages[2]["role"], "function_result");
    assert_eq!(messages[2]["function_call_id"], "call-1");
}
