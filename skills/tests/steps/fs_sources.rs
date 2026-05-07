//! Step defs for tests/features/fs_sources.feature.
//!
//! These scenarios drive the `skills` worker's filesystem-backed
//! sources end-to-end: a step writes a fixture .md file under the
//! fs root configured at boot, then we trigger a real
//! `skills::list` / `skills::resources-read` / `prompts::mcp-list`
//! / `prompts::mcp-get` call against the running worker and assert
//! on the response. The fs root is wiped between scenarios by the
//! cucumber `before` hook so each scenario starts from an empty
//! file system.

use std::path::PathBuf;

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_LIST: &str = "fs_last_list";
const LAST_READ: &str = "fs_last_read";
const LAST_PROMPT_LIST: &str = "fs_last_prompt_list";
const LAST_PROMPT_GET: &str = "fs_last_prompt_get";
const LAST_ERR: &str = "fs_last_err";

fn skill_path(world: &IiiSkillsWorld, rel: &str) -> Option<PathBuf> {
    world.fs_root.as_ref().map(|r| r.join("fs-skills").join(rel))
}

fn prompt_path(world: &IiiSkillsWorld, rel: &str) -> Option<PathBuf> {
    world
        .fs_root
        .as_ref()
        .map(|r| r.join("fs-prompts").join(rel))
}

fn write_file(path: &PathBuf, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(path, contents).expect("write fixture file");
}

/// Cucumber docstrings start with a leading newline once the opening
/// `"""` line is stripped. Real authors write markdown files starting
/// at byte 0, so for any docstring we strip a single leading newline
/// before persisting — matches what an editor would save.
fn normalize_docstring(s: &str) -> String {
    s.strip_prefix('\n').unwrap_or(s).to_string()
}

// ── seeding ─────────────────────────────────────────────────────────

#[given(regex = r#"^a fs skill file at "([^"]+)" with body:$"#)]
fn seed_fs_skill(
    world: &mut IiiSkillsWorld,
    rel: String,
    step: &cucumber::gherkin::Step,
) {
    let Some(path) = skill_path(world, &rel) else {
        return;
    };
    let body = normalize_docstring(
        step.docstring
            .as_deref()
            .expect("fs skill body requires a docstring"),
    );
    write_file(&path, &body);
}

#[when(regex = r#"^I overwrite the fs skill file at "([^"]+)" with body:$"#)]
fn overwrite_fs_skill(
    world: &mut IiiSkillsWorld,
    rel: String,
    step: &cucumber::gherkin::Step,
) {
    let Some(path) = skill_path(world, &rel) else {
        return;
    };
    let body = normalize_docstring(
        step.docstring
            .as_deref()
            .expect("fs skill body requires a docstring"),
    );
    write_file(&path, &body);
}

#[given(regex = r#"^a fs prompt file at "([^"]+)" with content:$"#)]
fn seed_fs_prompt(
    world: &mut IiiSkillsWorld,
    rel: String,
    step: &cucumber::gherkin::Step,
) {
    let Some(path) = prompt_path(world, &rel) else {
        return;
    };
    let body = normalize_docstring(
        step.docstring
            .as_deref()
            .expect("fs prompt body requires a docstring"),
    );
    write_file(&path, &body);
}

// ── triggers ────────────────────────────────────────────────────────

