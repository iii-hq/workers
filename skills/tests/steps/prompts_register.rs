//! Step defs for tests/features/prompts_register.feature.
//!
//! Drives `prompts::register`, `prompts::unregister`, and `prompts::list`
//! through the iii bus, covering validation (name, description,
//! function_id, duplicate/empty arguments), round-trip, idempotency.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "prompts_register_last_ok";
const LAST_ERR: &str = "prompts_register_last_err";
const LAST_LIST: &str = "prompts_register_last_list";
const REGISTERED_NAME: &str = "prompts_register_registered_name";
const REGISTERED_AT_FIRST: &str = "prompts_register_registered_at_first";
const UNREG_LAST: &str = "prompts_register_unreg_last";

async fn call_register(world: &mut IiiSkillsWorld, payload: Value) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "prompts::register".to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_OK.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

async fn call_list(world: &mut IiiSkillsWorld) {
    world.stash.remove(LAST_LIST);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    if let Ok(v) = iii
        .trigger(TriggerRequest {
            function_id: "prompts::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        world.stash.insert(LAST_LIST.into(), v);
    }
}

// ── validation ──────────────────────────────────────────────────────────

#[when(
    regex = r#"^I register a prompt with name "([^"]*)", description "([^"]*)", function_id "([^"]*)"$"#
)]
async fn register_simple(
    world: &mut IiiSkillsWorld,
    name: String,
    description: String,
    function_id: String,
) {
    call_register(
        world,
        json!({
            "name": name,
            "description": description,
            "function_id": function_id,
        }),
    )
    .await;
}

#[when("I register a prompt with duplicate argument names")]
async fn register_dup_args(world: &mut IiiSkillsWorld) {
    call_register(
        world,
        json!({
            "name": "dup",
            "description": "dup args",
            "function_id": "test::handler",
            "arguments": [
                { "name": "to", "required": true },
                { "name": "to", "required": false }
            ]
        }),
    )
    .await;
}

#[when("I register a prompt with an empty argument name")]
async fn register_empty_arg(world: &mut IiiSkillsWorld) {
    call_register(
        world,
        json!({
            "name": "dup",
            "description": "empty arg",
            "function_id": "test::handler",
            "arguments": [
                { "name": "", "required": true }
            ]
        }),
    )
    .await;
}

#[then("the prompts::register call fails")]
fn register_fails(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_ERR),
        "expected prompts::register to fail; got success: {:?}",
        world.stash.get(LAST_OK)
    );
}

#[then(regex = r#"^the prompts::register error mentions "([^"]+)"$"#)]
fn register_error_mentions(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let err = world
        .stash
        .get(LAST_ERR)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got {err:?}"
    );
}

#[then("the prompts::register call succeeds")]
fn register_succeeds(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_OK),
        "expected prompts::register to succeed; got error: {:?}",
        world.stash.get(LAST_ERR)
    );
}

// ── round-trip scenarios ────────────────────────────────────────────────

#[when(regex = r#"^I register a scoped prompt "([^"]*)" pointing at "([^"]*)"$"#)]
async fn register_scoped(world: &mut IiiSkillsWorld, base: String, function_id: String) {
    let name = world.scoped_id(&base);
    world
        .stash
        .insert(REGISTERED_NAME.into(), Value::String(name.clone()));
    call_register(
        world,
        json!({
            "name": name,
            "description": format!("scoped prompt {base}"),
            "function_id": function_id,
            "arguments": [
                { "name": "to", "description": "Recipient", "required": true }
            ]
        }),
    )
    .await;
}

#[when("I record the registered_at timestamp for the prompt")]
fn record_registered_at(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_OK).cloned().expect("LAST_OK not set");
    let ts = v["registered_at"].as_str().unwrap_or("").to_string();
    world
        .stash
        .insert(REGISTERED_AT_FIRST.into(), Value::String(ts));
}

