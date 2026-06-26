//! Steps for trigger bindings and recorded deliveries (@pure).
//!
//! Bindings go through the same parse/validate path the engine-routed
//! `TriggerHandler` uses (`SubscriberSet::add`), and deliveries are
//! whatever the production `Emitter` matched — so these scenarios pin
//! the exact filter semantics subscribers get in production.

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use iii_sdk::trigger::TriggerConfig;

use session_manager::events::EventKind;

use super::common_steps::{lookup_path, parse_expected};
use crate::common::world::SessionWorld;

fn add_binding(
    world: &mut SessionWorld,
    id: &str,
    trigger_type: &str,
    function_id: &str,
    step: &Step,
) -> Result<(), String> {
    let kind = EventKind::from_trigger_type(trigger_type)
        .unwrap_or_else(|| panic!("unknown trigger type {trigger_type}"));
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let config: serde_json::Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("binding config docstring must be valid JSON: {e}"));
    world.emitter.sets().for_kind(kind).add(TriggerConfig {
        id: id.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    })
}

#[given(regex = r#"^a binding "([^"]+)" on "([^"]+)" delivering to "([^"]+)" with config:$"#)]
async fn binding_with_config(
    world: &mut SessionWorld,
    id: String,
    trigger_type: String,
    function_id: String,
    step: &Step,
) {
    let result = add_binding(world, &id, &trigger_type, &function_id, step);
    if let Err(e) = &result {
        panic!("binding `{id}` on {trigger_type} unexpectedly rejected: {e}");
    }
    world.last_binding_result = Some(result);
}

#[when(regex = r#"^I try to bind "([^"]+)" on "([^"]+)" delivering to "([^"]+)" with config:$"#)]
async fn try_bind(
    world: &mut SessionWorld,
    id: String,
    trigger_type: String,
    function_id: String,
    step: &Step,
) {
    let result = add_binding(world, &id, &trigger_type, &function_id, step);
    world.last_binding_result = Some(result);
}

#[then("the binding registration succeeds")]
async fn binding_succeeds(world: &mut SessionWorld) {
    match &world.last_binding_result {
        Some(Ok(())) => {}
        Some(Err(e)) => panic!("expected binding to register, got rejection: {e}"),
        None => panic!("no binding registration was attempted"),
    }
}

#[then(regex = r#"^the binding registration fails mentioning "([^"]+)"$"#)]
async fn binding_fails(world: &mut SessionWorld, fragment: String) {
    match &world.last_binding_result {
        Some(Err(e)) => assert!(
            e.contains(&fragment),
            "rejection `{e}` does not mention `{fragment}`"
        ),
        Some(Ok(())) => {
            panic!("expected binding rejection mentioning `{fragment}`, but it registered")
        }
        None => panic!("no binding registration was attempted"),
    }
}

#[when(regex = r#"^I unbind "([^"]+)" from "([^"]+)"$"#)]
async fn unbind(world: &mut SessionWorld, id: String, trigger_type: String) {
    let kind = EventKind::from_trigger_type(&trigger_type)
        .unwrap_or_else(|| panic!("unknown trigger type {trigger_type}"));
    world.emitter.sets().for_kind(kind).remove(&id);
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
async fn received_n_of_type(
    world: &mut SessionWorld,
    function_id: String,
    expected: usize,
    trigger_type: String,
) {
    let got: Vec<_> = world
        .recorder
        .to_function(&function_id)
        .into_iter()
        .filter(|d| d.trigger_type == trigger_type)
        .collect();
    assert_eq!(
        got.len(),
        expected,
        "function {function_id} received {} {trigger_type} deliveries, expected {expected}: {got:#?}",
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

#[then(regex = r#"^delivery (\d+) to "([^"]+)" is a "([^"]+)" event$"#)]
async fn delivery_is_type(
    world: &mut SessionWorld,
    index: usize,
    function_id: String,
    trigger_type: String,
) {
    let got = world.recorder.to_function(&function_id);
    let delivery = got
        .get(index)
        .unwrap_or_else(|| panic!("function {function_id} has no delivery {index}: {got:#?}"));
    assert_eq!(delivery.trigger_type, trigger_type);
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
