//! Step defs for `tests/features/registry_*.feature`.
//!
//! Drives `registry::worker-list` and `registry::worker-info` through
//! the same iii engine connection the rest of the BDD harness uses;
//! HTTP is mocked with the shared wiremock server in `common::workers`.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::Value;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, Request, ResponseTemplate};

use crate::common::workers;
use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "registry_last_ok";
const LAST_ERR: &str = "registry_last_err";

async fn call_registry(world: &mut IiiSkillsWorld, function_id: &str, payload: Value) {
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

// ── wiremock setup steps ───────────────────────────────────────────

#[given(regex = r#"^a wiremock registry serving search "([^"]+)" with body:$"#)]
async fn wiremock_search(
    _world: &mut IiiSkillsWorld,
    q: String,
    step: &cucumber::gherkin::Step,
) {
    let body = step.docstring.as_deref().unwrap_or("").to_string();
    let Some(shared) = workers::shared() else {
        return;
    };
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", &q))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "application/json"),
        )
        .mount(&shared.mock_server)
        .await;
}

#[given(regex = r#"^a wiremock registry that returns (\d+) for search "([^"]+)"$"#)]
async fn wiremock_search_status(
    _world: &mut IiiSkillsWorld,
    status: u16,
    q: String,
) {
    let Some(shared) = workers::shared() else {
        return;
    };
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", &q))
        .respond_with(ResponseTemplate::new(status).set_body_string("upstream error"))
        .mount(&shared.mock_server)
        .await;
}

#[given(
    regex = r#"^a wiremock registry serving worker info "([^"]+)" at tag "([^"]+)" with body:$"#
)]
async fn wiremock_worker_info_tag(
    _world: &mut IiiSkillsWorld,
    worker: String,
    tag: String,
    step: &cucumber::gherkin::Step,
) {
    let body = step.docstring.as_deref().unwrap_or("").to_string();
    let Some(shared) = workers::shared() else {
        return;
    };
    Mock::given(method("GET"))
        .and(path(format!("/w/{worker}")))
        .and(query_param("tag", &tag))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "application/json"),
        )
        .mount(&shared.mock_server)
        .await;
}

#[given(
    regex = r#"^a wiremock registry serving worker info "([^"]+)" at version "([^"]+)" with body:$"#
)]
async fn wiremock_worker_info_version(
    _world: &mut IiiSkillsWorld,
    worker: String,
    version: String,
    step: &cucumber::gherkin::Step,
) {
    let body = step.docstring.as_deref().unwrap_or("").to_string();
    let Some(shared) = workers::shared() else {
        return;
    };
    Mock::given(method("GET"))
        .and(path(format!("/w/{worker}")))
        .and(query_param("version", &version))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "application/json"),
        )
        .mount(&shared.mock_server)
        .await;
}

#[given(regex = r#"^a wiremock registry that returns (\d+) for worker info "([^"]+)"$"#)]
async fn wiremock_worker_info_status(
    _world: &mut IiiSkillsWorld,
    status: u16,
    worker: String,
) {
    let Some(shared) = workers::shared() else {
        return;
    };
    Mock::given(method("GET"))
        .and(path(format!("/w/{worker}")))
        .respond_with(ResponseTemplate::new(status).set_body_string("not found"))
        .mount(&shared.mock_server)
        .await;
}

// ── trigger steps ──────────────────────────────────────────────────

#[when(regex = r#"^I trigger registry::worker-list with payload:$"#)]
async fn trigger_worker_list(world: &mut IiiSkillsWorld, step: &cucumber::gherkin::Step) {
    let payload = parse_payload(step);
    call_registry(world, "registry::worker-list", payload).await;
}

#[when(regex = r#"^I trigger registry::worker-info with payload:$"#)]
async fn trigger_worker_info(world: &mut IiiSkillsWorld, step: &cucumber::gherkin::Step) {
    let payload = parse_payload(step);
    call_registry(world, "registry::worker-info", payload).await;
}

// ── outcome assertions ────────────────────────────────────────────

#[then(regex = r#"^the (registry::[a-z\-]+) call succeeds$"#)]
fn registry_succeeds(world: &mut IiiSkillsWorld, _function_id: String) {
    if world.iii.is_none() {
        return;
    }
    assert!(
        world.stash.contains_key(LAST_OK),
        "expected success; got error: {:?}",
        world.stash.get(LAST_ERR)
    );
}

#[then(regex = r#"^the (registry::[a-z\-]+) call fails with a message mentioning "([^"]+)"$"#)]
fn registry_fails(world: &mut IiiSkillsWorld, _function_id: String, needle: String) {
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
        "expected failure; got success: {:?}",
        world.stash.get(LAST_OK)
    );
    assert!(
        err.contains(&needle),
        "expected error to mention {needle:?}; got: {err:?}"
    );
}

