//! Step defs for tests/features/skills_register.feature.
//!
//! Drives the `skills::register`, `skills::unregister`, and `skills::list`
//! surface through `iii.trigger(...)` so the BDD scenarios exercise the
//! full in-process handler path — including validation, state writes,
//! and the `skills::on-change` fan-out.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_ERR: &str = "skills_register_last_err";
const LAST_OK: &str = "skills_register_last_ok";
const LAST_LIST: &str = "skills_register_last_list";
const REGISTERED_ID: &str = "skills_register_registered_id";
const REGISTERED_AT_FIRST: &str = "skills_register_registered_at_first";

#[given("the iii engine is reachable")]
async fn engine_reachable(_world: &mut IiiSkillsWorld) {}

async fn call_register(world: &mut IiiSkillsWorld, payload: Value) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "skills::register".to_string(),
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

async fn call_unregister(world: &mut IiiSkillsWorld, id: &str) -> Option<Value> {
    let iii = world.iii.clone()?;
    iii.trigger(TriggerRequest {
        function_id: "skills::unregister".to_string(),
        payload: json!({ "id": id }),
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .ok()
}

async fn call_list(world: &mut IiiSkillsWorld) {
    world.stash.remove(LAST_LIST);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    if let Ok(v) = iii
        .trigger(TriggerRequest {
            function_id: "skills::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        world.stash.insert(LAST_LIST.into(), v);
    }
}

// ── register validation ─────────────────────────────────────────────────

#[when(regex = r#"^I register a skill with id "([^"]*)" and body "([^"]*)"$"#)]
async fn register_simple(world: &mut IiiSkillsWorld, id: String, body: String) {
    call_register(world, json!({ "id": id, "skill": body })).await;
}

#[when(regex = r#"^I register a skill with id "([^"]*)" and a body of (\d+) bytes$"#)]
async fn register_sized(world: &mut IiiSkillsWorld, id: String, bytes: usize) {
    let body = "x".repeat(bytes);
    call_register(world, json!({ "id": id, "skill": body })).await;
}

#[when(regex = r#"^I register a skill with id "([^"]*)" and an empty body$"#)]
async fn register_empty_body(world: &mut IiiSkillsWorld, id: String) {
    call_register(world, json!({ "id": id, "skill": "" })).await;
}

#[then("the skills::register call fails")]
fn register_fails(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_ERR),
        "expected skills::register to fail; got success: {:?}",
        world.stash.get(LAST_OK)
    );
}

#[then(regex = r#"^the skills::register error mentions "([^"]+)"$"#)]
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
        "expected skills::register error to mention {needle:?}; got {err:?}"
    );
}

#[then("the skills::register call succeeds")]
fn register_succeeds(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_OK),
        "expected skills::register to succeed; got error: {:?}",
        world.stash.get(LAST_ERR)
    );
}

// ── round-trip scenarios ────────────────────────────────────────────────

#[when(regex = r#"^I register a scoped skill "([^"]*)" with body "([^"]*)"$"#)]
async fn register_scoped(world: &mut IiiSkillsWorld, base: String, body: String) {
    let id = world.scoped_id(&base);
    world
        .stash
        .insert(REGISTERED_ID.into(), Value::String(id.clone()));
    call_register(world, json!({ "id": id, "skill": body })).await;
}

#[when("I record the registered_at timestamp")]
fn record_registered_at(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_OK)
        .cloned()
        .expect("a previous When step should have set LAST_OK");
    let ts = v["registered_at"].as_str().unwrap_or("").to_string();
    world
        .stash
        .insert(REGISTERED_AT_FIRST.into(), Value::String(ts));
}

#[when(regex = r#"^I re-register the scoped skill with body "([^"]*)"$"#)]
async fn reregister_scoped(world: &mut IiiSkillsWorld, body: String) {
    let Some(id_value) = world.stash.get(REGISTERED_ID).cloned() else {
        return;
    };
    let id = id_value.as_str().unwrap_or("").to_string();
    // Ensure the `registered_at` timestamp actually differs on re-register.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    call_register(world, json!({ "id": id, "skill": body })).await;
}

#[then("the re-registered timestamp is different from the first")]
fn timestamps_differ(world: &mut IiiSkillsWorld) {
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
    assert!(!first.is_empty(), "no recorded first timestamp");
    assert!(!latest.is_empty(), "no latest timestamp");
    assert_ne!(first, latest, "timestamps should differ on re-register");
}

// ── unregister ──────────────────────────────────────────────────────────

#[when("I unregister the scoped skill")]
async fn unregister_scoped(world: &mut IiiSkillsWorld) {
    let Some(id_value) = world.stash.get(REGISTERED_ID).cloned() else {
        return;
    };
    let id = id_value.as_str().unwrap_or("").to_string();
    if let Some(v) = call_unregister(world, &id).await {
        world.stash.insert("skills_unreg_last".into(), v);
    }
}

#[when("I unregister the scoped skill again")]
async fn unregister_scoped_twice(world: &mut IiiSkillsWorld) {
    unregister_scoped(world).await
}

#[then(regex = r#"^the last unregister returned removed=(true|false)$"#)]
fn unregister_removed_flag(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get("skills_unreg_last")
        .expect("no unregister result recorded");
    let got = v["removed"].as_bool().unwrap_or(false);
    let want = expected == "true";
    assert_eq!(got, want, "removed flag mismatch: {:?}", v);
}

// ── list ────────────────────────────────────────────────────────────────

#[when("I list skills")]
async fn list_skills(world: &mut IiiSkillsWorld) {
    call_list(world).await;
}

#[then("the scoped skill appears in the listing")]
fn scoped_in_listing(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let id = world
        .stash
        .get(REGISTERED_ID)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["skills"].as_array().expect("missing .skills array");
    let found = arr.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
    assert!(found, "scoped skill {id:?} not in listing: {arr:?}");
}

#[then("the scoped skill does not appear in the listing")]
fn scoped_not_in_listing(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let id = world
        .stash
        .get(REGISTERED_ID)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["skills"].as_array().expect("missing .skills array");
    let found = arr.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
    assert!(!found, "scoped skill {id:?} still in listing: {arr:?}");
}

#[then("the listing entries carry bytes and registered_at but no skill body")]
fn list_metadata_shape(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["skills"].as_array().expect("missing .skills array");
    for entry in arr {
        assert!(entry["bytes"].as_u64().is_some(), "missing bytes: {entry}");
        assert!(
            entry["registered_at"].as_str().is_some(),
            "missing registered_at: {entry}"
        );
        assert!(
            entry.get("skill").is_none(),
            "list must not echo bodies: {entry}"
        );
    }
}

#[then("the listing is sorted by id")]
fn listing_sorted(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["skills"].as_array().expect("missing .skills array");
    let ids: Vec<&str> = arr.iter().filter_map(|e| e["id"].as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "listing not sorted by id: {ids:?}");
}

// ── seed helpers used by ordering tests ─────────────────────────────────

#[given(regex = r#"^a skill "([^"]+)" is already registered$"#)]
async fn seed_skill(world: &mut IiiSkillsWorld, id_base: String) {
    let id = world.scoped_id(&id_base);
    let body = format!("# {id}\nbody\n");
    call_register(world, json!({ "id": id, "skill": body })).await;
}
