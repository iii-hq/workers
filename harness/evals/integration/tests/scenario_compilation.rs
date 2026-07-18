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

#[test]
fn representative_compiled_scenarios_match_snapshots() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scenarios = manifest.join("scenarios");
    let snapshots = manifest.join("tests").join("snapshots");
    let regenerate = std::env::var_os("REGEN_SCENARIO_SNAPSHOTS").is_some();
    for slug in [
        "streamed-text",
        "exactly-once-function",
        "hold-mutation-505",
        "crash-recovery-507",
    ] {
        let fixture = ScenarioFixture::load(&scenarios.join(slug)).unwrap();
        let actual = canonical_json_pretty(&snapshot(&fixture));
        let path = snapshots.join(format!("{slug}.compiled.json"));
        if regenerate {
            std::fs::write(&path, &actual).unwrap();
            continue;
        }
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
