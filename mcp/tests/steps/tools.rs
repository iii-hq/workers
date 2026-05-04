//! Step defs for tests/features/tools.feature.
//!
//! Drives `tools/list` and `tools/call` against the in-process
//! `bdd::echo` / `bdd::boom` fixtures plus the hidden-namespace guard.

use cucumber::{given, then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiMcpWorld;

const LAST: &str = "tools_last_response";

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

#[given("the bdd::echo and bdd::boom fixtures are registered")]
fn fixtures_present(_world: &mut IiiMcpWorld) {
    // common/workers.rs registered both at engine init.
}

#[when("I list tools")]
async fn list_tools(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 100, "method": "tools/list" }),
    )
    .await;
}

#[then("the tool listing includes bdd__echo with its inputSchema")]
fn includes_echo(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let tools = v["result"]["tools"].as_array().expect("missing .tools");
    let echo = tools
        .iter()
        .find(|t| t["name"].as_str() == Some("bdd__echo"))
        .unwrap_or_else(|| panic!("bdd__echo not in list: {tools:?}"));
    assert_eq!(echo["inputSchema"]["required"][0], "msg", "{echo}");
}

#[then("the tool listing excludes hidden namespaces")]
fn excludes_hidden(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let tools = v["result"]["tools"].as_array().expect("missing .tools");
    for t in tools {
        let name = t["name"].as_str().unwrap_or("");
        for prefix in [
            "engine__",
            "state__",
            "stream__",
            "iii__",
            "iii.",
            "mcp__",
            "a2a__",
            "skills__",
            "prompts__",
        ] {
            assert!(
                !name.starts_with(prefix),
                "hidden namespace leaked into tools/list: {name}"
            );
        }
    }
}

#[when(regex = r#"^I call the tool "([^"]+)" with arguments \{([^}]*)\}$"#)]
async fn call_tool_inline(world: &mut IiiMcpWorld, name: String, args_csv: String) {
    let args: Value = if args_csv.trim().is_empty() {
        json!({})
    } else {
        // Tiny CSV → JSON for "key=val,key=val" forms used in features.
        let mut obj = serde_json::Map::new();
        for pair in args_csv.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                obj.insert(
                    k.trim().to_string(),
                    Value::String(v.trim().trim_matches('"').to_string()),
                );
            }
        }
        Value::Object(obj)
    };
    call(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 200,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        }),
    )
    .await;
}

#[when("I call tools/call with no params")]
async fn call_tool_no_params(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 201, "method": "tools/call" }),
    )
    .await;
}

#[then(regex = r#"^the tool response text contains "([^"]+)"$"#)]
fn tool_text_contains(world: &mut IiiMcpWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let text = v["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing content text: {v}"));
    assert!(text.contains(&needle), "missing {needle:?}: {text}");
}

#[then("the tool response is marked isError")]
fn tool_is_error(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(v["result"]["isError"].as_bool(), Some(true), "{v}");
}

#[then("the tool response is not marked isError")]
fn tool_not_error(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(v["result"]["isError"].as_bool(), Some(false), "{v}");
}
