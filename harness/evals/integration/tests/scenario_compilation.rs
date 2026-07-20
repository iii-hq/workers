use std::collections::BTreeSet;
use std::path::Path;

use harness_integration::canonical::canonical_json_pretty;
use harness_integration::fixtures::ScenarioFixture;
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

fn scenario_slugs(scenarios: &Path) -> BTreeSet<String> {
    std::fs::read_dir(scenarios)
        .unwrap_or_else(|error| panic!("reading {}: {error}", scenarios.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir() && path.join("scenario.yaml").is_file())
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_else(|| {
                    panic!("scenario directory is not valid UTF-8: {}", path.display())
                })
                .to_owned()
        })
        .collect()
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
        "compiled snapshot inventory must match scenarios 1:1; \
         missing snapshots: {missing:?}; orphaned snapshots: {orphaned:?}"
    );
}

#[test]
fn all_compiled_scenarios_match_snapshots() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scenarios = manifest.join("scenarios");
    let snapshots = manifest.join("tests").join("snapshots");
    let regenerate = std::env::var_os("REGEN_SCENARIO_SNAPSHOTS").is_some();

    let scenario_slugs = scenario_slugs(&scenarios);
    if regenerate {
        for slug in &scenario_slugs {
            let fixture = ScenarioFixture::load(&scenarios.join(slug)).unwrap();
            let actual = canonical_json_pretty(&snapshot(&fixture));
            let path = snapshots.join(format!("{slug}.compiled.json"));
            std::fs::write(&path, &actual).unwrap();
        }
    }

    assert_snapshot_inventory(&scenario_slugs, &snapshot_slugs(&snapshots));

    for slug in &scenario_slugs {
        let fixture = ScenarioFixture::load(&scenarios.join(slug)).unwrap();
        let actual = canonical_json_pretty(&snapshot(&fixture));
        let path = snapshots.join(format!("{slug}.compiled.json"));
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        assert_eq!(
            actual, expected,
            "compiled snapshot for {slug}; regenerate with \
             REGEN_SCENARIO_SNAPSHOTS=1 cargo test --test scenario_compilation"
        );
    }
}

#[test]
fn inferred_function_history_contains_call_and_result() {
    let scenarios = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    let fixture = ScenarioFixture::load(&scenarios.join("exactly-once-function")).unwrap();
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