#[when(regex = r#"^I re-register the scoped prompt with description "([^"]*)"$"#)]
async fn reregister_scoped(world: &mut IiiSkillsWorld, description: String) {
    let Some(name_value) = world.stash.get(REGISTERED_NAME).cloned() else {
        return;
    };
    let name = name_value.as_str().unwrap_or("").to_string();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    call_register(
        world,
        json!({
            "name": name,
            "description": description,
            "function_id": "test::handler",
        }),
    )
    .await;
}

#[then("the re-registered prompt timestamp is different from the first")]
fn prompt_timestamps_differ(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let first = world
        .stash
        .get(REGISTERED_AT_FIRST)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let latest = world
        .stash
        .get(LAST_OK)
        .and_then(|v| v["registered_at"].as_str().map(String::from))
        .unwrap_or_default();
    assert!(!first.is_empty() && !latest.is_empty());
    assert_ne!(first, latest);
}

// ── unregister ──────────────────────────────────────────────────────────

#[when("I unregister the scoped prompt")]
async fn unregister_scoped(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let Some(name_value) = world.stash.get(REGISTERED_NAME).cloned() else {
        return;
    };
    let name = name_value.as_str().unwrap_or("").to_string();
    if let Ok(v) = iii
        .trigger(TriggerRequest {
            function_id: "prompts::unregister".to_string(),
            payload: json!({ "name": name }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        world.stash.insert(UNREG_LAST.into(), v);
    }
}

#[when("I unregister the scoped prompt again")]
async fn unregister_scoped_twice(world: &mut IiiSkillsWorld) {
    unregister_scoped(world).await
}

#[then(regex = r#"^the last prompt unregister returned removed=(true|false)$"#)]
fn prompt_unregister_removed(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(UNREG_LAST)
        .expect("no prompt unregister result recorded");
    let got = v["removed"].as_bool().unwrap_or(false);
    let want = expected == "true";
    assert_eq!(got, want, "removed flag mismatch: {v:?}");
}

// ── list ────────────────────────────────────────────────────────────────

#[when("I list prompts")]
async fn list_prompts(world: &mut IiiSkillsWorld) {
    call_list(world).await;
}

#[then("the scoped prompt appears in the listing")]
fn scoped_prompt_in_listing(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let name = world
        .stash
        .get(REGISTERED_NAME)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["prompts"].as_array().expect("missing .prompts array");
    let found = arr
        .iter()
        .any(|e| e["name"].as_str() == Some(name.as_str()));
    assert!(found, "scoped prompt {name:?} not in listing: {arr:?}");
}

#[then("the scoped prompt does not appear in the listing")]
fn scoped_prompt_not_in_listing(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let name = world
        .stash
        .get(REGISTERED_NAME)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["prompts"].as_array().expect("missing .prompts array");
    let found = arr
        .iter()
        .any(|e| e["name"].as_str() == Some(name.as_str()));
    assert!(!found, "scoped prompt {name:?} still in listing: {arr:?}");
}

#[then("the prompt listing is sorted by name")]
fn prompt_listing_sorted(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["prompts"].as_array().expect("missing .prompts array");
    let names: Vec<&str> = arr.iter().filter_map(|e| e["name"].as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "prompts not sorted by name: {names:?}");
}

#[then("each prompt listing entry carries arguments count, function_id, and registered_at")]
fn prompt_list_metadata_shape(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["prompts"].as_array().expect("missing .prompts array");
    for entry in arr {
        assert!(
            entry["arguments"].as_u64().is_some(),
            "missing arguments count: {entry}"
        );
        assert!(
            entry["function_id"].as_str().is_some(),
            "missing function_id: {entry}"
        );
        assert!(
            entry["registered_at"].as_str().is_some(),
            "missing registered_at: {entry}"
        );
    }
}

// ── seed helper ─────────────────────────────────────────────────────────

#[given(regex = r#"^a prompt "([^"]+)" is already registered pointing at "([^"]+)"$"#)]
async fn seed_prompt(world: &mut IiiSkillsWorld, name_base: String, function_id: String) {
    let name = world.scoped_id(&name_base);
    call_register(
        world,
        json!({
            "name": name,
            "description": format!("seeded {name_base}"),
            "function_id": function_id,
        }),
    )
    .await;
}