async fn trigger(
    world: &mut IiiSkillsWorld,
    function_id: &str,
    payload: Value,
    slot: &str,
) {
    world.stash.remove(slot);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(slot.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

// `I list skills`, `I read iii://skills`, `I read the URI "..."`,
// `I register a skill with id "..." and body "..."`, `the
// skills::register call succeeds`, `the contents mime type is "..."`,
// `the contents text contains "..."`, `the contents text does not
// contain "..."` are all defined elsewhere (skills_register.rs +
// skills_resources.rs) — we reuse them.

#[when("I call prompts::mcp-list")]
async fn call_mcp_list(world: &mut IiiSkillsWorld) {
    trigger(world, "prompts::mcp-list", json!({}), LAST_PROMPT_LIST).await;
}

#[when(regex = r#"^I call prompts::mcp-get for "([^"]+)"$"#)]
async fn call_mcp_get(world: &mut IiiSkillsWorld, name: String) {
    trigger(
        world,
        "prompts::mcp-get",
        json!({ "name": name }),
        LAST_PROMPT_GET,
    )
    .await;
}

// ── assertions on skills::list ──────────────────────────────────────

#[then(regex = r#"^the listing has a fs entry with id "([^"]+)" and origin "([^"]+)"$"#)]
fn listing_has_fs_entry(world: &mut IiiSkillsWorld, id: String, origin: String) {
    if world.iii.is_none() {
        return;
    }
    // The shared skills_register step "I list skills" stashes its
    // result under "skills_register_last_list", so we read from that
    // slot rather than maintaining our own.
    let v = world
        .stash
        .get("skills_register_last_list")
        .or_else(|| world.stash.get(LAST_LIST))
        .expect("no skills::list result recorded; call `I list skills` first");
    let arr = v["skills"]
        .as_array()
        .expect("missing .skills array in list response");
    let found = arr
        .iter()
        .find(|e| e["id"].as_str() == Some(id.as_str()));
    let entry = found.unwrap_or_else(|| {
        panic!(
            "no fs entry with id {id:?}; got: {}",
            serde_json::to_string_pretty(arr).unwrap_or_default()
        )
    });
    assert_eq!(
        entry["origin"].as_str(),
        Some(origin.as_str()),
        "wrong origin for {id:?}: {entry}"
    );
}

#[then(regex = r#"^no listing entry has id "([^"]+)"$"#)]
fn listing_lacks_id(world: &mut IiiSkillsWorld, id: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get("skills_register_last_list")
        .or_else(|| world.stash.get(LAST_LIST))
        .expect("no skills::list result recorded");
    let arr = v["skills"].as_array().expect("missing .skills array");
    let found = arr.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
    assert!(
        !found,
        "unexpected listing entry with id {id:?}: {}",
        serde_json::to_string_pretty(arr).unwrap_or_default()
    );
}

// ── assertions on the iii://skills index ───────────────────────────

#[then(regex = r#"^the index has a fs entry "([^"]+)" indented by (\d+) spaces$"#)]
fn index_has_fs_entry(world: &mut IiiSkillsWorld, id: String, indent_spaces: usize) {
    if world.iii.is_none() {
        return;
    }
    // skills_resources.rs stashes the index read under "fs_last_read"
    // for `I read iii://skills`. We use the same slot name for our
    // step, so just look for whichever the latest read populated.
    let v = world
        .stash
        .get("skills_resources_last_read")
        .or_else(|| world.stash.get(LAST_READ))
        .expect("no read recorded — call `I read iii://skills` first");
    let text = v["contents"][0]["text"].as_str().unwrap_or_default();
    let needle_indent = " ".repeat(indent_spaces);
    let bullet_prefix = format!("{needle_indent}- [`{id}`](iii://{id})");
    assert!(
        text.lines().any(|l| l.starts_with(&bullet_prefix)),
        "missing line starting with {bullet_prefix:?} in index:\n{text}"
    );
}

// ── assertions on prompts::mcp-list / prompts::mcp-get ─────────────

#[then(regex = r#"^the mcp prompt list includes "([^"]+)" with description "([^"]+)"$"#)]
fn mcp_list_includes(world: &mut IiiSkillsWorld, name: String, description: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_PROMPT_LIST)
        .expect("no prompts::mcp-list result recorded");
    let arr = v["prompts"]
        .as_array()
        .expect("missing .prompts array in mcp-list response");
    let entry = arr
        .iter()
        .find(|p| p["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| {
            panic!(
                "no prompt named {name:?}; got: {}",
                serde_json::to_string_pretty(arr).unwrap_or_default()
            )
        });
    assert_eq!(
        entry["description"].as_str(),
        Some(description.as_str()),
        "description mismatch for {name:?}: {entry}"
    );
}

#[then(regex = r#"^the mcp prompt list does not include "([^"]+)"$"#)]
fn mcp_list_excludes(world: &mut IiiSkillsWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_PROMPT_LIST)
        .expect("no prompts::mcp-list result recorded");
    let arr = v["prompts"].as_array().expect("missing .prompts array");
    let found = arr
        .iter()
        .any(|p| p["name"].as_str() == Some(name.as_str()));
    assert!(!found, "unexpected prompt {name:?} in: {arr:?}");
}

#[then(regex = r#"^the mcp prompt response description is "([^"]+)"$"#)]
fn mcp_get_description(world: &mut IiiSkillsWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_PROMPT_GET)
        .expect("no prompts::mcp-get result recorded");
    assert_eq!(
        v["description"].as_str(),
        Some(want.as_str()),
        "description mismatch in: {v}"
    );
}

#[then("the mcp prompt response has a single user-text message")]
fn mcp_get_single_user_text(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_PROMPT_GET)
        .expect("no prompts::mcp-get result recorded");
    let messages = v["messages"]
        .as_array()
        .expect("missing .messages in mcp-get response");
    assert_eq!(messages.len(), 1, "expected 1 message; got: {messages:?}");
    assert_eq!(messages[0]["role"], "user", "wrong role: {}", messages[0]);
    assert_eq!(
        messages[0]["content"]["type"], "text",
        "wrong content type: {}",
        messages[0]
    );
}

#[then(regex = r#"^the mcp prompt user-text message contains "([^"]+)"$"#)]
fn mcp_get_text_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_PROMPT_GET)
        .expect("no prompts::mcp-get result recorded");
    let text = v["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.contains(&needle),
        "missing {needle:?} in: {text}"
    );
}
