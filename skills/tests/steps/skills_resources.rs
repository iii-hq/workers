//! Step defs for tests/features/skills_resources.feature.
//!
//! Exercises the `iii://` resource resolver via `skills::resources-*`:
//! the three URI shapes, output normalization, recursion guard.

use cucumber::{given, then, when};
use iii_sdk::{IIIError, RegisterFunction, TriggerRequest};
use serde_json::{json, Value};

use crate::common::world::IiiSkillsWorld;

const LAST_READ: &str = "skills_resources_last_read";
const LAST_ERR: &str = "skills_resources_last_err";
const LAST_LIST: &str = "skills_resources_last_list";
const LAST_TEMPLATES: &str = "skills_resources_last_templates";
const SECTION_FN: &str = "skills_resources_section_fn";

async fn trigger_json(world: &mut IiiSkillsWorld, function_id: &str, payload: Value, slot: &str) {
    world.stash.remove(slot);
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
            world.stash.insert(slot.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

// ── seeding ─────────────────────────────────────────────────────────────

#[given(regex = r#"^a skill with id "([^"]+)" and body:$"#)]
async fn seed_skill_block(
    world: &mut IiiSkillsWorld,
    id_base: String,
    step: &cucumber::gherkin::Step,
) {
    let id = world.scoped_id(&id_base);
    let body = step
        .docstring
        .clone()
        .expect("skill body requires a docstring");
    world
        .stash
        .insert("resources_seed_id".into(), Value::String(id.clone()));
    trigger_json(
        world,
        "skills::register",
        json!({ "id": id, "skill": body }),
        "resources_seed_ok",
    )
    .await;
}

#[given("a sub-skill function that returns a markdown string")]
async fn seed_section_string(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let fn_id = format!("bdd::section-str-{}", world.unique_id);
    world
        .stash
        .insert(SECTION_FN.into(), Value::String(fn_id.clone()));
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), |_input: Value| async move {
            Ok::<_, IIIError>(Value::String("# str-section\nplain text".into()))
        })
        .description("bdd: returns a markdown string directly"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
}

#[given("a sub-skill function that returns a {content} object")]
async fn seed_section_content(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let fn_id = format!("bdd::section-content-{}", world.unique_id);
    world
        .stash
        .insert(SECTION_FN.into(), Value::String(fn_id.clone()));
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), |_input: Value| async move {
            Ok::<_, IIIError>(json!({ "content": "# object-section\nwrapped" }))
        })
        .description("bdd: returns a {content} object"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
}

#[given("a sub-skill function that returns an arbitrary JSON object")]
async fn seed_section_json(world: &mut IiiSkillsWorld) {
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let fn_id = format!("bdd::section-json-{}", world.unique_id);
    world
        .stash
        .insert(SECTION_FN.into(), Value::String(fn_id.clone()));
    iii.register_function(
        RegisterFunction::new_async(fn_id.clone(), |_input: Value| async move {
            Ok::<_, IIIError>(json!({ "count": 42, "label": "json" }))
        })
        .description("bdd: returns arbitrary JSON"),
    );
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
}

// ── resources-list / templates ──────────────────────────────────────────

#[when("I call skills::resources-list")]
async fn call_resources_list(world: &mut IiiSkillsWorld) {
    trigger_json(world, "skills::resources-list", json!({}), LAST_LIST).await;
}

#[then("the resource list includes iii://skills")]
fn list_includes_index(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = v["resources"].as_array().expect("missing .resources");
    assert!(
        arr.iter()
            .any(|r| r["uri"].as_str() == Some("iii://skills")),
        "missing iii://skills index: {arr:?}"
    );
}

#[then("the resource list includes the seeded skill URI")]
fn list_includes_seeded(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let id = world
        .stash
        .get("resources_seed_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let want = format!("iii://{id}");
    let v = world.stash.get(LAST_LIST).expect("no list recorded");
    let arr = v["resources"].as_array().expect("missing .resources");
    assert!(
        arr.iter().any(|r| r["uri"].as_str() == Some(want.as_str())),
        "missing {want:?}: {arr:?}"
    );
}

#[when("I call skills::resources-templates")]
async fn call_resources_templates(world: &mut IiiSkillsWorld) {
    trigger_json(
        world,
        "skills::resources-templates",
        json!({}),
        LAST_TEMPLATES,
    )
    .await;
}

#[then("the templates listing contains the skill and skill-section templates")]
fn templates_two_entries(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world
        .stash
        .get(LAST_TEMPLATES)
        .expect("no templates recorded");
    let arr = v["resourceTemplates"]
        .as_array()
        .expect("missing .resourceTemplates");
    assert_eq!(arr.len(), 2, "expected 2 templates, got {arr:?}");
    let patterns: Vec<&str> = arr
        .iter()
        .filter_map(|t| t["uriTemplate"].as_str())
        .collect();
    assert!(
        patterns.contains(&"iii://{skill_id}"),
        "patterns: {patterns:?}"
    );
    assert!(
        patterns.contains(&"iii://{skill_id}/{function_id}"),
        "patterns: {patterns:?}"
    );
}

// ── resources-read ──────────────────────────────────────────────────────

#[when("I read iii://skills")]
async fn read_index(world: &mut IiiSkillsWorld) {
    trigger_json(
        world,
        "skills::resources-read",
        json!({ "uri": "iii://skills" }),
        LAST_READ,
    )
    .await;
}

#[when("I read the seeded skill URI")]
async fn read_seeded(world: &mut IiiSkillsWorld) {
    let id = world
        .stash
        .get("resources_seed_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let uri = format!("iii://{id}");
    trigger_json(
        world,
        "skills::resources-read",
        json!({ "uri": uri }),
        LAST_READ,
    )
    .await;
}

#[when(regex = r#"^I read the section URI with skill id "([^"]*)"$"#)]
async fn read_section(world: &mut IiiSkillsWorld, skill_id_base: String) {
    let skill_id = if skill_id_base.is_empty() {
        "anything".to_string()
    } else {
        world.scoped_id(&skill_id_base)
    };
    let fn_id = world
        .stash
        .get(SECTION_FN)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let uri = format!("iii://{skill_id}/{fn_id}");
    trigger_json(
        world,
        "skills::resources-read",
        json!({ "uri": uri }),
        LAST_READ,
    )
    .await;
}

#[when(regex = r#"^I read the URI "([^"]+)"$"#)]
async fn read_raw(world: &mut IiiSkillsWorld, uri: String) {
    trigger_json(
        world,
        "skills::resources-read",
        json!({ "uri": uri }),
        LAST_READ,
    )
    .await;
}

// ── assertions on the last read ─────────────────────────────────────────

#[then(regex = r#"^the contents mime type is "([^"]+)"$"#)]
fn read_mime(world: &mut IiiSkillsWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_READ).expect("no read recorded");
    let mime = v["contents"][0]["mimeType"].as_str().unwrap_or("");
    assert_eq!(mime, want, "mime mismatch in: {v}");
}

#[then(regex = r#"^the contents text contains "([^"]+)"$"#)]
fn read_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_READ).expect("no read recorded");
    let text = v["contents"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains(&needle), "missing {needle:?}: {text}");
}

#[then(regex = r#"^the contents text does not contain "([^"]+)"$"#)]
fn read_not_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST_READ).expect("no read recorded");
    let text = v["contents"][0]["text"].as_str().unwrap_or("");
    assert!(!text.contains(&needle), "unexpected {needle:?} in: {text}");
}

#[then(regex = r#"^the read fails with a message mentioning "([^"]+)"$"#)]
fn read_error_mentions(world: &mut IiiSkillsWorld, needle: String) {
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
