//! Step defs for `tests/features/resources.feature`.
//!
//! `resources/*` is delegated to the `skills` worker; when skills isn't
//! registered against the engine the bridge falls through to empty
//! payloads. These scenarios pin the empty-fallback envelope shapes so
//! the bridge stays compatible with MCP clients on a clean engine.

use cucumber::{then, when};
use serde_json::json;

use crate::common::http::{last_rpc, post_jsonrpc};
use crate::common::world::McpWorld;

#[when("I send a resources/list request")]
async fn send_resources_list(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!({"jsonrpc": "2.0", "id": 10, "method": "resources/list"}),
    )
    .await;
}

#[when(regex = r#"^I send a resources/read request for uri "([^"]+)"$"#)]
async fn send_resources_read(world: &mut McpWorld, uri: String) {
    post_jsonrpc(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "resources/read",
            "params": { "uri": uri }
        }),
    )
    .await;
}

#[when("I send a resources/templates/list request")]
async fn send_resources_templates_list(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "resources/templates/list"
        }),
    )
    .await;
}

#[then("result.resources is an array")]
fn result_resources_is_array(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["resources"].is_array(),
        "expected result.resources to be an array: {v:#}"
    );
}

#[then("result.contents is an array")]
fn result_contents_is_array(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["contents"].is_array(),
        "expected result.contents to be an array: {v:#}"
    );
}

#[then("result.resourceTemplates is an array")]
fn result_resource_templates_is_array(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["resourceTemplates"].is_array(),
        "expected result.resourceTemplates to be an array: {v:#}"
    );
}
