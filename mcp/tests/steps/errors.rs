//! Step defs for `tests/features/errors.feature`.
//!
//! Pins the JSON-RPC error envelopes the bridge returns for malformed
//! / unsupported / hidden requests. Codes mirror the JSON-RPC spec
//! constants in [`mcp::jsonrpc`].

use cucumber::{then, when};
use serde_json::json;

use crate::common::http::{last_rpc, post_jsonrpc};
use crate::common::world::McpWorld;

#[when("I send a JSON array body")]
async fn send_json_array(world: &mut McpWorld) {
    post_jsonrpc(
        world,
        json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]),
    )
    .await;
}

#[when("I send a body that is missing the jsonrpc field")]
async fn send_missing_jsonrpc(world: &mut McpWorld) {
    post_jsonrpc(world, json!({"id": 5, "method": "ping"})).await;
}

#[when(regex = r#"^I send an unknown method "([^"]+)"$"#)]
async fn send_unknown_method(world: &mut McpWorld, method: String) {
    post_jsonrpc(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": method
        }),
    )
    .await;
}

#[when(regex = r#"^I send a tools/call with name "([^"]*)"$"#)]
async fn send_tools_call_with_name(world: &mut McpWorld, name: String) {
    let mut params = serde_json::Map::new();
    if !name.is_empty() {
        params.insert("name".into(), serde_json::Value::String(name));
    }
    post_jsonrpc(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": params
        }),
    )
    .await;
}

#[then(regex = r#"^the JSON-RPC error code is (-?\d+)$"#)]
fn error_code(world: &mut McpWorld, code: i64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    let got = v["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("missing error.code: {v}"));
    assert_eq!(got, code, "{v}");
}

#[then(regex = r#"^the JSON-RPC error message contains "([^"]+)"$"#)]
fn error_message_contains(world: &mut McpWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    let msg = v["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("missing error.message: {v}"));
    assert!(
        msg.contains(&needle),
        "error.message {msg:?} does not contain {needle:?}"
    );
}

#[then("the JSON-RPC envelope has no result")]
fn no_result(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_rpc(world);
    assert!(v.get("result").is_none(), "unexpected result field: {v}");
}
