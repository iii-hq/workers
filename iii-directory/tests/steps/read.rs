//! Step defs for `tests/features/read.feature`.
//!
//! Drives the read-side surface of the iii-directory worker
//! (`skills::list`, `skill::fetch`) against fixture files written
//! directly into `skills_folder`. The `iii://` URI surface is
//! exercised via `skill::fetch` (no MCP-shaped wrappers exist any
//! more — see [`crate::functions::skills`]).

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_LIST: &str = "read_last_list";
const LAST_FETCH: &str = "read_last_fetch";
const LAST_FETCH_ERR: &str = "read_last_err";

#[given("the iii engine is reachable")]
async fn engine_reachable(_world: &mut IiiSkillsWorld) {}

// ── fixture helpers ─────────────────────────────────────────────────

fn write_fixture(world: &IiiSkillsWorld, rel: &str, body: &str) {
    let Some(root) = world.skills_folder.as_ref() else {
        return;
    };
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create_dir_all");
    }
    std::fs::write(&path, body).expect("write fixture");
}

#[given(regex = r#"^a skill file at "([^"]+)" with body:$"#)]
async fn skill_file_at(world: &mut IiiSkillsWorld, rel: String, step: &cucumber::gherkin::Step) {
    let body = step.docstring.as_deref().unwrap_or("").to_string();
    write_fixture(world, &rel, &body);
}

#[when(regex = r#"^I overwrite the skill file at "([^"]+)" with body:$"#)]
async fn overwrite_skill_file(
    world: &mut IiiSkillsWorld,
    rel: String,
    step: &cucumber::gherkin::Step,
) {
    let body = step.docstring.as_deref().unwrap_or("").to_string();
    write_fixture(world, &rel, &body);
}

// ── skills::list ────────────────────────────────────────────────────

#[when("I list skills")]
async fn list_skills(world: &mut IiiSkillsWorld) {
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

#[then(regex = r#"^the listing has an entry with id "([^"]+)"$"#)]
fn listing_has(world: &mut IiiSkillsWorld, id: String) {
    if world.iii.is_none() {
        return;
    }
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["skills"].as_array().expect("missing .skills array");
    let found = arr.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
    assert!(found, "expected id {id:?} in listing: {arr:?}");
}

#[then(regex = r#"^no listing entry has id "([^"]+)"$"#)]
fn listing_lacks(world: &mut IiiSkillsWorld, id: String) {
    if world.iii.is_none() {
        return;
    }
    let list = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = list["skills"].as_array().expect("missing .skills array");
    let found = arr.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
    assert!(!found, "id {id:?} unexpectedly in listing: {arr:?}");
}

// ── iii:// reads via skill::fetch ──────────────────────────────────

async fn fetch_via_skill_alias(world: &mut IiiSkillsWorld, uris: Vec<String>) {
    world.stash.remove(LAST_FETCH);
    world.stash.remove(LAST_FETCH_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "skill::fetch".to_string(),
            payload: json!({ "uris": uris }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_FETCH.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_FETCH_ERR.into(), Value::String(e.to_string()));
        }
    }
}

#[when(regex = r#"^I read the URI "([^"]+)"$"#)]
async fn read_uri(world: &mut IiiSkillsWorld, uri: String) {
    fetch_via_skill_alias(world, vec![uri]).await;
}

#[when(regex = r#"^I fetch the URIs "([^"]+)"$"#)]
async fn fetch_uris(world: &mut IiiSkillsWorld, csv: String) {
    let uris: Vec<String> = csv.split(',').map(|s| s.trim().to_string()).collect();
    fetch_via_skill_alias(world, uris).await;
}

#[then(regex = r#"^the fetched text contains "([^"]+)"$"#)]
fn fetched_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_FETCH).expect("no fetch recorded");
    let text = v.as_str().unwrap_or_default();
    assert!(
        text.contains(&needle),
        "fetched text should contain {needle:?}; got: {text:?}"
    );
}

#[then(regex = r#"^the fetched text has (\d+) sections joined by "([^"]+)"$"#)]
fn fetched_section_count(world: &mut IiiSkillsWorld, n: usize, sep: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_FETCH).expect("no fetch recorded");
    let text = v.as_str().unwrap_or_default();
    let count = text.matches(sep.as_str()).count();
    // n sections → n-1 joiners
    assert_eq!(
        count,
        n.saturating_sub(1),
        "section-count mismatch in: {text:?}"
    );
}

#[then(regex = r#"^the read fails with a message mentioning "([^"]+)"$"#)]
fn read_fails_mentioning(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let err = world
        .stash
        .get(LAST_FETCH_ERR)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!err.is_empty(), "expected an error; got success");
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got: {err:?}"
    );
}

// ── auto-rendered index assertions ─────────────────────────────────

#[then(regex = r#"^the index has entry "([^"]+)" indented by (\d+) spaces$"#)]
fn index_indented(world: &mut IiiSkillsWorld, id: String, spaces: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_FETCH).expect("no fetch recorded");
    let text = v.as_str().unwrap_or_default();
    let needle_inline = format!("[`{id}`](iii://{id})");
    let line = text
        .lines()
        .find(|l| l.contains(&needle_inline))
        .unwrap_or_else(|| panic!("entry {id:?} not found in index:\n{text}"));
    let actual_indent = line.chars().take_while(|c| *c == ' ').count();
    assert_eq!(
        actual_indent, spaces,
        "indent mismatch for {id:?}: line={line:?}"
    );
}
