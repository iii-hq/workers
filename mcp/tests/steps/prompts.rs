//! Step defs for tests/features/prompts.feature.
//!
//! Drives `prompts/list` and `prompts/get` through the dispatcher; the
//! prompts::mcp-* stubs in common/workers.rs return a fixture prompt.

use cucumber::{then, when};
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

use crate::common::world::IiiMcpWorld;

const LAST: &str = "prompts_last_response";

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

#[when("I list prompts")]
async fn list_prompts(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 400, "method": "prompts/list" }),
    )
    .await;
}

#[then(regex = r#"^the prompts listing includes "([^"]+)" with a required argument$"#)]
fn list_has_prompt(world: &mut IiiMcpWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let arr = v["result"]["prompts"]
        .as_array()
        .expect("missing .result.prompts");
    let entry = arr
        .iter()
        .find(|p| p["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("missing {name:?}: {arr:?}"));
    let args = entry["arguments"].as_array().expect("missing .arguments");
    assert!(
        args.iter().any(|a| a["required"].as_bool() == Some(true)),
        "no required arg: {entry}"
    );
}

#[when(regex = r#"^I get the prompt "([^"]+)" with arguments to=([^\s]+)$"#)]
async fn get_prompt(world: &mut IiiMcpWorld, name: String, to: String) {
    call(
        world,
        json!({
            "jsonrpc": "2.0",
            "id": 401,
            "method": "prompts/get",
            "params": { "name": name, "arguments": { "to": to } }
        }),
    )
    .await;
}

#[when("I send prompts/get with no params")]
async fn get_no_params(world: &mut IiiMcpWorld) {
    call(
        world,
        json!({ "jsonrpc": "2.0", "id": 402, "method": "prompts/get" }),
    )
    .await;
}

#[then("the prompt response has a description and a single user message")]
fn prompt_shape(world: &mut IiiMcpWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert!(
        v["result"]["description"].as_str().is_some(),
        "missing description: {v}"
    );
    let msgs = v["result"]["messages"]
        .as_array()
        .expect("missing .result.messages");
    assert_eq!(msgs.len(), 1, "{v}");
    assert_eq!(msgs[0]["role"].as_str(), Some("user"));
}

#[then(regex = r#"^the prompt user message contains "([^"]+)"$"#)]
fn prompt_text_contains(world: &mut IiiMcpWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    let text = v["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing user message text: {v}"));
    assert!(text.contains(&needle), "missing {needle:?}: {text}");
}

#[then(regex = r#"^the response is a JSON-RPC error with code (-?\d+) for prompts$"#)]
fn assert_prompts_error(world: &mut IiiMcpWorld, code: i64) {
    if world.iii.is_none() {
        return;
    }
    let v = world.stash.get(LAST).expect("no response recorded");
    assert_eq!(v["error"]["code"].as_i64(), Some(code), "{v}");
}
