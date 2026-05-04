//! Step defs for tests/features/resources.feature.
//!
//! Drives `resources/list`, `resources/read`, and
//! `resources/templates/list`. The skills::resources-* stubs in
//! common/workers.rs return fixture envelopes the assertions match.

use cucumber::{then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiMcpWorld;

const LAST: &str = "resources_last_response";

async fn call(world: &mut IiiMcpWorld, body: Value) {
    world.stash.remove(LAST);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    if let Ok(envelope) = iii
        .trigger(TriggerRequest {
            function_id: "mcp::handler".into(),
            payload: body,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    {
        let unwrapped = envelope.get("body").cloned().unwrap_or(envelope);
        world.stash.insert(LAST.into(), unwrapped);
    }
}

#[when("I list resources")]
async fn list_resources(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 300, "method": "resources/list" }),
    )
    .await;
}

#[then("the resource listing includes the iii://skills index")]
fn list_has_index(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let arr = v["result"]["resources"]
        .as_array()
        .expect("missing .result.resources");
    assert!(
        arr.iter()
            .any(|r| r["uri"].as_str() == Some("iii://skills")),
        "missing iii://skills: {arr:?}"
    );
}

#[when(regex = r#"^I read the resource "([^"]+)"$"#)]
async fn read_resource(world: &mut IiiMcpWorld, uri: String) {
    call(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 301,
            "method": "resources/read",
            "params": { "uri": uri }
        }),
    )
    .await;
}

#[when("I send resources/read with no params")]
async fn read_no_params(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 302, "method": "resources/read" }),
    )
    .await;
}

#[then(regex = r#"^the resource read mime type is "([^"]+)"$"#)]
fn read_mime(world: &mut IiiMcpWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(
        v["result"]["contents"][0]["mimeType"].as_str(),
        Some(want.as_str()),
        "{v}"
    );
}

#[then(regex = r#"^the resource read text mentions "([^"]+)"$"#)]
fn read_text_mentions(world: &mut IiiMcpWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let text = v["result"]["contents"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing contents.text: {v}"));
    assert!(text.contains(&needle), "missing {needle:?}: {text}");
}

#[when("I list resource templates")]
async fn list_templates(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 303, "method": "resources/templates/list" }),
    )
    .await;
}

#[then("the templates listing has the skill and skill-section URIs")]
fn templates_two(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let arr = v["result"]["resourceTemplates"]
        .as_array()
        .expect("missing .resourceTemplates");
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

#[then(regex = r#"^the response is a JSON-RPC error with code (-?\d+) for resources$"#)]
fn assert_resources_error(world: &mut IiiMcpWorld, code: i64) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(v["error"]["code"].as_i64(), Some(code), "{v}");
}
