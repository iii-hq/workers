//! HTTP helpers shared by every `@engine` steps module.
//!
//! Every `When` step that posts a JSON-RPC body to the bridge funnels
//! through [`post_jsonrpc`], which stashes the HTTP status, the
//! response Content-Type, the raw body, and (when present) the parsed
//! JSON-RPC envelope. `Then` steps read those slots back via the
//! `last_*` accessors.

use serde_json::Value;

use crate::common::world::McpWorld;

pub const STATUS_KEY: &str = "http_status";
pub const CT_KEY: &str = "http_content_type";
pub const BODY_KEY: &str = "http_body";
pub const RPC_KEY: &str = "jsonrpc";

/// POST `body` to `world.http_url`, stash the result. Soft-skip
/// (no-op) when the engine is not connected so `@engine` scenarios
/// degrade gracefully on hosts without `iii`.
pub async fn post_jsonrpc(world: &mut McpWorld, body: Value) {
    if world.iii.is_none() {
        return;
    }
    let resp = world
        .http
        .post(&world.http_url)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(body.to_string())
        .send()
        .await
        .expect("POST /mcp");
    let status = resp.status().as_u16();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let text = resp.text().await.expect("body text");

    world
        .stash
        .insert(STATUS_KEY.into(), Value::Number(status.into()));
    world.stash.insert(CT_KEY.into(), Value::String(ct));
    world
        .stash
        .insert(BODY_KEY.into(), Value::String(text.clone()));
    match parse_sse_opt(&text) {
        Some(rpc) => {
            world.stash.insert(RPC_KEY.into(), rpc);
        }
        None => {
            world.stash.remove(RPC_KEY);
        }
    }
}

/// Strip the SSE envelope from `body` and return the JSON-RPC payload.
/// Returns `None` for empty bodies (notification 202 acks have no
/// `data:` line).
pub fn parse_sse_opt(body: &str) -> Option<Value> {
    let line = body.lines().find_map(|l| l.strip_prefix("data: "))?;
    serde_json::from_str(line).ok()
}

pub fn last_status(world: &McpWorld) -> u16 {
    world
        .stash
        .get(STATUS_KEY)
        .and_then(|v| v.as_u64())
        .map(|n| n as u16)
        .expect("no http status recorded; run a When step that posts to /mcp first")
}

pub fn last_content_type(world: &McpWorld) -> &str {
    world
        .stash
        .get(CT_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

pub fn last_body(world: &McpWorld) -> &str {
    world
        .stash
        .get(BODY_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

pub fn last_rpc(world: &McpWorld) -> &Value {
    world.stash.get(RPC_KEY).unwrap_or_else(|| {
        panic!(
            "no JSON-RPC envelope recorded; either the request was a notification (202) \
             or the body was not SSE. raw body: {:?}",
            last_body(world)
        )
    })
}
