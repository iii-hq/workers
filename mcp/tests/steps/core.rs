//! Step defs for tests/features/core.feature.
//!
//! Drives `initialize`, `ping`, and unknown-method via the
//! `mcp::handler` function (production code path).

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiMcpWorld;

const LAST: &str = "core_last_response";

/// Trigger `mcp::handler` with `body` and stash the unwrapped JSON-RPC
/// response (i.e. `envelope.body`). On engine absence the scenario will
/// soft-skip later via `world.iii.is_none()` checks.
async fn call_handler(world: &mut IiiMcpWorld, body: Value) {
    world.stash.remove(LAST);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    let res = iii
        .trigger(TriggerRequest {
            function_id: "mcp::handler".into(),
            payload: body,
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    if let Ok(envelope) = res {
        let unwrapped = envelope.get("body").cloned().unwrap_or(envelope);
        world.stash.insert(LAST.into(), unwrapped);
    }
}

#[given("the mcp dispatcher is up")]
fn dispatcher_up(_world: &mut IiiMcpWorld) {
    // No-op; the bdd.rs `before` hook already connected the engine and
    // registered `mcp::handler`. This step exists for Gherkin readability.
}

#[when(regex = r#"^I send the JSON-RPC request "([^"]+)" with id (\d+)$"#)]
async fn send_simple(world: &mut IiiMcpWorld, method: String, id: u64) {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method
    });
    call_handler(world, body).await;
}

#[when("I send a JSON-RPC notification with no id")]
async fn send_notification(world: &mut IiiMcpWorld) {
    let body = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
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
        world.stash.insert(LAST.into(), envelope);
    }
}

#[then("the response advertises protocol version 2025-06-18")]
fn assert_protocol(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(
        v["result"]["protocolVersion"].as_str(),
        Some("2025-06-18"),
        "{v}"
    );
}

#[then("the response advertises the tools, resources, and prompts capabilities")]
fn assert_caps(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let caps = &v["result"]["capabilities"];
    assert!(caps["tools"].is_object(), "{v}");
    assert!(caps["resources"].is_object(), "{v}");
    assert!(caps["prompts"].is_object(), "{v}");
}

#[then("the response result is empty")]
fn assert_empty_result(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let result = &v["result"];
    assert!(result.is_object(), "result must be object: {v}");
    assert_eq!(result.as_object().unwrap().len(), 0, "{v}");
}

#[then(regex = r#"^the response is a JSON-RPC error with code (-?\d+)$"#)]
fn assert_error_code(world: &mut IiiMcpWorld, code: i64) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(v["error"]["code"].as_i64(), Some(code), "{v}");
    assert!(v.get("result").is_none() || v["result"].is_null(), "{v}");
}

#[then("the dispatcher returned a 204 with empty body for the notification")]
fn assert_notification_204(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let envelope = world.stash.get(LAST).expect("no envelope recorded");
    assert_eq!(envelope["status_code"].as_i64(), Some(204), "{envelope}");
    assert!(envelope["body"].is_null(), "{envelope}");
}
