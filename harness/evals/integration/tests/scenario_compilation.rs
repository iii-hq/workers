use harness_integration::fixtures::ScenarioFixture;
use harness_integration::scenarios::RegisteredScenario;
use harness_integration::types::script::JsonMatcherV1;

fn load(entry: &RegisteredScenario) -> ScenarioFixture {
    ScenarioFixture::from_registered(entry)
        .unwrap_or_else(|error| panic!("compiling {}: {error:#}", entry.slug))
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
