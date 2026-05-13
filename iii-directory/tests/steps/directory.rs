//! Step defs for `tests/features/directory_*.feature`.
//!
//! Drives the `directory::*` introspection surface against the same
//! engine connection the rest of the BDD harness uses. Most steps are
//! `When I call directory::<fn> with payload:` followed by `Then the
//! response has …` shape assertions.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::Value;

use crate::common::world::IiiSkillsWorld;

// ── fixture helpers ─────────────────────────────────────────────────

#[given(regex = r#"^a how-to skill file at "([^"]+)" with body:$"#)]
async fn how_to_skill_file_at(
    world: &mut IiiSkillsWorld,
    rel: String,
    step: &cucumber::gherkin::Step,
) {
    let Some(root) = world.skills_folder.as_ref() else {
        return;
    };
    let path = root.join(&rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create_dir_all");
    }
    // Cucumber inserts a leading newline in docstrings; strip it so the
    // YAML frontmatter starts at byte 0 (split_frontmatter is strict
    // about the opening `---\n` fence position).
    let raw = step.docstring.as_deref().unwrap_or("");
    let body = raw.strip_prefix('\n').unwrap_or(raw);
    std::fs::write(&path, body).expect("write fixture");
}

const LAST_OK: &str = "directory_last_ok";
const LAST_ERR: &str = "directory_last_err";

async fn call_directory(world: &mut IiiSkillsWorld, function_id: &str, payload: Value) {
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
            timeout_ms: Some(10_000),
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

fn parse_payload(step: &cucumber::gherkin::Step) -> Value {
    let raw = step.docstring.as_deref().unwrap_or("{}");
    serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("payload docstring is not valid JSON: {raw:?} — {e}"))
}

// ── generic dispatchers ─────────────────────────────────────────────

#[when(regex = r#"^I call (directory::[a-z:\-]+) with payload:$"#)]
async fn call_with_payload(
    world: &mut IiiSkillsWorld,
    function_id: String,
    step: &cucumber::gherkin::Step,
) {
    let payload = parse_payload(step);
    call_directory(world, &function_id, payload).await;
}

#[then(
    regex = r#"^the (directory::engine::[a-z:\-]+) call fails with a message mentioning "([^"]+)"$"#
)]
fn directory_fails(world: &mut IiiSkillsWorld, _function_id: String, needle: String) {
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
        !err.is_empty(),
        "expected an error; got success: {:?}",
        world.stash.get(LAST_OK)
    );
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got: {err:?}"
    );
}

fn last_ok(world: &IiiSkillsWorld) -> &Value {
    world.stash.get(LAST_OK).unwrap_or_else(|| {
        panic!(
            "no successful call recorded; last error: {:?}",
            world.stash.get(LAST_ERR)
        )
    })
}

// ── directory::function-list assertions ──────────────────────────────

fn functions_array(v: &Value) -> &[Value] {
    v["functions"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

#[then(regex = r#"^the directory functions list includes "([^"]+)"$"#)]
fn functions_includes(world: &mut IiiSkillsWorld, function_id: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = functions_array(v);
    let found = arr
        .iter()
        .any(|e| e["function_id"].as_str() == Some(function_id.as_str()));
    assert!(found, "expected {function_id:?} in functions list: {arr:?}");
}

#[then("the directory functions list is empty")]
fn functions_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = functions_array(v);
    assert!(arr.is_empty(), "expected empty list; got: {arr:?}");
}

#[then("the directory functions list is non-empty")]
fn functions_non_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = functions_array(v);
    assert!(!arr.is_empty(), "expected non-empty list");
}

#[then(regex = r#"^every directory functions list entry has function_id prefix "([^"]+)"$"#)]
fn functions_every_prefix(world: &mut IiiSkillsWorld, prefix: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = functions_array(v);
    for entry in arr {
        let fid = entry["function_id"].as_str().unwrap_or("");
        assert!(
            fid.starts_with(&prefix),
            "function_id {fid:?} does not start with {prefix:?}"
        );
    }
}

#[then("every directory functions list entry has a non-null worker_name")]
fn functions_every_worker(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    for entry in functions_array(v) {
        assert!(
            !entry["worker_name"].is_null(),
            "null worker_name in entry: {entry:?}"
        );
    }
}

// ── directory::function-info assertions ──────────────────────────────

#[then(regex = r#"^the directory function-info response has function_id "([^"]+)"$"#)]
fn fi_function_id(world: &mut IiiSkillsWorld, fid: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert_eq!(v["function_id"].as_str().unwrap_or(""), fid);
}

#[then("the directory function-info response has a non-empty description")]
fn fi_description_non_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(
        v["description"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "missing or empty description: {v}"
    );
}

#[then("the directory function-info response has a request_schema")]
fn fi_request_schema(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(!v["request_schema"].is_null(), "missing request_schema");
}

#[then("the directory function-info response has a response_schema")]
fn fi_response_schema(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(!v["response_schema"].is_null(), "missing response_schema");
}

#[then("the directory function-info how_guide is absent")]
fn fi_how_guide_absent(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(
        v.get("how_guide").is_none_or(|g| g.is_null()),
        "expected no how_guide; got: {:?}",
        v.get("how_guide")
    );
}

