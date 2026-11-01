//! Step defs for `tests/features/on_change.feature`.
//!
//! Observes the worker's change functions from the shared connection with
//! `deliver: output`, writes through the worker's own functions, and checks
//! what each observer's target received. Targets are recorder functions
//! registered once per process; the observers a scenario creates are
//! disposed after it (`bdd.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cucumber::{given, then, when};
use iii_sdk::{CallRequest, Deliver, Error, IIIClient, RegisterFunction, Source};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

type Received = Arc<Mutex<Vec<Value>>>;

/// Every recorder registered in this process, by target id.
fn recorders() -> &'static Mutex<HashMap<String, Received>> {
    static RECORDERS: OnceLock<Mutex<HashMap<String, Received>>> = OnceLock::new();
    RECORDERS.get_or_init(Default::default)
}

/// The recorder behind `target`, registered on first use and emptied for
/// the scenario that asks for it.
fn recorder(iii: &IIIClient, target: &str) -> Received {
    let mut all = recorders().lock().unwrap();
    if let Some(received) = all.get(target) {
        received.lock().unwrap().clear();
        return received.clone();
    }
    let received: Received = Arc::default();
    let sink = received.clone();
    iii.register_function(
        target,
        RegisterFunction::new(move |event: Value| {
            sink.lock().unwrap().push(event);
            Ok::<_, Error>(json!({ "ok": true }))
        }),
    );
    all.insert(target.to_string(), received.clone());
    received
}

fn received(target: &str) -> Vec<Value> {
    recorders()
        .lock()
        .unwrap()
        .get(target)
        .map(|received| received.lock().unwrap().clone())
        .unwrap_or_default()
}

async fn observe(world: &mut IiiSkillsWorld, source: Source, target: &str) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    recorder(&iii, target);
    let handle = iii.observe(source.deliver(Deliver::Output), target);
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer neither became active nor failed within 10 s")
        .unwrap_or_else(|e| panic!("observer of {target} failed: {e}"));
    world.observers.push(handle);
}

#[given(regex = r#"^an observer of "([^"]+)" delivering to "([^"]+)"$"#)]
async fn observer_of(world: &mut IiiSkillsWorld, function_id: String, target: String) {
    observe(world, Source::function(function_id), &target).await;
}

#[given(
    regex = r#"^an observer of "([^"]+)" filtered on "([^"]+)" equal to "([^"]+)" delivering to "([^"]+)"$"#
)]
async fn filtered_observer_of(
    world: &mut IiiSkillsWorld,
    function_id: String,
    field: String,
    value: String,
    target: String,
) {
    let filters = json!({ field: { "$eq": value } });
    observe(
        world,
        Source::function(function_id).filters(filters),
        &target,
    )
    .await;
}

/// Gherkin docstrings start with the newline after the opening `"""`;
/// frontmatter must start at byte 0.
fn docstring(step: &cucumber::gherkin::Step) -> String {
    let body = step.docstring.as_deref().unwrap_or("");
    body.strip_prefix('\n').unwrap_or(body).to_string()
}

async fn write(world: &IiiSkillsWorld, function_id: &str, payload: Value) {
    let Some(iii) = world.iii.as_ref() else {
        return;
    };
    iii.call(CallRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .unwrap_or_else(|e| panic!("{function_id}: {e}"));
}

#[when(regex = r#"^I create the skill "([^"]+)" with body:$"#)]
async fn create_skill(world: &mut IiiSkillsWorld, id: String, step: &cucumber::gherkin::Step) {
    let content = docstring(step);
    write(
        world,
        "directory::skills::create",
        json!({ "id": id, "content": content }),
    )
    .await;
}

#[when(regex = r#"^I update the skill "([^"]+)" with body:$"#)]
async fn update_skill(world: &mut IiiSkillsWorld, id: String, step: &cucumber::gherkin::Step) {
    let content = docstring(step);
    write(
        world,
        "directory::skills::update",
        json!({ "id": id, "content": content }),
    )
    .await;
}

#[when(regex = r#"^I delete the skill "([^"]+)"$"#)]
async fn delete_skill(world: &mut IiiSkillsWorld, id: String) {
    write(world, "directory::skills::delete", json!({ "id": id })).await;
}

#[when(regex = r#"^I create the system prompt "([^"]+)" with body:$"#)]
async fn create_system_prompt(
    world: &mut IiiSkillsWorld,
    name: String,
    step: &cucumber::gherkin::Step,
) {
    let content = docstring(step);
    write(
        world,
        "directory::system-prompts::create",
        json!({ "name": name, "content": content }),
    )
    .await;
}

/// Waits until `target` holds `n` events, then a little longer so a
/// delivery beyond `n` shows up too.
async fn settle(target: &str, n: usize) -> Vec<Value> {
    for _ in 0..100 {
        if received(target).len() >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    received(target)
}

#[then(regex = r#"^"([^"]+)" receives (\d+) events?$"#)]
async fn receives_n(world: &mut IiiSkillsWorld, target: String, n: usize) {
    if world.iii.is_none() {
        return;
    }
    let events = settle(&target, n).await;
    assert_eq!(events.len(), n, "{target} received {events:?}");
}

#[then(regex = r#"^"([^"]+)" receives nothing$"#)]
async fn receives_nothing(world: &mut IiiSkillsWorld, target: String) {
    if world.iii.is_none() {
        return;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let events = received(&target);
    assert!(events.is_empty(), "{target} received {events:?}");
}

/// Asserts that `target` received `expected`; the events of separate writes
/// may arrive in any order.
fn assert_received(target: &str, expected: Value) {
    let events = received(target);
    assert!(
        events.contains(&expected),
        "{target} did not receive {expected}; it received {events:?}"
    );
}

#[then(
    regex = r#"^"([^"]+)" received an event with op "([^"]+)", namespace "([^"]+)", name "([^"]+)" and id "([^"]+)"$"#
)]
async fn skill_event_received(
    world: &mut IiiSkillsWorld,
    target: String,
    op: String,
    namespace: String,
    name: String,
    id: String,
) {
    if world.iii.is_none() {
        return;
    }
    assert_received(
        &target,
        json!({ "op": op, "namespace": namespace, "name": name, "id": id }),
    );
}

#[then(regex = r#"^"([^"]+)" received an event with op "([^"]+)" and name "([^"]+)"$"#)]
async fn named_event_received(
    world: &mut IiiSkillsWorld,
    target: String,
    op: String,
    name: String,
) {
    if world.iii.is_none() {
        return;
    }
    assert_received(&target, json!({ "op": op, "name": name }));
}
