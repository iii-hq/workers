//! Step defs for tests/features/skills_fetch.feature.
//!
//! Drives the batched `skills::fetch_skill` tool and its public alias
//! `skill::fetch` through `iii.trigger(...)`. Both ids share one core
//! fn so most scenarios exercise either id; the alias-parity scenarios
//! re-use the same assertions to confirm they return identical output.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "skills_fetch_last_ok";
const LAST_ERR: &str = "skills_fetch_last_err";

fn seed_slot(name: &str) -> String {
    format!("skills_fetch_seed_id_{name}")
}

async fn trigger_fetch(world: &mut IiiSkillsWorld, function_id: &str, payload: Value) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
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

async fn register_skill(world: &mut IiiSkillsWorld, id: &str, body: &str) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::register".to_string(),
            payload: json!({ "id": id, "skill": body }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
}

fn seeded_id(world: &IiiSkillsWorld, name: &str) -> String {
    world
        .stash
        .get(&seed_slot(name))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default()
}

fn last_text(world: &IiiSkillsWorld) -> String {
    world
        .stash
        .get(LAST_OK)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

// ── seeding ─────────────────────────────────────────────────────────────

#[given(regex = r#"^a fetch-skill seeded as "([^"]+)" with body:$"#)]
async fn seed_named(world: &mut IiiSkillsWorld, name: String, step: &cucumber::gherkin::Step) {
    if world.iii.is_none() {
        return;
    }
    let body = step
        .docstring
        .clone()
        .expect("seed body requires a docstring");
    let id = world.scoped_id(&name);
    world
        .stash
        .insert(seed_slot(&name), Value::String(id.clone()));
    register_skill(world, &id, &body).await;
}

// ── fetch invocations ──────────────────────────────────────────────────

fn build_uris_payload(world: &IiiSkillsWorld, names_csv: &str) -> Value {
    let uris: Vec<String> = names_csv
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|name| format!("iii://{}", seeded_id(world, name)))
        .collect();
    json!({ "uris": uris })
}

#[when(regex = r#"^I call (skills::fetch_skill|skill::fetch) on seeds "([^"]+)"$"#)]
async fn call_on_seeds(world: &mut IiiSkillsWorld, function_id: String, names_csv: String) {
    let payload = build_uris_payload(world, &names_csv);
    trigger_fetch(world, &function_id, payload).await;
}

#[when(
    regex = r#"^I call skills::fetch_skill with both seeds "([^"]+)" in uris and seed "([^"]+)" in uri$"#
)]
async fn call_with_both_uri_and_uris(
    world: &mut IiiSkillsWorld,
    uris_name: String,
    uri_name: String,
) {
    let uris_payload = build_uris_payload(world, &uris_name);
    let single_uri = format!("iii://{}", seeded_id(world, &uri_name));
    let payload = json!({
        "uris": uris_payload["uris"].clone(),
        "uri": single_uri,
    });
    trigger_fetch(world, "skills::fetch_skill", payload).await;
}

#[when(regex = r#"^I call (skills::fetch_skill|skill::fetch) with no uri or uris$"#)]
async fn call_without_input(world: &mut IiiSkillsWorld, function_id: String) {
    trigger_fetch(world, &function_id, json!({})).await;
}

#[when(regex = r#"^I call skills::fetch_skill with the raw URI "([^"]*)"$"#)]
async fn call_with_raw_uri(world: &mut IiiSkillsWorld, uri: String) {
    trigger_fetch(world, "skills::fetch_skill", json!({ "uri": uri })).await;
}

// ── assertions ─────────────────────────────────────────────────────────

#[then("the fetch call succeeds")]
fn fetch_succeeds(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_OK),
        "expected fetch to succeed; got error: {:?}",
        world.stash.get(LAST_ERR)
    );
}

#[then(regex = r#"^the fetch call fails with a message mentioning "([^"]+)"$"#)]
fn fetch_fails(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let err = world
        .stash
        .get(LAST_ERR)
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            panic!(
                "expected fetch to fail; got success: {:?}",
                world.stash.get(LAST_OK)
            )
        });
    assert!(
        err.contains(&needle),
        "expected fetch error to mention {needle:?}; got {err:?}"
    );
}

#[then(regex = r#"^the fetch text contains "([^"]+)"$"#)]
fn fetch_text_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let text = last_text(world);
    assert!(
        text.contains(&needle),
        "expected fetch text to contain {needle:?}; got {text:?}"
    );
}

#[then(regex = r#"^the fetch text does not contain "([^"]+)"$"#)]
fn fetch_text_excludes(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let text = last_text(world);
    assert!(
        !text.contains(&needle),
        "expected fetch text NOT to contain {needle:?}; got {text:?}"
    );
}

#[then(regex = r#"^the fetch text starts with "([^"]+)" followed by the seeded "([^"]+)" id$"#)]
fn fetch_text_starts_with_uri_for_seed(
    world: &mut IiiSkillsWorld,
    prefix: String,
    seed_name: String,
) {
    if world.iii.is_none() {
        return;
    }
    let text = last_text(world);
    let id = seeded_id(world, &seed_name);
    let want = format!("{prefix}{id}");
    assert!(
        text.starts_with(&want),
        "expected fetch text to start with {want:?}; got {text:?}"
    );
}

#[then(regex = r#"^the fetch text has (\d+) sections?$"#)]
fn fetch_text_section_count(world: &mut IiiSkillsWorld, want: usize) {
    if world.iii.is_none() {
        return;
    }
    let text = last_text(world);
    // Sections are joined with `\n\n---\n\n`. n sections => n-1 separators.
    let separators = text.matches("\n\n---\n\n").count();
    let got = if text.is_empty() { 0 } else { separators + 1 };
    assert_eq!(
        got, want,
        "expected {want} sections (separators+1); got {got}. text={text:?}"
    );
}