#[then(regex = r#"^the directory function-info how_guide skill_id is "([^"]+)"$"#)]
fn fi_how_guide_skill_id(world: &mut IiiSkillsWorld, skill_id: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["how_guide"]["skill_id"].as_str().unwrap_or("");
    assert_eq!(got, skill_id, "how_guide payload: {:?}", v["how_guide"]);
}

#[then(regex = r#"^the directory function-info how_guide body contains "([^"]+)"$"#)]
fn fi_how_guide_body_contains(world: &mut IiiSkillsWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let body = v["how_guide"]["body"].as_str().unwrap_or("");
    assert!(
        body.contains(&needle),
        "how_guide body should contain {needle:?}; got: {body:?}"
    );
}

#[then(regex = r#"^the directory function-info how_guide title is "([^"]+)"$"#)]
fn fi_how_guide_title(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["how_guide"]["title"].as_str().unwrap_or("");
    assert_eq!(got, expected, "how_guide payload: {:?}", v["how_guide"]);
}

#[then(regex = r#"^the directory function-info related_skills includes skill_id "([^"]+)"$"#)]
fn fi_related_includes(world: &mut IiiSkillsWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["related_skills"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let found = arr
        .iter()
        .any(|e| e["skill_id"].as_str() == Some(expected.as_str()));
    assert!(
        found,
        "expected related_skills to contain skill_id {expected:?}; got: {arr:?}"
    );
}

#[then(
    regex = r#"^the directory function-info related_skills does not include skill_id "([^"]+)"$"#
)]
fn fi_related_excludes(world: &mut IiiSkillsWorld, excluded: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["related_skills"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let found = arr
        .iter()
        .any(|e| e["skill_id"].as_str() == Some(excluded.as_str()));
    assert!(
        !found,
        "expected related_skills to NOT contain skill_id {excluded:?}; got: {arr:?}"
    );
}

#[then("every directory function-info related_skills entry has a non-empty title")]
fn fi_related_titles_non_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["related_skills"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for entry in arr {
        let title = entry["title"].as_str().unwrap_or("");
        assert!(!title.is_empty(), "missing title in entry: {entry:?}");
    }
}

// ── directory::trigger-list (trigger types) ──────────────────────────

fn triggers_array(v: &Value) -> &[Value] {
    v["triggers"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

#[then(regex = r#"^the directory triggers list includes "([^"]+)"$"#)]
fn triggers_includes(world: &mut IiiSkillsWorld, id: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = triggers_array(v);
    let found = arr.iter().any(|e| e["id"].as_str() == Some(id.as_str()));
    assert!(found, "expected {id:?} in triggers list: {arr:?}");
}

// ── directory::trigger-info ───────────────────────────────────────────

#[then(regex = r#"^the directory trigger-info response has id "([^"]+)"$"#)]
fn ti_id(world: &mut IiiSkillsWorld, id: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert_eq!(v["id"].as_str().unwrap_or(""), id);
}

#[then(regex = r#"^the directory trigger-info response has worker_name "([^"]+)"$"#)]
fn ti_worker_name(world: &mut IiiSkillsWorld, worker: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert_eq!(v["worker_name"].as_str().unwrap_or(""), worker);
}

#[then("the directory trigger-info response has a non-empty description")]
fn ti_description_non_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let d = v["description"].as_str().unwrap_or("");
    assert!(!d.is_empty(), "expected non-empty description; got: {v}");
}

#[then("the directory trigger-info instance_count is a number")]
fn ti_instance_count_number(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(
        v["instance_count"].is_u64() || v["instance_count"].is_i64(),
        "instance_count should be a number; got: {v}"
    );
}

// ── directory::registered-trigger-list ───────────────────────────────

#[then("the directory registered-triggers response has a registered_triggers array")]
fn rt_has_array(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(
        v["registered_triggers"].is_array(),
        "missing registered_triggers array: {v}"
    );
}

#[then("the directory registered-triggers list is empty")]
fn rt_list_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["registered_triggers"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    assert!(arr.is_empty(), "expected empty list; got: {arr:?}");
}

// ── directory::worker-list / worker-info ─────────────────────────────

fn workers_array(v: &Value) -> &[Value] {
    v["workers"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

#[then("the directory workers response has a workers array")]
fn workers_has_array(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(v["workers"].is_array(), "missing workers array: {v}");
}

#[then("the directory workers list is non-empty")]
fn workers_non_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = workers_array(v);
    assert!(!arr.is_empty(), "expected at least one worker");
}

#[then("the directory workers list is empty")]
fn workers_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = workers_array(v);
    assert!(arr.is_empty(), "expected empty list; got: {arr:?}");
}

#[then("every directory workers list entry has a non-empty id")]
fn workers_every_id(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    for entry in workers_array(v) {
        let id = entry["id"].as_str().unwrap_or("");
        assert!(!id.is_empty(), "missing id in entry: {entry:?}");
    }
}

#[then("every directory workers list entry has a non-empty status")]
fn workers_every_status(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    for entry in workers_array(v) {
        let status = entry["status"].as_str().unwrap_or("");
        assert!(!status.is_empty(), "missing status in entry: {entry:?}");
    }
}

#[then("every directory workers list entry has a null description")]
fn workers_every_null_description(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    for entry in workers_array(v) {
        assert!(
            entry["description"].is_null(),
            "expected null description (engine carries none); got: {entry:?}"
        );
    }
}
