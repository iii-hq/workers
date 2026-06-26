//! Step defs for `tests/features/read.feature`.
//!
//! Drives the read-side surface of the iii-directory worker
//! (`directory::skills::list`, `directory::skills::get`) against
//! fixture files written directly into `skills_folder`. The legacy
//! `iii://` URI scheme (rendered tree, function-backed sections,
//! batched fetch) is gone — see [`crate::functions::skills`].

use cucumber::{given, then, when};
use iii_sdk::protocol::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_LIST: &str = "read_last_list";
const LAST_GET: &str = "read_last_get";
const LAST_GET_ERR: &str = "read_last_get_err";
const LAST_INDEX: &str = "read_last_index";

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
    // Gherkin docstrings come back with a leading `\n` (the newline right
    // after the opening `"""`). That trips the YAML frontmatter parser
    // because it expects the body to start with `---\n` at byte 0, so
    // strip it before writing — otherwise scenarios that declare
    // frontmatter never see it round-trip.
    let normalized = body.strip_prefix('\n').unwrap_or(body);
    std::fs::write(&path, normalized).expect("write fixture");
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
            function_id: "directory::skills::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        world.stash.insert(LAST_LIST.into(), v);
    }
}

fn list_entry<'a>(world: &'a IiiSkillsWorld, id: &str) -> Option<&'a Value> {
    let list = world.stash.get(LAST_LIST)?;
    let arr = list.get("skills")?.as_array()?;
    arr.iter().find(|e| e["id"].as_str() == Some(id))
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

#[then(regex = r#"^the listing entry "([^"]+)" has title "([^"]+)"$"#)]
fn listing_entry_title(world: &mut IiiSkillsWorld, id: String, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let entry = list_entry(world, &id).unwrap_or_else(|| panic!("id {id:?} missing from listing"));
    assert_eq!(entry["title"].as_str().unwrap_or(""), expected);
}

#[then(regex = r#"^the listing entry "([^"]+)" has description "([^"]+)"$"#)]
fn listing_entry_description(world: &mut IiiSkillsWorld, id: String, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let entry = list_entry(world, &id).unwrap_or_else(|| panic!("id {id:?} missing from listing"));
    assert_eq!(entry["description"].as_str().unwrap_or(""), expected);
}

#[then(regex = r#"^the listing entry "([^"]+)" has type "([^"]+)"$"#)]
fn listing_entry_type(world: &mut IiiSkillsWorld, id: String, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let entry = list_entry(world, &id).unwrap_or_else(|| panic!("id {id:?} missing from listing"));
    assert_eq!(
        entry["type"].as_str().unwrap_or(""),
        expected,
        "entry: {entry:?}"
    );
}

#[then(regex = r#"^the listing entry "([^"]+)" has a null type$"#)]
fn listing_entry_null_type(world: &mut IiiSkillsWorld, id: String) {
    if world.iii.is_none() {
        return;
    }
    let entry = list_entry(world, &id).unwrap_or_else(|| panic!("id {id:?} missing from listing"));
    assert!(
        entry["type"].is_null(),
        "expected null type for {id:?}; got: {entry:?}"
    );
}

// ── skills::get ─────────────────────────────────────────────────────

#[when(regex = r#"^I get skill "([^"]*)"$"#)]
async fn get_skill(world: &mut IiiSkillsWorld, id: String) {
    world.stash.remove(LAST_GET);
    world.stash.remove(LAST_GET_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::get".to_string(),
            payload: json!({ "id": id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_GET.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_GET_ERR.into(), Value::String(e.to_string()));
        }
    }
}

#[then(regex = r#"^the get response has id "([^"]+)"$"#)]
fn get_id(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no get recorded");
    assert_eq!(v["id"].as_str().unwrap_or(""), expected);
}

#[then(regex = r#"^the get response has title "([^"]+)"$"#)]
fn get_title(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no get recorded");
    assert_eq!(v["title"].as_str().unwrap_or(""), expected);
}

#[then(regex = r#"^the get response has type "([^"]+)"$"#)]
fn get_type(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no get recorded");
    assert_eq!(
        v["type"].as_str().unwrap_or(""),
        expected,
        "response: {v:?}"
    );
}

#[then("the get response has a null type")]
fn get_null_type(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no get recorded");
    assert!(
        v["type"].is_null(),
        "expected null type; got response: {v:?}"
    );
}

#[then(regex = r#"^the get response body contains "([^"]+)"$"#)]
fn get_body_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no get recorded");
    let body = v["body"].as_str().unwrap_or("");
    assert!(
        body.contains(&needle),
        "expected body to contain {needle:?}; got: {body:?}"
    );
}

#[then("the get response has a non-empty modified_at")]
fn get_modified_nonempty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no get recorded");
    let modified = v["modified_at"].as_str().unwrap_or("");
    assert!(!modified.is_empty(), "modified_at empty: {v}");
}

#[then(regex = r#"^the get fails with a message mentioning "([^"]+)"$"#)]
fn get_fails_mentioning(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let err = world
        .stash
        .get(LAST_GET_ERR)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(
        !err.is_empty(),
        "expected an error; got success: {:?}",
        world.stash.get(LAST_GET)
    );
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got: {err:?}"
    );
}

// ── skills::index ───────────────────────────────────────────────────

#[when("I index skills")]
async fn index_skills(world: &mut IiiSkillsWorld) {
    world.stash.remove(LAST_INDEX);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    if let Ok(v) = iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::index".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        world.stash.insert(LAST_INDEX.into(), v);
    }
}

#[then(regex = r#"^the index response has at least (\d+) workers$"#)]
fn index_min_count(world: &mut IiiSkillsWorld, min: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_INDEX).expect("no index recorded");
    let count = v["workers_count"].as_u64().unwrap_or(0) as usize;
    assert!(
        count >= min,
        "expected workers_count >= {min}; got {count} in {v:?}"
    );
}

#[then(regex = r#"^the index response body contains "([^"]+)"$"#)]
fn index_body_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_INDEX).expect("no index recorded");
    let body = v["body"].as_str().unwrap_or("");
    assert!(
        body.contains(&needle),
        "expected index body to contain {needle:?}; got:\n{body}"
    );
}

#[then(regex = r#"^the index response body does not contain "([^"]+)"$"#)]
fn index_body_lacks(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_INDEX).expect("no index recorded");
    let body = v["body"].as_str().unwrap_or("");
    assert!(
        !body.contains(&needle),
        "expected index body NOT to contain {needle:?}; got:\n{body}"
    );
}
