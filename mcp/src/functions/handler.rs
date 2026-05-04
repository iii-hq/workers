//! `mcp::handler` JSON-RPC dispatcher.
//!
//! Reachable two ways:
//!
//!   1. Over HTTP — the iii engine wraps each `POST /<api_path>` request in
//!      its trigger-input envelope (`{ body: ..., headers: ..., ... }`), so
//!      we strip that off before parsing the JSON-RPC frame.
//!   2. Direct invocation via `iii.trigger("mcp::handler", body)` — used by
//!      the BDD tests; we treat the entire input as the JSON-RPC body when
//!      no `body` field is present.
//!
//! Each MCP method routes to either an inline reply or a single
//! `iii.trigger` call. The dispatcher is intentionally flat: every match
//! arm is short, every helper a few lines. If you find yourself reaching
//! for shared state, add it to the `Ctx` struct rather than threading
//! more arguments through the helpers.

use std::sync::Arc;

use iii_sdk::{FunctionInfo, IIIError, TriggerRequest, III};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::McpConfig;
use crate::protocol::{
    self, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND, PARSE_ERROR,
};

/// Shared dispatcher state. `iii` is the engine connection; `cfg` is the
/// frozen `McpConfig` snapshot.
pub struct Ctx {
    pub iii: Arc<III>,
    pub cfg: Arc<McpConfig>,
}

/// Entry point called by the registered `mcp::handler` function. `input`
/// is the iii HTTP-trigger envelope (or the raw JSON-RPC body for direct
/// invocation). The returned value is the HTTP response envelope —
/// `{status_code, headers, body}` for HTTP, or the bare `body` value for
/// direct callers (we always emit the HTTP envelope; in-process callers
/// can dig into `.body`).
pub async fn handle(ctx: &Ctx, input: Value) -> Result<Value, IIIError> {
    // The iii HTTP trigger sends `{ body, headers, ..., method }` to the
    // function. The function stack also lets us call it via
    // `iii.trigger("mcp::handler", { ...jsonrpc... })`, which is what the
    // BDD harness does. Detect both shapes.
    let body = input
        .get("body")
        .cloned()
        .filter(|v| !v.is_null())
        .unwrap_or(input);

    let body = match parse_body(body) {
        Ok(b) => b,
        Err(parse_err_response) => {
            return Ok(http_envelope(parse_err_response));
        }
    };

    let response = dispatch(ctx, body).await;
    match response {
        Some(r) => Ok(http_envelope(r)),
        // JSON-RPC notifications get an HTTP 204-style empty body. The
        // iii HTTP trigger contract still wants a `{status_code,...}`
        // envelope, so respond with 204 + empty body.
        None => Ok(json!({
            "status_code": 204,
            "headers": { "content-type": "application/json" },
            "body": Value::Null
        })),
    }
}

fn parse_body(body: Value) -> Result<Value, Value> {
    // Accept the body either as already-parsed JSON or as a raw string
    // the engine forwarded verbatim. Reject anything else with
    // -32700 PARSE_ERROR per JSON-RPC 2.0.
    match body {
        Value::String(s) => match serde_json::from_str::<Value>(&s) {
            Ok(v) => Ok(v),
            Err(e) => Err(json!(JsonRpcResponse::error(
                None,
                PARSE_ERROR,
                format!("Parse error: {e}")
            ))),
        },
        Value::Object(_) | Value::Array(_) => Ok(body),
        Value::Null => Err(json!(JsonRpcResponse::error(
            None,
            PARSE_ERROR,
            "Empty request body"
        ))),
        other => Err(json!(JsonRpcResponse::error(
            None,
            PARSE_ERROR,
            format!("Request body must be JSON, got {other}")
        ))),
    }
}

fn http_envelope(body: Value) -> Value {
    json!({
        "status_code": 200,
        "headers": { "content-type": "application/json" },
        "body": body
    })
}

