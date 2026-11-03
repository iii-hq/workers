//! @engine steps for bridged-instance stacks: drive mutations through
//! a `BridgeStore`-backed production stack and assert what each
//! instance's local observers received via the main's event feed.
//!
//! Bridge-local deliveries arrive asynchronously (mutation ->
//! publish-events -> main's `session::store::on-events` -> relay ->
//! local emitter), so the receipt assertions poll with a deadline.

use std::time::Duration;

use cucumber::gherkin::Step;
use cucumber::{given, then, when};

use session_manager::events::EventKind;

use super::common_steps::{lookup_path, parse_expected};
use crate::common::bridge::build_bridge_stack;
use crate::common::world::SessionWorld;

#[given(regex = r#"^a bridged instance "([^"]+)"$"#)]
async fn bridged_instance(world: &mut SessionWorld, name: String) {
    if world.skip_without_engine() {
        return;
    }
    let engine = world.engine.clone().expect("guarded above");
    let stack = build_bridge_stack(&engine).await;
    world.bridges.insert(name, stack);
}

#[given(regex = r#"^via bridged instance "([^"]+)" I call "([^"]+)" with:$"#)]
#[when(regex = r#"^via bridged instance "([^"]+)" I call "([^"]+)" with:$"#)]
async fn bridge_call(world: &mut SessionWorld, instance: String, function: String, step: &Step) {
    if world.skip_without_engine() {
        return;
    }
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let payload: serde_json::Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("payload docstring must be valid JSON: {e}"));
    world.call_via_bridge(&instance, &function, payload).await;
}

#[given(
    regex = r#"^a local observer "([^"]+)" of "([^"]+)" in bridged instance "([^"]+)" delivering to "([^"]+)" with filters:$"#
)]
async fn bridge_local_observer(
    world: &mut SessionWorld,
    id: String,
    source: String,
    instance: String,
    target: String,
    step: &Step,
) {
    if world.skip_without_engine() {
        return;
    }
    assert!(
        EventKind::from_function_id(&source).is_some(),
        "{source} is not a session event function"
    );
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let filters: serde_json::Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("filters docstring must be valid JSON: {e}"));
    let stack = world
        .bridges
        .get(&instance)
        .unwrap_or_else(|| panic!("no bridged instance named `{instance}`"));
    stack
        .recorder
        .observe(&id, &source, &filters, &target)
        .expect("bridge-local observer filters must be valid");
}

fn bridge_deliveries(
    world: &SessionWorld,
    instance: &str,
    function_id: &str,
) -> Vec<crate::common::recorder::Delivery> {
    world
        .bridges
        .get(instance)
        .unwrap_or_else(|| panic!("no bridged instance named `{instance}`"))
        .recorder
        .to_function(function_id)
}

#[then(
    regex = r#"^within (\d+) seconds bridged instance "([^"]+)" function "([^"]+)" receives at least (\d+) deliver(?:y|ies)$"#
)]
async fn bridge_receives_at_least(
    world: &mut SessionWorld,
    secs: u64,
    instance: String,
    function_id: String,
    expected: usize,
) {
    if world.skip_without_engine() {
        return;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let got = bridge_deliveries(world, &instance, &function_id);
        if got.len() >= expected {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "bridged instance {instance} function {function_id} received {} deliveries \
                 within {secs}s, expected at least {expected}: {got:#?}",
                got.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[then(
    regex = r#"^after (\d+) seconds? bridged instance "([^"]+)" function "([^"]+)" has received exactly (\d+) deliver(?:y|ies)$"#
)]
async fn bridge_receives_exactly(
    world: &mut SessionWorld,
    secs: u64,
    instance: String,
    function_id: String,
    expected: usize,
) {
    if world.skip_without_engine() {
        return;
    }
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let got = bridge_deliveries(world, &instance, &function_id);
    assert_eq!(
        got.len(),
        expected,
        "bridged instance {instance} function {function_id} received {} deliveries, \
         expected exactly {expected}: {got:#?}",
        got.len()
    );
}

#[then(
    regex = r#"^some delivery to bridged instance "([^"]+)" function "([^"]+)" has "([^"]+)" = (.+)$"#
)]
async fn some_bridge_delivery_has(
    world: &mut SessionWorld,
    instance: String,
    function_id: String,
    path: String,
    raw: String,
) {
    if world.skip_without_engine() {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let got = bridge_deliveries(world, &instance, &function_id);
    assert!(
        got.iter()
            .any(|d| lookup_path(&d.payload, &path) == Some(&expected)),
        "no delivery to {instance}/{function_id} has `{path}` = {expected}: {got:#?}"
    );
}

#[then(
    regex = r#"^every delivery to bridged instance "([^"]+)" function "([^"]+)" has "([^"]+)" = (.+)$"#
)]
async fn every_bridge_delivery_has(
    world: &mut SessionWorld,
    instance: String,
    function_id: String,
    path: String,
    raw: String,
) {
    if world.skip_without_engine() {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let got = bridge_deliveries(world, &instance, &function_id);
    assert!(
        !got.is_empty(),
        "no deliveries to {instance}/{function_id} at all"
    );
    for (i, d) in got.iter().enumerate() {
        assert_eq!(
            lookup_path(&d.payload, &path),
            Some(&expected),
            "delivery {i} to {instance}/{function_id} field `{path}` mismatch: {}",
            d.payload
        );
    }
}