fn last_ok(world: &IiiSkillsWorld) -> &Value {
    world
        .stash
        .get(LAST_OK)
        .unwrap_or_else(|| panic!("no successful call recorded; last error: {:?}", world.stash.get(LAST_ERR)))
}

// ── registry::worker-list assertions ──────────────────────────────

#[then(regex = r#"^the registry worker-list response includes worker "([^"]+)"$"#)]
fn worker_list_includes(world: &mut IiiSkillsWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["workers"]
        .as_array()
        .expect("missing workers array");
    let found = arr.iter().any(|w| w["name"].as_str() == Some(name.as_str()));
    assert!(found, "expected {name:?} in worker-list results: {arr:?}");
}

#[then("the registry worker-list response is empty")]
fn worker_list_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["workers"]
        .as_array()
        .expect("missing workers array");
    assert!(arr.is_empty(), "expected empty workers; got: {arr:?}");
}

#[then(regex = r#"^the registry worker-list response worker "([^"]+)" has version "([^"]+)"$"#)]
fn worker_list_version(world: &mut IiiSkillsWorld, name: String, version: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["workers"]
        .as_array()
        .expect("missing workers array");
    let entry = arr
        .iter()
        .find(|w| w["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("{name:?} not in worker-list: {arr:?}"));
    assert_eq!(
        entry["version"].as_str().unwrap_or(""),
        version,
        "version mismatch on entry: {entry:?}"
    );
}

// ── registry::worker-info assertions ──────────────────────────────
//
// The worker-info response shape wraps the worker payload in a
// top-level `worker` field — same shape as directory::worker-info —
// so the assertions read `v["worker"][...]`.

#[then(regex = r#"^the registry worker-info worker name is "([^"]+)"$"#)]
fn wi_worker_name(world: &mut IiiSkillsWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert_eq!(v["worker"]["name"].as_str().unwrap_or(""), name);
}

#[then(regex = r#"^the registry worker-info worker version is "([^"]+)"$"#)]
fn wi_worker_version(world: &mut IiiSkillsWorld, version: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert_eq!(v["worker"]["version"].as_str().unwrap_or(""), version);
}

#[then(regex = r#"^the registry worker-info worker description is "([^"]+)"$"#)]
fn wi_worker_description(world: &mut IiiSkillsWorld, desc: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert_eq!(v["worker"]["description"].as_str().unwrap_or(""), desc);
}

#[then("the registry worker-info response has a non-empty readme")]
fn wi_readme_non_empty(world: &mut IiiSkillsWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let r = v["readme"].as_str().unwrap_or("");
    assert!(!r.is_empty(), "expected non-empty readme; got: {v}");
}

#[then(regex = r#"^the registry worker-info api_reference functions count is (\d+)$"#)]
fn wi_functions_count(world: &mut IiiSkillsWorld, n: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["api_reference"]["functions"]
        .as_array()
        .expect("missing api_reference.functions");
    assert_eq!(arr.len(), n, "functions array: {arr:?}");
}

#[then(regex = r#"^the registry worker-info api_reference triggers count is (\d+)$"#)]
fn wi_triggers_count(world: &mut IiiSkillsWorld, n: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["api_reference"]["triggers"]
        .as_array()
        .expect("missing api_reference.triggers");
    assert_eq!(arr.len(), n, "triggers array: {arr:?}");
}

#[then(regex = r#"^the registry worker-info skills_tree skills count is (\d+)$"#)]
fn wi_skills_tree_count(world: &mut IiiSkillsWorld, n: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = v["skills_tree"]["skills"]
        .as_array()
        .expect("missing skills_tree.skills");
    assert_eq!(arr.len(), n, "skills array: {arr:?}");
}

// ── cache verification ────────────────────────────────────────────

#[then(regex = r#"^the wiremock registry received exactly (\d+) request to "([^"]+)"$"#)]
async fn wiremock_request_count(_world: &mut IiiSkillsWorld, n: usize, path: String) {
    let Some(shared) = workers::shared() else {
        return;
    };
    let received = shared
        .mock_server
        .received_requests()
        .await
        .unwrap_or_default();
    let count = received
        .iter()
        .filter(|r: &&Request| r.url.path() == path)
        .count();
    assert_eq!(
        count, n,
        "expected {n} requests to {path:?}; got {count}: {:?}",
        received.iter().map(|r| r.url.to_string()).collect::<Vec<_>>()
    );
}
