//! Steps for `gateway.feature` and the gateway checks added to
//! `initialize.feature`.

use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::{json, Value};

use crate::common::http::{last_header, last_rpc, last_status, post_jsonrpc, send};
use crate::common::world::McpWorld;

const SESSION_KEY: &str = "mcp_session";
const OBSERVER_KEY: &str = "observer_id";

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn session(world: &McpWorld) -> String {
    world
        .stash
        .get(SESSION_KEY)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn scope(world: &McpWorld) -> String {
    format!("mcp-bdd-{}", world.unique_id)
}

async fn tool(world: &mut McpWorld, name: &str, args: Value) {
    let sid = session(world);
    let headers: Vec<(&str, &str)> = if sid.is_empty() {
        vec![]
    } else {
        vec![("mcp-session-id", sid.as_str())]
    };
    send(
        world,
        "POST",
        &headers,
        Some(rpc(
            7,
            "tools/call",
            json!({ "name": name, "arguments": args }),
        )),
    )
    .await;
}

fn tool_text(world: &McpWorld) -> (bool, String) {
    let r = &last_rpc(world)["result"];
    (
        r.get("isError") == Some(&json!(true)),
        r["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

// ----- initialize additions ---------------------------------------------

#[then("result.capabilities has no prompts")]
async fn no_prompts(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(last_rpc(world)["result"]["capabilities"]
        .get("prompts")
        .is_none());
}

#[then("the response carries an Mcp-Session-Id header")]
async fn has_session_header(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    assert!(last_header(world, "mcp-session-id").is_some_and(|s| !s.is_empty()));
}

#[then(regex = r#"^result\.instructions mentions "([^"]+)"$"#)]
async fn instructions_mention(world: &mut McpWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let i = last_rpc(world)["result"]["instructions"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(i.contains(&needle), "{i}");
}

// ----- gateway ------------------------------------------------------------

#[then("the advertised tools start with the gateway set")]
async fn gateway_tools_first(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let names: Vec<String> = last_rpc(world)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let gateway: Vec<String> = iii_mcp::gateway::TOOL_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(&names[..gateway.len()], &gateway[..]);
}

#[when(regex = r#"^I call iii_call with function "([^"]+)"$"#)]
async fn call_iii_call(world: &mut McpWorld, function_id: String) {
    tool(world, "iii_call", json!({ "function_id": function_id })).await;
}

#[then("the tool result is not an error")]
async fn tool_ok(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    let (err, text) = tool_text(world);
    assert!(!err, "{text}");
}

#[then(regex = r#"^the tool result mentions "([^"]+)"$"#)]
async fn tool_mentions(world: &mut McpWorld, needle: String) {
    if world.iii.is_none() {
        return;
    }
    assert!(tool_text(world).1.contains(&needle));
}

#[when(regex = r#"^I send a ping request from Origin "([^"]+)"$"#)]
async fn ping_from_origin(world: &mut McpWorld, origin: String) {
    send(
        world,
        "POST",
        &[("origin", origin.as_str())],
        Some(rpc(3, "ping", json!({}))),
    )
    .await;
}

#[when(regex = r#"^I send a ping request with session "([^"]+)"$"#)]
async fn ping_with_session(world: &mut McpWorld, sid: String) {
    send(
        world,
        "POST",
        &[("mcp-session-id", sid.as_str())],
        Some(rpc(3, "ping", json!({}))),
    )
    .await;
}

#[when("I send a ping request with the ended session")]
async fn ping_ended(world: &mut McpWorld) {
    let sid = session(world);
    send(
        world,
        "POST",
        &[("mcp-session-id", sid.as_str())],
        Some(rpc(3, "ping", json!({}))),
    )
    .await;
}

#[then("the http response status is 403")]
async fn status_403(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    assert_eq!(last_status(world), 403);
}

#[then("the http response status is 404")]
async fn status_404(world: &mut McpWorld) {
    if world.iii.is_none() {
        return;
    }
    assert_eq!(last_status(world), 404);
}

#[given("I open an MCP session")]
async fn open_session(world: &mut McpWorld) {
    post_jsonrpc(world, rpc(1, "initialize", json!({}))).await;
    if world.iii.is_none() {
        return;
    }
    let sid = last_header(world, "mcp-session-id").expect("initialize returns Mcp-Session-Id");
    world.stash.insert(SESSION_KEY.into(), Value::String(sid));
}

#[when(regex = r#"^I observe "([^"]+)" for this scenario's scope$"#)]
async fn observe_scope(world: &mut McpWorld, function_id: String) {
    let scope = scope(world);
    tool(
        world,
        "iii_observe",
        json!({ "function_id": function_id, "filters": { "input.scope": { "$eq": scope } } }),
    )
    .await;
    if world.iii.is_none() {
        return;
    }
    let (err, text) = tool_text(world);
    assert!(!err, "iii_observe failed: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    world
        .stash
        .insert(OBSERVER_KEY.into(), v["observer_id"].clone());
}

#[when("I set a state key in this scenario's scope")]
async fn set_state(world: &mut McpWorld) {
    let scope = scope(world);
    tool(world, "iii_call", json!({ "function_id": "state::set", "payload": { "scope": scope, "key": "k", "value": 1 } })).await;
    if world.iii.is_none() {
        return;
    }
    let (err, text) = tool_text(world);
    assert!(
        !err,
        "state::set failed (is the state worker running?): {text}"
    );
    // A call outside the scope must not be delivered.
    tool(world, "iii_call", json!({ "function_id": "state::set", "payload": { "scope": format!("{scope}-other"), "key": "k", "value": 2 } })).await;
}

#[then(
    regex = r#"^iii_events returns (\d+) events? for this scenario's scope within (\d+) seconds$"#
)]
async fn events_arrive(world: &mut McpWorld, want: usize, secs: u64) {
    if world.iii.is_none() {
        return;
    }
    let oid = world
        .stash
        .get(OBSERVER_KEY)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut seen: Vec<Value> = vec![];
    while Instant::now() < deadline && seen.len() < want {
        tool(world, "iii_events", json!({ "observer_id": oid })).await;
        let v: Value = serde_json::from_str(&tool_text(world).1).unwrap_or_default();
        seen.extend(v["events"].as_array().cloned().unwrap_or_default());
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    tool(world, "iii_events", json!({ "observer_id": oid })).await;
    let v: Value = serde_json::from_str(&tool_text(world).1).unwrap_or_default();
    seen.extend(v["events"].as_array().cloned().unwrap_or_default());
    assert_eq!(seen.len(), want, "events: {seen:?}");
    assert_eq!(seen[0]["input"]["scope"], scope(world));
}

#[when("I end the session with DELETE")]
async fn end_session(world: &mut McpWorld) {
    let sid = session(world);
    send(world, "DELETE", &[("mcp-session-id", sid.as_str())], None).await;
}
