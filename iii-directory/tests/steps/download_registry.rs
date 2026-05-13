//! Step defs for `tests/features/download_registry.feature`.
//!
//! The shared wiremock server is configured as the worker's
//! `registry_url` at registration time (see `common::workers`). Each
//! scenario adds its own per-route mocks; the BDD `before`-hook resets
//! them between scenarios.

use cucumber::{given, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use crate::common::workers;
use crate::common::world::IiiSkillsWorld;

const LAST_OK: &str = "download_last_ok";
const LAST_ERR: &str = "download_last_err";

#[given(
    regex = r#"^a wiremock registry serving worker "([^"]+)" at version "([^"]+)" with body:$"#
)]
async fn wiremock_at_version(
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
        .and(path(format!("/w/{worker}/skills")))
        .and(query_param("version", &version))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "application/json"),
        )
        .mount(&shared.mock_server)
        .await;
}

#[given(regex = r#"^a wiremock registry serving worker "([^"]+)" at tag "([^"]+)" with body:$"#)]
async fn wiremock_at_tag(
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
        .and(path(format!("/w/{worker}/skills")))
        .and(query_param("tag", &tag))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "application/json"),
        )
        .mount(&shared.mock_server)
        .await;
}

#[given(regex = r#"^a wiremock registry that returns 404 for worker "([^"]+)"$"#)]
async fn wiremock_404(_world: &mut IiiSkillsWorld, worker: String) {
    let Some(shared) = workers::shared() else {
        return;
    };
    Mock::given(method("GET"))
        .and(path(format!("/w/{worker}/skills")))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&shared.mock_server)
        .await;
}

async fn run_download(world: &mut IiiSkillsWorld, payload: Value) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::download".to_string(),
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

#[when(regex = r#"^I trigger directory::skills::download with worker="([^"]+)" version="([^"]+)"$"#)]
async fn trigger_with_version(world: &mut IiiSkillsWorld, worker: String, version: String) {
    run_download(world, json!({ "worker": worker, "version": version })).await;
}

#[when(regex = r#"^I trigger directory::skills::download with worker="([^"]+)" tag="([^"]+)"$"#)]
async fn trigger_with_tag(world: &mut IiiSkillsWorld, worker: String, tag: String) {
    run_download(world, json!({ "worker": worker, "tag": tag })).await;
}

#[when(regex = r#"^I trigger directory::skills::download with worker="([^"]+)" alone$"#)]
async fn trigger_alone(world: &mut IiiSkillsWorld, worker: String) {
    run_download(world, json!({ "worker": worker })).await;
}
