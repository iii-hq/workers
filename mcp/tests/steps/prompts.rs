//! Step defs for `tests/features/prompts.feature`.
//!
//! `prompts/*` delegates to the `skills` worker via `prompts::mcp-list`
//! / `prompts::mcp-get`. These scenarios pin the empty-fallback
//! envelope shapes so the bridge stays compatible with MCP clients on
//! a clean engine where skills isn't registered.

use cucumber::{then, when};
use serde_json::json;

use crate::common::http::{last_rpc, post_jsonrpc};
use crate::common::world::McpWorld;

#[when("I send a prompts/list request")]
async fn send_prompts_list(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!({"jsonrpc": "2.0", "id": 20, "method": "prompts/list"}),
    )
    .await;
}

#[when(regex = r#"^I send a prompts/get request for name "([^"]+)"$"#)]
async fn send_prompts_get(world: &mut McpWorld, name: String) {
    post_jsonrpc(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "prompts/get",
            "params": { "name": name }
        }),
    )
    .await;
}

#[then("result.prompts is an array")]
fn result_prompts_is_array(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["prompts"].is_array(),
        "expected result.prompts to be an array: {v:#}"
    );
}

#[then("result.messages is an array")]
fn result_messages_is_array(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["messages"].is_array(),
        "expected result.messages to be an array: {v:#}"
    );
}