/// Dispatch a single JSON-RPC frame. Returns `None` for notifications
/// (no response per JSON-RPC 2.0).
pub async fn dispatch(ctx: &Ctx, body: Value) -> Option<Value> {
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").cloned();
    let params = body.get("params").cloned();

    // Notifications are MCP/JSON-RPC messages with no `id`. Per spec we
    // never respond. We don't need to track per-session state in v0.1
    // (no `notifications/cancelled`, no `notifications/initialized` gating)
    // so the easiest correct behaviour is to swallow them.
    if method.starts_with("notifications/") || id.is_none() {
        if method.starts_with("notifications/") {
            tracing::debug!(method, "notification received (no response)");
        }
        return None;
    }

    let result = match method {
        "initialize" => Ok(protocol::initialize_result()),
        "ping" => Ok(json!({})),

        "tools/list" => tools_list(ctx).await,
        "tools/call" => tools_call(ctx, params).await,

        "resources/list" => delegate(ctx, "skills::resources-list", json!({})).await,
        "resources/read" => resources_read(ctx, params).await,
        "resources/templates/list" => delegate(ctx, "skills::resources-templates", json!({})).await,

        "prompts/list" => delegate(ctx, "prompts::mcp-list", json!({})).await,
        "prompts/get" => prompts_get(ctx, params).await,

        other => {
            return Some(json!(JsonRpcResponse::error(
                id,
                METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
            )));
        }
    };

    Some(match result {
        Ok(v) => json!(JsonRpcResponse::success(id, v)),
        Err((code, msg)) => json!(JsonRpcResponse::error(id, code, msg)),
    })
}

type DispatchResult = Result<Value, (i32, String)>;

