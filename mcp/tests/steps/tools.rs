//! Step defs for `tests/features/tools.feature`.
//!
//! Covers `tools/list` envelope shape; `tools/call` invariants are
//! exercised in `errors.feature`.

use cucumber::{then, when};
use serde_json::json;

use crate::common::http::{last_rpc, post_jsonrpc};
use crate::common::world::McpWorld;

#[when("I send a tools/list request")]
async fn send_tools_list(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
}

#[then("result.tools is an array")]
fn result_tools_is_array(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(
        v["result"]["tools"].is_array(),
        "expected result.tools to be an array, got {v:#}"
    );
}

#[then("no advertised tool name starts with a hidden prefix")]
fn no_hidden_prefix_in_tools(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    let arr = v["result"]["tools"].as_array().unwrap_or_else(|| {
        panic!("expected result.tools to be an array, got {v:#}");
    });
    // The MCP tool name encoding maps `::` to `__`, so check the
    // matching prefixes too.
    let bad: &[&str] = &[
        "engine__",
        "state__",
        "stream__",
        "iii.",
        "iii__",
        "mcp__",
        "a2a__",
        "skills__",
        "prompts__",
    ];
    for tool in arr {
        let name = tool["name"].as_str().unwrap_or("");
        for prefix in bad {
            assert!(
                !name.starts_with(prefix),
                "hidden-prefix tool leaked through tools/list: {name:?}"
            );
        }
    }
}
