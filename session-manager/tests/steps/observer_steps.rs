//! Steps for observers of the session event functions and what they
//! received (@pure).
//!
//! Observers live in the scenario's recorder, which applies their filters
//! with iii's filter language to every call the production `Emitter` makes,
//! so these scenarios pin what an observer of each `session::on-*` function
//! receives in production.

use cucumber::gherkin::Step;
use cucumber::{given, then, when};

use session_manager::events::EventKind;

use super::common_steps::{lookup_path, parse_expected};
use crate::common::world::SessionWorld;

fn assert_event_function(function_id: &str) {
    assert!(
        EventKind::from_function_id(function_id).is_some(),
        "{function_id} is not a session event function"
    );
}

#[given(regex = r#"^an observer "([^"]+)" of "([^"]+)" delivering to "([^"]+)" with filters:$"#)]
async fn observer_with_filters(
    world: &mut SessionWorld,
    id: String,
    source: String,
    target: String,
    step: &Step,
) {
    assert_event_function(&source);
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let filters: serde_json::Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("filters docstring must be valid JSON: {e}"));
    world
        .recorder
        .observe(&id, &source, &filters, &target)
        .unwrap_or_else(|e| panic!("observer `{id}` of {source} has invalid filters: {e}"));
}

#[given(regex = r#"^an observer "([^"]+)" of "([^"]+)" delivering to "([^"]+)"$"#)]
async fn observer(world: &mut SessionWorld, id: String, source: String, target: String) {
    assert_event_function(&source);
    world
        .recorder
        .observe(&id, &source, &serde_json::json!({}), &target)
        .expect("{} matches every call");
}

#[when(regex = r#"^I dispose observer "([^"]+)"$"#)]
async fn dispose(world: &mut SessionWorld, id: String) {
    world.recorder.dispose(&id);
}

// ---------------------------------------------------------------------------
// Delivery assertions
// ---------------------------------------------------------------------------

#[then(regex = r#"^function "([^"]+)" received (\d+) deliver(?:y|ies)$"#)]
async fn received_n(world: &mut SessionWorld, function_id: String, expected: usize) {
    let got = world.recorder.to_function(&function_id);
    assert_eq!(
        got.len(),
        expected,
        "function {function_id} received {} deliveries, expected {expected}: {got:#?}",
        got.len()
    );
}

#[then(regex = r#"^function "([^"]+)" received no deliveries$"#)]
async fn received_none(world: &mut SessionWorld, function_id: String) {
    let got = world.recorder.to_function(&function_id);
    assert!(
        got.is_empty(),
        "function {function_id} unexpectedly received {} deliveries: {got:#?}",
        got.len()
    );
}

#[then(regex = r#"^function "([^"]+)" received (\d+) "([^"]+)" deliver(?:y|ies)$"#)]
async fn received_n_from(
    world: &mut SessionWorld,
    function_id: String,
    expected: usize,
    source: String,
) {
    let got: Vec<_> = world
        .recorder
        .to_function(&function_id)
        .into_iter()
        .filter(|d| d.source == source)
        .collect();
    assert_eq!(
        got.len(),
        expected,
        "function {function_id} received {} {source} deliveries, expected {expected}: {got:#?}",
        got.len()
    );
}

#[then("no deliveries were recorded")]
async fn no_deliveries_at_all(world: &mut SessionWorld) {
    let all = world.recorder.all();
    assert!(
        all.is_empty(),
        "expected zero deliveries, got {}: {all:#?}",
        all.len()
    );
}

/// The emitter's gate: an event function nobody observes is never called.
#[then(regex = r#"^"([^"]+)" was not called$"#)]
async fn not_called(world: &mut SessionWorld, function_id: String) {
    let calls = world.recorder.calls_to(&function_id);
    assert!(
        calls.is_empty(),
        "{function_id} was called {} times: {calls:#?}",
        calls.len()
    );
}

#[then(regex = r#"^"([^"]+)" was called (\d+) times?$"#)]
async fn called_n(world: &mut SessionWorld, function_id: String, expected: usize) {
    let calls = world.recorder.calls_to(&function_id);
    assert_eq!(
        calls.len(),
        expected,
        "{function_id} was called {} times, expected {expected}: {calls:#?}",
        calls.len()
    );
}

#[then(regex = r#"^delivery (\d+) to "([^"]+)" is a "([^"]+)" event$"#)]
async fn delivery_is_from(
    world: &mut SessionWorld,
    index: usize,
    function_id: String,
    source: String,
) {
    let got = world.recorder.to_function(&function_id);
    let delivery = got
        .get(index)
        .unwrap_or_else(|| panic!("function {function_id} has no delivery {index}: {got:#?}"));
    assert_eq!(delivery.source, source);
}

#[then(regex = r#"^delivery (\d+) to "([^"]+)" has "([^"]+)" = (.+)$"#)]
async fn delivery_field(
    world: &mut SessionWorld,
    index: usize,
    function_id: String,
    path: String,
    raw: String,
) {
    let expected = parse_expected(&world.substitute(&raw));
    let got = world.recorder.to_function(&function_id);
    let delivery = got
        .get(index)
        .unwrap_or_else(|| panic!("function {function_id} has no delivery {index}: {got:#?}"));
    let actual = lookup_path(&delivery.payload, &path).unwrap_or_else(|| {
        panic!(
            "delivery {index} to {function_id} has no field `{path}`: {}",
            delivery.payload
        )
    });
    assert_eq!(
        actual, &expected,
        "delivery {index} to {function_id} field `{path}` is {actual}, expected {expected}"
    );
}

#[then(regex = r#"^delivery (\d+) to "([^"]+)" has no field "([^"]+)"$"#)]
async fn delivery_field_absent(
    world: &mut SessionWorld,
    index: usize,
    function_id: String,
    path: String,
) {
    let got = world.recorder.to_function(&function_id);
    let delivery = got
        .get(index)
        .unwrap_or_else(|| panic!("function {function_id} has no delivery {index}: {got:#?}"));
    assert!(
        lookup_path(&delivery.payload, &path).is_none(),
        "delivery {index} to {function_id} unexpectedly has field `{path}`: {}",
        delivery.payload
    );
}

#[given("recorded deliveries are cleared")]
#[when("I clear recorded deliveries")]
async fn clear_deliveries(world: &mut SessionWorld) {
    world.recorder.clear();
}
