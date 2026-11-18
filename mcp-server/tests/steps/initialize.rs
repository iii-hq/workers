//! Step defs for `tests/features/initialize.feature`. Also owns the
//! shared `Given the iii engine is reachable` step every `@engine`
//! feature uses in its Background.

use cucumber::{given, then, when};
use serde_json::json;

use crate::common::http::{last_content_type, last_rpc, last_status, post_jsonrpc};
use crate::common::world::McpWorld;

/// Shared no-op given. Engine reachability is asserted implicitly by
/// every `@engine` step's `if world.iii.is_none() { return; }` guard:
/// when the engine is missing the entire scenario degrades into a
/// soft skip. Defining the step here keeps the cucumber Background
/// readable without forcing a hard failure on hosts without `iii`.
#[given("the iii engine is reachable")]
async fn engine_reachable(_world: &mut McpWorld) {}

#[when("I send an initialize request")]
async fn send_initialize(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )
    .await;
}

#[when(regex = r#"^I send an initialize request claiming protocolVersion "([^"]+)"$"#)]
async fn send_initialize_with_proto(world: &mut McpWorld, claimed: String) {
    post_jsonrpc(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "initialize",
            "params": {"protocolVersion": claimed}
        }),
    )
    .await;
}

#[then("the http response status is 200")]
fn status_is_200(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    assert_eq!(last_status(world), 200);
}

#[then(regex = r#"^the response Content-Type starts with "([^"]+)"$"#)]
fn content_type_starts_with(world: &mut McpWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let ct = last_content_type(world);
    assert!(
        ct.starts_with(&want),
        "expected Content-Type to start with {want:?}, got {ct:?}"
    );
}

#[then(regex = r#"^the JSON-RPC envelope has jsonrpc "([^"]+)" and id (\d+)$"#)]
fn rpc_envelope(world: &mut McpWorld, jsonrpc: String, id: u64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert_eq!(v["jsonrpc"].as_str(), Some(jsonrpc.as_str()));
    assert_eq!(v["id"].as_u64(), Some(id));
}

#[then(regex = r#"^result\.protocolVersion is "([^"]+)"$"#)]
fn protocol_version_is(world: &mut McpWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert_eq!(
        v["result"]["protocolVersion"].as_str(),
        Some(want.as_str()),
        "{v}"
    );
}

#[then(regex = r#"^result\.capabilities\.(\w+) is an object$"#)]
fn capability_object(world: &mut McpWorld, key: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["capabilities"][&key].is_object(),
        "capabilities.{key} is not an object: {v}"
    );
}

#[then(regex = r#"^result\.serverInfo\.name is "([^"]+)"$"#)]
fn server_info_name(world: &mut McpWorld, want: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert_eq!(
        v["result"]["serverInfo"]["name"].as_str(),
        Some(want.as_str()),
        "{v}"
    );
}

#[then("result.serverInfo.version matches the crate version")]
fn server_info_version(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert_eq!(
        v["result"]["serverInfo"]["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "{v}"
    );
}

#[when("I send a ping request")]
async fn send_ping(world: &mut McpWorld) {
    post_jsonrpc(world, json!({"jsonrpc": "2.0", "id": 99, "method": "ping"})).await;
}

#[then("result is an empty object")]
fn result_empty_object(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    let obj = v["result"]
        .as_object()
        .unwrap_or_else(|| panic!("result is not an object: {v}"));
    assert!(obj.is_empty(), "expected empty result object, got {v}");
}