async fn tools_list(ctx: &Ctx) -> DispatchResult {
    let result = ctx
        .iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(ctx.cfg.state_timeout_ms),
        })
        .await
        .map_err(|e| (INTERNAL_ERROR, format!("engine::functions::list: {e}")))?;

    let fns: Vec<FunctionInfo> = serde_json::from_value(
        result
            .get("functions")
            .cloned()
            .ok_or_else(|| (INTERNAL_ERROR, "engine::functions::list: missing functions field".into()))?,
    )
    .map_err(|e| (INTERNAL_ERROR, format!("deserialize functions: {e}")))?;
    let tools: Vec<Value> = fns
        .iter()
        .filter(|f| !protocol::is_hidden(&f.function_id, &ctx.cfg.hidden_prefixes))
        .filter(|f| !ctx.cfg.require_expose || protocol::is_mcp_exposed(f))
        .map(protocol::function_to_tool)
        .collect();
    Ok(json!({ "tools": tools }))
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn tools_call(ctx: &Ctx, params: Option<Value>) -> DispatchResult {
    let p: ToolsCallParams = parse_params(params)?;

    let function_id = protocol::tool_name_to_function_id(&p.name);
    if protocol::is_hidden(&function_id, &ctx.cfg.hidden_prefixes) {
        return Ok(protocol::tool_error(&format!(
            "Tool '{}' is in an internal namespace and cannot be called",
            p.name
        )));
    }

    let payload = if p.arguments.is_null() {
        json!({})
    } else {
        p.arguments
    };

    match ctx
        .iii
        .trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(ctx.cfg.state_timeout_ms),
        })
        .await
    {
        Ok(v) => Ok(protocol::tool_text(&v)),
        // Tool-side failures come back as `isError: true` rather than a
        // JSON-RPC error: the call reached the engine, the engine ran
        // the function, the function said no. Clients that want a
        // protocol-level error can `notifications/cancelled` instead.
        Err(e) => {
            tracing::warn!(function_id, error = %e, "tool call failed");
            Ok(protocol::tool_error(&format!("Error: {e}")))
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResourcesReadParams {
    uri: String,
}

async fn resources_read(ctx: &Ctx, params: Option<Value>) -> DispatchResult {
    let p: ResourcesReadParams = parse_params(params)?;
    delegate(ctx, "skills::resources-read", json!({ "uri": p.uri })).await
}

#[derive(Debug, Deserialize)]
struct PromptsGetParams {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

async fn prompts_get(ctx: &Ctx, params: Option<Value>) -> DispatchResult {
    let p: PromptsGetParams = parse_params(params)?;
    let mut payload = json!({ "name": p.name });
    if let Some(args) = p.arguments {
        payload["arguments"] = args;
    }
    delegate(ctx, "prompts::mcp-get", payload).await
}

/// Trigger a sibling iii function and return its result verbatim. Used
/// for the resources/* and prompts/* delegations into the skills worker.
async fn delegate(ctx: &Ctx, function_id: &str, payload: Value) -> DispatchResult {
    match ctx
        .iii
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(ctx.cfg.state_timeout_ms),
        })
        .await
    {
        Ok(v) => Ok(v),
        Err(e) => Err((INTERNAL_ERROR, format!("{function_id}: {e}"))),
    }
}

fn parse_params<T: serde::de::DeserializeOwned>(params: Option<Value>) -> Result<T, (i32, String)> {
    let v = params.ok_or((INVALID_PARAMS, "Missing params".to_string()))?;
    serde_json::from_value(v).map_err(|e| (INVALID_PARAMS, format!("Invalid params: {e}")))
}

#[cfg(test)]
mod tests {
    //! Pure unit tests for the dispatcher pieces that don't need an iii
    //! handle. Engine-bound dispatch is covered by the BDD suite under
    //! `tests/features/`.

    use super::*;

    #[test]
    fn parse_body_accepts_json_object() {
        let v = parse_body(json!({"jsonrpc":"2.0","method":"ping","id":1})).unwrap();
        assert_eq!(v["method"], "ping");
    }

    #[test]
    fn parse_body_decodes_string_payload() {
        let raw = r#"{"jsonrpc":"2.0","method":"ping","id":1}"#;
        let v = parse_body(Value::String(raw.into())).unwrap();
        assert_eq!(v["method"], "ping");
    }

    #[test]
    fn parse_body_rejects_garbage_string() {
        let err = parse_body(Value::String("not-json".into())).unwrap_err();
        let code = err["error"]["code"].as_i64().unwrap();
        assert_eq!(code, PARSE_ERROR as i64);
    }

    #[test]
    fn parse_body_rejects_null() {
        let err = parse_body(Value::Null).unwrap_err();
        let code = err["error"]["code"].as_i64().unwrap();
        assert_eq!(code, PARSE_ERROR as i64);
    }

    #[test]
    fn parse_body_rejects_scalar() {
        let err = parse_body(json!(42)).unwrap_err();
        let code = err["error"]["code"].as_i64().unwrap();
        assert_eq!(code, PARSE_ERROR as i64);
    }

    #[test]
    fn http_envelope_wraps_body() {
        let env = http_envelope(json!({"x": 1}));
        assert_eq!(env["status_code"], 200);
        assert_eq!(env["body"]["x"], 1);
        assert_eq!(env["headers"]["content-type"], "application/json");
    }

    #[test]
    fn parse_params_missing_returns_invalid_params() {
        let err: Result<ToolsCallParams, _> = parse_params(None);
        let (code, _) = err.unwrap_err();
        assert_eq!(code, INVALID_PARAMS);
    }

    #[test]
    fn parse_params_malformed_returns_invalid_params() {
        let bad = Some(json!({ "wrong_key": 1 }));
        let err: Result<ToolsCallParams, _> = parse_params(bad);
        let (code, _) = err.unwrap_err();
        assert_eq!(code, INVALID_PARAMS);
    }

    #[test]
    fn parse_params_accepts_well_formed() {
        let p = Some(json!({ "name": "demo__echo", "arguments": { "x": 1 } }));
        let parsed: ToolsCallParams = parse_params(p).unwrap();
        assert_eq!(parsed.name, "demo__echo");
        assert_eq!(parsed.arguments["x"], 1);
    }

    #[test]
    fn parse_params_arguments_default_to_null() {
        let p = Some(json!({ "name": "demo__echo" }));
        let parsed: ToolsCallParams = parse_params(p).unwrap();
        assert_eq!(parsed.name, "demo__echo");
        assert!(parsed.arguments.is_null());
    }
}
