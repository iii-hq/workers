//! Step defs for `tests/features/prompts.feature`.
//!
//! Drives `prompts::list` and `prompts::get` (the post-MCP-purge,
//! plain-shape API). No MCP envelope or role/messages wrapper.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_LIST: &str = "prompts_last_list";
const LAST_GET: &str = "prompts_last_get";
const LAST_GET_ERR: &str = "prompts_last_get_err";

fn write_prompt_fixture(world: &IiiSkillsWorld, rel: &str, content: &str) {
    let Some(root) = world.skills_folder.as_ref() else {
        return;
    };
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create_dir_all");
    }
    // Cucumber includes a leading newline in docstrings; strip it so
    // the YAML frontmatter starts at byte 0 of the on-disk file.
    let body = content.strip_prefix('\n').unwrap_or(content);
    std::fs::write(&path, body).expect("write fixture");
}

#[given(regex = r#"^a prompt file at "([^"]+)" with content:$"#)]
async fn prompt_file_at(world: &mut IiiSkillsWorld, rel: String, step: &cucumber::gherkin::Step) {
    let body = step.docstring.as_deref().unwrap_or("").to_string();
    write_prompt_fixture(world, &rel, &body);
}

// ── prompts::list ───────────────────────────────────────────────────

#[when("I list prompts")]
async fn list_prompts(world: &mut IiiSkillsWorld) {
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

#[then(regex = r#"^the prompts listing has an entry "([^"]+)" with description "([^"]+)"$"#)]
fn list_has(world: &mut IiiSkillsWorld, name: String, desc: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = v["prompts"].as_array().expect("missing .prompts array");
    let entry = arr
        .iter()
        .find(|e| e["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("prompt {name:?} not in listing: {arr:?}"));
    assert_eq!(entry["description"].as_str().unwrap_or(""), desc);
}

#[then(regex = r#"^the prompts listing entry "([^"]+)" has a non-empty modified_at$"#)]
fn list_modified_nonempty(world: &mut IiiSkillsWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = v["prompts"].as_array().expect("missing .prompts array");
    let entry = arr
        .iter()
        .find(|e| e["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("prompt {name:?} not in listing"));
    let modified = entry["modified_at"].as_str().unwrap_or("");
    assert!(!modified.is_empty(), "modified_at empty for {name:?}");
}

#[then(regex = r#"^the prompts listing does not include "([^"]+)"$"#)]
fn list_lacks(world: &mut IiiSkillsWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = v["prompts"].as_array().expect("missing .prompts array");
    let found = arr
        .iter()
        .any(|e| e["name"].as_str() == Some(name.as_str()));
    assert!(!found, "prompt {name:?} unexpectedly in listing: {arr:?}");
}

// ── prompts::get ────────────────────────────────────────────────────

#[when(regex = r#"^I get prompt "([^"]*)"$"#)]
async fn call_get(world: &mut IiiSkillsWorld, name: String) {
    world.stash.remove(LAST_GET);
    world.stash.remove(LAST_GET_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "prompts::get".to_string(),
            payload: json!({ "name": name }),
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

#[then(regex = r#"^the prompt response has name "([^"]+)"$"#)]
fn get_name(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no prompts::get recorded");
    assert_eq!(v["name"].as_str().unwrap_or(""), expected);
}

#[then(regex = r#"^the prompt response has description "([^"]+)"$"#)]
fn get_description(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no prompts::get recorded");
    assert_eq!(v["description"].as_str().unwrap_or(""), expected);
}

#[then(regex = r#"^the prompt response body contains "([^"]+)"$"#)]
fn get_body_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no prompts::get recorded");
    let body = v["body"].as_str().unwrap_or("");
    assert!(
        body.contains(&needle),
        "expected body to contain {needle:?}; got: {body:?}"
    );
}

#[then("the prompt response has a non-empty modified_at")]
fn get_modified_nonempty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_GET).expect("no prompts::get recorded");
    let modified = v["modified_at"].as_str().unwrap_or("");
    assert!(!modified.is_empty(), "modified_at empty: {v}");
}

#[then(regex = r#"^the prompts::get call fails with a message mentioning "([^"]+)"$"#)]
fn get_fails(world: &mut IiiSkillsWorld, needle: String) {
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
        "expected error; got success: {:?}",
        world.stash.get(LAST_GET)
    );
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got: {err:?}"
    );
}
