//! Steps for trigger bindings and recorded deliveries (@pure).
//!
//! Bindings go through the same parse/validate path the engine-routed
//! `TriggerHandler` uses (`SubscriberSet::add`), and deliveries are
//! whatever the production `Emitter` matched — so these scenarios pin
//! the exact filter semantics subscribers get in production.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use iii_sdk::trigger::TriggerConfig;
use serde_json::Value;

use session_manager::events::{EventDeliverer, EventKind, EventObserver};

use super::common_steps::{lookup_path, parse_expected};
use crate::common::recorder::{Recorder, RecorderDeliverer};
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
        namespace: None,
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

// ---------------------------------------------------------------------------
// Observers of the session::on-* observable functions (iii 0.25)
// ---------------------------------------------------------------------------

/// Stands in for the SDK observer: records each `obs.call` in the world's
/// recorder as a delivery to `observer:<id>`, typed by the observable
/// function, so the delivery steps above read it like a binding's.
struct ObserverSink {
    id: String,
    function_id: &'static str,
    sink: RecorderDeliverer,
}

#[async_trait]
impl EventObserver for ObserverSink {
    fn id(&self) -> &str {
        &self.id
    }

    async fn deliver(&self, event: Value) -> Result<(), String> {
        self.sink
            .deliver(self.function_id, &observer_target(&self.id), event)
            .await;
        Ok(())
    }
}

fn observer_target(id: &str) -> String {
    format!("observer:{id}")
}

fn observable_kind(function_id: &str) -> EventKind {
    EventKind::all()
        .into_iter()
        .find(|kind| kind.observable_function() == function_id)
        .unwrap_or_else(|| panic!("unknown observable function {function_id}"))
}

fn add_observer(
    world: &mut SessionWorld,
    id: &str,
    function_id: &str,
    step: &Step,
) -> Result<(), String> {
    let kind = observable_kind(function_id);
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let args: Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("observer args docstring must be valid JSON: {e}"));
    let observer = ObserverSink {
        id: id.to_string(),
        function_id: kind.observable_function(),
        sink: RecorderDeliverer(world.recorder.clone()),
    };
    world
        .emitter
        .sets()
        .for_kind(kind)
        .add_observer(&args, Arc::new(observer))
}

/// Observer deliveries run on their own task: wait until `expected` have
/// landed (or a deadline passes), then give stragglers a moment to show.
async fn settled_observer_deliveries(recorder: &Recorder, id: &str, expected: usize) -> usize {
    let target = observer_target(id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while recorder.to_function(&target).len() < expected && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    recorder.to_function(&target).len()
}

#[given(regex = r#"^an observer "([^"]+)" of "([^"]+)" with args:$"#)]
async fn observer_with_args(
    world: &mut SessionWorld,
    id: String,
    function_id: String,
    step: &Step,
) {
    if let Err(e) = add_observer(world, &id, &function_id, step) {
        panic!("observer `{id}` of {function_id} unexpectedly rejected: {e}");
    }
}

#[when(regex = r#"^I try to observe "([^"]+)" as "([^"]+)" with args:$"#)]
async fn try_observe(world: &mut SessionWorld, function_id: String, id: String, step: &Step) {
    let result = add_observer(world, &id, &function_id, step);
    world.last_binding_result = Some(result);
}

#[then(regex = r#"^the observer is refused mentioning "([^"]+)"$"#)]
async fn observer_refused(world: &mut SessionWorld, fragment: String) {
    binding_fails(world, fragment).await;
}

#[when(regex = r#"^I remove observer "([^"]+)" from "([^"]+)"$"#)]
async fn remove_observer(world: &mut SessionWorld, id: String, function_id: String) {
    world
        .emitter
        .sets()
        .for_kind(observable_kind(&function_id))
        .remove_observer(&id);
}

#[then(regex = r#"^observer "([^"]+)" received (\d+) deliver(?:y|ies)$"#)]
async fn observer_received_n(world: &mut SessionWorld, id: String, expected: usize) {
    let got = settled_observer_deliveries(&world.recorder, &id, expected).await;
    assert_eq!(
        got,
        expected,
        "observer {id} received {got} deliveries, expected {expected}: {:#?}",
        world.recorder.to_function(&observer_target(&id))
    );
}

#[then(regex = r#"^observer "([^"]+)" received what function "([^"]+)" received$"#)]
async fn observer_received_like_binding(world: &mut SessionWorld, id: String, function_id: String) {
    let bound = world.recorder.to_function(&function_id);
    assert!(
        !bound.is_empty(),
        "function {function_id} received nothing to compare"
    );
    settled_observer_deliveries(&world.recorder, &id, bound.len()).await;
    let observed = world.recorder.to_function(&observer_target(&id));
    let kind = EventKind::all()
        .into_iter()
        .find(|kind| kind.trigger_type() == bound[0].trigger_type)
        .expect("bindings record their trigger type");
    let mut bound: Vec<String> = bound.iter().map(|d| d.payload.to_string()).collect();
    let mut seen: Vec<String> = observed
        .iter()
        .map(|d| {
            assert_eq!(d.trigger_type, kind.observable_function());
            d.payload.to_string()
        })
        .collect();
    // Each observer delivery runs on its own task: compare as multisets.
    bound.sort();
    seen.sort();
    assert_eq!(
        seen, bound,
        "observer {id} and function {function_id} differ"
    );
}

#[given("recorded deliveries are cleared")]
#[when("I clear recorded deliveries")]
async fn clear_deliveries(world: &mut SessionWorld) {
    world.recorder.clear();
}
