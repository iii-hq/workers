//! @engine steps: drive the registered production surface through a
//! live iii engine. Every step soft-skips (early-returns) when no
//! engine is reachable, so `cargo test --test bdd` stays green on
//! hosts without `iii`.

use std::time::Duration;

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use serde_json::Value;

use super::common_steps::{lookup_path, parse_expected};
use crate::common::engine::{received_by, register_recorder_function};
use crate::common::world::SessionWorld;

#[given("the iii engine is reachable")]
async fn engine_reachable(_world: &mut SessionWorld) {
    // Soft marker: each step below guards on world.engine itself.
}

#[given(regex = r#"^over the engine I call "([^"]+)" with:$"#)]
#[when(regex = r#"^over the engine I call "([^"]+)" with:$"#)]
async fn engine_call(world: &mut SessionWorld, function: String, step: &Step) {
    let Some(iii) = world.engine.clone() else {
        return;
    };
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let payload: Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("payload docstring must be valid JSON: {e}"));
    match iii
        .trigger(TriggerRequest {
            function_id: function,
            payload,
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
    {
        Ok(v) => {
            world.last_response = Some(v);
            world.last_error = None;
        }
        Err(e) => {
            world.last_response = None;
            world.last_error = Some(e.to_string());
        }
    }
}

#[then("the engine call succeeds")]
async fn engine_call_succeeds(world: &mut SessionWorld) {
    if world.engine.is_none() {
        return;
    }
    if let Some(err) = &world.last_error {
        panic!("expected engine call to succeed, got: {err}");
    }
    assert!(world.last_response.is_some(), "no engine call was made");
}

#[then(regex = r#"^the engine call fails with code "([^"]+)"$"#)]
async fn engine_call_fails(world: &mut SessionWorld, code: String) {
    if world.engine.is_none() {
        return;
    }
    let err = world
        .last_error
        .as_ref()
        .unwrap_or_else(|| panic!("expected engine failure with {code} but the call succeeded"));
    assert!(
        err.contains(&code),
        "expected engine error containing `{code}`, got `{err}`"
    );
}

#[then(regex = r#"^the engine response field "([^"]+)" is (.+)$"#)]
async fn engine_response_field(world: &mut SessionWorld, path: String, raw: String) {
    if world.engine.is_none() {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let response = world
        .last_response
        .as_ref()
        .expect("no engine call was made");
    let actual = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("engine response has no field `{path}`: {response}"));
    assert_eq!(
        actual, &expected,
        "engine response field `{path}` is {actual}, expected {expected}"
    );
}

#[then(regex = r#"^the engine response field "([^"]+)" has length (\d+)$"#)]
async fn engine_response_field_length(world: &mut SessionWorld, path: String, expected: usize) {
    if world.engine.is_none() {
        return;
    }
    let response = world
        .last_response
        .as_ref()
        .expect("no engine call was made");
    let value = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("engine response has no field `{path}`: {response}"));
    let len = value
        .as_array()
        .unwrap_or_else(|| panic!("field `{path}` is not an array: {value}"))
        .len();
    assert_eq!(
        len, expected,
        "field `{path}` has length {len}, expected {expected}"
    );
}

#[then("the engine response is null")]
async fn engine_response_null(world: &mut SessionWorld) {
    if world.engine.is_none() {
        return;
    }
    let response = world
        .last_response
        .as_ref()
        .expect("no engine call was made");
    assert!(
        response.is_null(),
        "expected null engine response, got {response}"
    );
}

#[given(regex = r#"^over the engine I alias the response field "([^"]+)" as "([^"]+)"$"#)]
#[when(regex = r#"^over the engine I alias the response field "([^"]+)" as "([^"]+)"$"#)]
#[then(regex = r#"^over the engine I alias the response field "([^"]+)" as "([^"]+)"$"#)]
async fn engine_alias(world: &mut SessionWorld, path: String, alias: String) {
    if world.engine.is_none() {
        return;
    }
    let response = world
        .last_response
        .as_ref()
        .expect("no engine call was made");
    let value = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("engine response has no field `{path}`: {response}"));
    let rawval = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    world.aliases.insert(alias, rawval);
}

// ---------------------------------------------------------------------------
// Engine subscribers
// ---------------------------------------------------------------------------

#[given(regex = r#"^an engine subscriber "([^"]+)" on "([^"]+)" with config:$"#)]
async fn engine_subscriber(
    world: &mut SessionWorld,
    alias: String,
    trigger_type: String,
    step: &Step,
) {
    let Some(iii) = world.engine.clone() else {
        return;
    };
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let config: Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("subscriber config docstring must be valid JSON: {e}"));

    let function_id = register_recorder_function(&iii);
    // Give the engine a beat to learn about the recorder function
    // before routing events at it.
    tokio::time::sleep(Duration::from_millis(150)).await;

    iii.register_trigger(RegisterTriggerInput {
        trigger_type,
        function_id: function_id.clone(),
        config,
        metadata: None,
    })
    .expect("register engine trigger");
    // And a beat for the registration to round-trip to our handler.
    tokio::time::sleep(Duration::from_millis(250)).await;

    world.engine_subscribers.insert(alias, function_id);
}

fn subscriber_fn(world: &SessionWorld, alias: &str) -> String {
    world
        .engine_subscribers
        .get(alias)
        .unwrap_or_else(|| panic!("no engine subscriber aliased `{alias}`"))
        .clone()
}

#[then(
    regex = r#"^within (\d+) seconds the engine subscriber "([^"]+)" receives at least (\d+) deliver(?:y|ies)$"#
)]
async fn engine_receives_at_least(
    world: &mut SessionWorld,
    secs: u64,
    alias: String,
    expected: usize,
) {
    if world.engine.is_none() {
        return;
    }
    let function_id = subscriber_fn(world, &alias);
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let got = received_by(&function_id);
        if got.len() >= expected {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "engine subscriber {alias} received {} deliveries within {secs}s, expected at least {expected}: {got:#?}",
                got.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[then(
    regex = r#"^after (\d+) seconds? the engine subscriber "([^"]+)" has received exactly (\d+) deliver(?:y|ies)$"#
)]
async fn engine_receives_exactly(
    world: &mut SessionWorld,
    secs: u64,
    alias: String,
    expected: usize,
) {
    if world.engine.is_none() {
        return;
    }
    let function_id = subscriber_fn(world, &alias);
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let got = received_by(&function_id);
    assert_eq!(
        got.len(),
        expected,
        "engine subscriber {alias} received {} deliveries, expected exactly {expected}: {got:#?}",
        got.len()
    );
}

#[then(regex = r#"^some engine delivery to "([^"]+)" has "([^"]+)" = (.+)$"#)]
async fn some_engine_delivery_has(
    world: &mut SessionWorld,
    alias: String,
    path: String,
    raw: String,
) {
    if world.engine.is_none() {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let function_id = subscriber_fn(world, &alias);
    let got = received_by(&function_id);
    assert!(
        got.iter().any(|p| lookup_path(p, &path) == Some(&expected)),
        "no delivery to {alias} has `{path}` = {expected}: {got:#?}"
    );
}

#[then(regex = r#"^every engine delivery to "([^"]+)" has "([^"]+)" = (.+)$"#)]
async fn every_engine_delivery_has(
    world: &mut SessionWorld,
    alias: String,
    path: String,
    raw: String,
) {
    if world.engine.is_none() {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let function_id = subscriber_fn(world, &alias);
    let got = received_by(&function_id);
    assert!(!got.is_empty(), "no deliveries to {alias} at all");
    for (i, p) in got.iter().enumerate() {
        assert_eq!(
            lookup_path(p, &path),
            Some(&expected),
            "delivery {i} to {alias} field `{path}` mismatch: {p}"
        );
    }
}

// ---------------------------------------------------------------------------
// Main-store JSONL readback (the in-process fs-mode registration's
// data_dir — see common::workers::Shared)
// ---------------------------------------------------------------------------

fn main_store_file(world: &SessionWorld, session_id: &str) -> std::path::PathBuf {
    let shared = crate::common::workers::shared().expect("engine registration ran");
    let session_id = world.substitute(session_id);
    shared.data_dir.join(format!(
        "{}.jsonl",
        session_manager::store::encode_session_id(&session_id)
    ))
}

/// All parsed records of one session file, in file (append) order.
fn main_store_records(world: &SessionWorld, session_id: &str) -> Vec<Value> {
    let path = main_store_file(world, session_id);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read main store file {}: {e}", path.display()));
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("malformed JSONL line: {e}")))
        .collect()
}

#[then(regex = r#"^the main store file for session "([^"]+)" exists$"#)]
async fn main_store_file_exists(world: &mut SessionWorld, session_id: String) {
    if world.engine.is_none() {
        return;
    }
    let path = main_store_file(world, &session_id);
    assert!(path.exists(), "expected {} to exist", path.display());
}

#[then(regex = r#"^the main store file for session "([^"]+)" does not exist$"#)]
async fn main_store_file_absent(world: &mut SessionWorld, session_id: String) {
    if world.engine.is_none() {
        return;
    }
    let path = main_store_file(world, &session_id);
    assert!(!path.exists(), "expected {} to be gone", path.display());
}

#[then(regex = r#"^the latest meta record for session "([^"]+)" has "([^"]+)" = (.+)$"#)]
async fn latest_meta_record(
    world: &mut SessionWorld,
    session_id: String,
    path: String,
    raw: String,
) {
    if world.engine.is_none() {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let records = main_store_records(world, &session_id);
    let meta = records
        .iter()
        .rfind(|r| r["type"] == "meta")
        .unwrap_or_else(|| panic!("no meta record in session file: {records:#?}"));
    let actual = lookup_path(&meta["meta"], &path)
        .unwrap_or_else(|| panic!("meta record has no field `{path}`: {meta}"));
    assert_eq!(
        actual, &expected,
        "latest meta record field `{path}` mismatch"
    );
}

#[then(
    regex = r#"^the latest record for entry "([^"]+)" of session "([^"]+)" has "([^"]+)" = (.+)$"#
)]
async fn latest_entry_record(
    world: &mut SessionWorld,
    entry_id: String,
    session_id: String,
    path: String,
    raw: String,
) {
    if world.engine.is_none() {
        return;
    }
    let entry_id = world.substitute(&entry_id);
    let expected = parse_expected(&world.substitute(&raw));
    let records = main_store_records(world, &session_id);
    let entry = records
        .iter()
        .rfind(|r| r["type"] == "entry" && r["entry"]["id"] == entry_id.as_str())
        .unwrap_or_else(|| panic!("no record for entry {entry_id}: {records:#?}"));
    let actual = lookup_path(&entry["entry"], &path)
        .unwrap_or_else(|| panic!("entry record has no field `{path}`: {entry}"));
    assert_eq!(
        actual, &expected,
        "latest record for entry {entry_id} field `{path}` mismatch"
    );
}

#[then(regex = r#"^the latest leaf record for session "([^"]+)" is "([^"]+)"$"#)]
async fn latest_leaf_record(world: &mut SessionWorld, session_id: String, expected: String) {
    if world.engine.is_none() {
        return;
    }
    let expected = world.substitute(&expected);
    let records = main_store_records(world, &session_id);
    let leaf = records
        .iter()
        .rfind(|r| r["type"] == "leaf")
        .unwrap_or_else(|| panic!("no leaf record in session file: {records:#?}"));
    assert_eq!(
        leaf["entry_id"],
        Value::String(expected),
        "latest leaf mismatch"
    );
}
