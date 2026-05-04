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
    self, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND,
    PARSE_ERROR,
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

/// Outcome of validating a JSON-RPC request frame.
///
/// Kept as an enum (rather than `Result`) so the notification fast-path is
/// distinguishable from the well-formed-request path without overloading
/// `Ok`/`Err` semantics.
enum FrameValidation {
    /// Well-formed request that should be dispatched.
    Request {
        method: String,
        id: Value,
        params: Option<Value>,
    },
    /// Well-formed JSON-RPC notification (no `id`). Per spec, no response
    /// is ever sent. We swallow these in v0.1 since there's no
    /// per-session state to update.
    Notification { method: String },
    /// Malformed frame. The dispatcher returns this body verbatim with
    /// the JSON-RPC `id` echoed back when present.
    InvalidRequest(Value),
}

/// JSON-RPC 2.0 §4: a request MUST be a JSON object with a string
/// `method`. Pure helper so we can unit-test it without an engine handle.
/// Returns the parsed (method, id, params) for well-formed frames, the
/// notification path for spec-compliant notifications, or a fully formed
/// JSON-RPC error frame for malformed input. Reject `{}`, `[]`, and
/// scalar payloads instead of falling through to the notification
/// fast-path (which would swallow them as a silent 204).
fn validate_frame(body: &Value) -> FrameValidation {
    let id = body.get("id").cloned();
    let obj = match body.as_object() {
        Some(o) => o,
        None => {
            return FrameValidation::InvalidRequest(json!(JsonRpcResponse::error(
                id,
                INVALID_REQUEST,
                "Invalid Request: body must be a JSON object",
            )));
        }
    };
    let method = match obj.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => {
            return FrameValidation::InvalidRequest(json!(JsonRpcResponse::error(
                id,
                INVALID_REQUEST,
                "Invalid Request: missing or non-string `method`",
            )));
        }
    };
    let params = obj.get("params").cloned();

    if method.starts_with("notifications/") || id.is_none() {
        return FrameValidation::Notification { method };
    }

    FrameValidation::Request {
        method,
        id: id.unwrap_or(Value::Null),
        params,
    }
}

/// Dispatch a single JSON-RPC frame. Returns `None` for notifications
/// (no response per JSON-RPC 2.0).
pub async fn dispatch(ctx: &Ctx, body: Value) -> Option<Value> {
    let (method, id, params) = match validate_frame(&body) {
        FrameValidation::Request { method, id, params } => (method, Some(id), params),
        FrameValidation::Notification { method } => {
            if method.starts_with("notifications/") {
                tracing::debug!(method = %method, "notification received (no response)");
            }
            return None;
        }
        FrameValidation::InvalidRequest(err) => return Some(err),
    };
    let method = method.as_str();

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

    let fns: Vec<FunctionInfo> =
        serde_json::from_value(result.get("functions").cloned().ok_or_else(|| {
            (
                INTERNAL_ERROR,
                "engine::functions::list: missing functions field".into(),
            )
        })?)
        .map_err(|e| (INTERNAL_ERROR, format!("deserialize functions: {e}")))?;
    let tools: Vec<Value> = fns
        .iter()
        .filter(|f| !protocol::is_hidden(&f.function_id, &ctx.cfg.hidden_prefixes))
        .filter(|f| !ctx.cfg.require_expose || protocol::is_mcp_exposed(f))
        .filter(|f| {
            // Hide function ids that would round-trip through the
            // `::` ↔ `__` mapping ambiguously — clients must never see
            // a tool name they can't reliably call back.
            if protocol::is_tool_name_ambiguous(&f.function_id) {
                tracing::warn!(
                    function_id = %f.function_id,
                    "skipping function: id contains '__' and would collide with the MCP tool-name encoding"
                );
                false
            } else {
                true
            }
        })
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

    // Verify the ::↔__ mapping round-trips. Catches ambiguous tool
    // names (e.g. `worker_v2__util__action` would decode to
    // `worker_v2::util::action` AND a different `worker_v2__util::action`).
    // Without this, a client could silently call the wrong function.
    let re_encoded = protocol::function_id_to_tool_name(&function_id);
    if re_encoded != p.name {
        return Ok(protocol::tool_error(&format!(
            "Tool '{}' is ambiguous (function id '{}' would re-encode to '{}'); refusing to dispatch",
            p.name, function_id, re_encoded
        )));
    }

    if protocol::is_hidden(&function_id, &ctx.cfg.hidden_prefixes) {
        return Ok(protocol::tool_error(&format!(
            "Tool '{}' is in an internal namespace and cannot be called",
            p.name
        )));
    }

    // require_expose is an *execution* guard, not just a listing filter.
    // When enabled, clients can only invoke functions explicitly tagged
    // `metadata.mcp.expose == true`. We resolve the function info from
    // the engine here so a guessed/synthesised tool name can't bypass
    // the expose flag just because it isn't under a hidden prefix.
    if ctx.cfg.require_expose {
        match lookup_function_info(ctx, &function_id).await? {
            Some(info) if protocol::is_mcp_exposed(&info) => {}
            _ => {
                return Ok(protocol::tool_error(&format!(
                    "Tool '{}' is not exposed for MCP (require_expose=true)",
                    p.name
                )));
            }
        }
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

/// Look up a single function's metadata via `engine::functions::list`.
/// Returns `Ok(None)` if no function with that id is registered.
///
/// Called from `tools/call` only when `require_expose` is enabled, so
/// the extra round trip is opt-in for security-conscious deployments.
async fn lookup_function_info(
    ctx: &Ctx,
    function_id: &str,
) -> Result<Option<FunctionInfo>, (i32, String)> {
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

    let fns: Vec<FunctionInfo> =
        serde_json::from_value(result.get("functions").cloned().ok_or_else(|| {
            (
                INTERNAL_ERROR,
                "engine::functions::list: missing functions field".into(),
            )
        })?)
        .map_err(|e| (INTERNAL_ERROR, format!("deserialize functions: {e}")))?;

    Ok(fns.into_iter().find(|f| f.function_id == function_id))
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

    fn invalid_request_code(v: &Value) -> i64 {
        v["error"]["code"].as_i64().unwrap_or(0)
    }

    #[test]
    fn validate_frame_accepts_well_formed_request() {
        let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        match validate_frame(&body) {
            FrameValidation::Request { method, id, .. } => {
                assert_eq!(method, "ping");
                assert_eq!(id, json!(1));
            }
            other => panic!("expected Request, got {:?}", as_dbg(other)),
        }
    }

    #[test]
    fn validate_frame_classifies_notification_when_id_missing() {
        let body = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        match validate_frame(&body) {
            FrameValidation::Notification { method } => {
                assert_eq!(method, "notifications/initialized");
            }
            other => panic!("expected Notification, got {:?}", as_dbg(other)),
        }
    }

    #[test]
    fn validate_frame_rejects_empty_object() {
        // Prior bug: `{}` was treated as a notification and returned
        // HTTP 204 instead of -32600 INVALID_REQUEST.
        let body = json!({});
        match validate_frame(&body) {
            FrameValidation::InvalidRequest(err) => {
                assert_eq!(invalid_request_code(&err), INVALID_REQUEST as i64);
            }
            other => panic!("expected InvalidRequest, got {:?}", as_dbg(other)),
        }
    }

    #[test]
    fn validate_frame_rejects_array_body() {
        let body = json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]);
        match validate_frame(&body) {
            FrameValidation::InvalidRequest(err) => {
                assert_eq!(invalid_request_code(&err), INVALID_REQUEST as i64);
            }
            other => panic!("expected InvalidRequest, got {:?}", as_dbg(other)),
        }
    }

    #[test]
    fn validate_frame_rejects_id_only_with_no_method() {
        // Prior bug: `{"id":1}` would fall through to METHOD_NOT_FOUND
        // with an empty method string. Now -32600 INVALID_REQUEST.
        let body = json!({"id":1});
        match validate_frame(&body) {
            FrameValidation::InvalidRequest(err) => {
                assert_eq!(invalid_request_code(&err), INVALID_REQUEST as i64);
                assert_eq!(err["id"], json!(1), "id must be echoed back");
            }
            other => panic!("expected InvalidRequest, got {:?}", as_dbg(other)),
        }
    }

    #[test]
    fn validate_frame_rejects_non_string_method() {
        let body = json!({"jsonrpc":"2.0","id":1,"method":42});
        match validate_frame(&body) {
            FrameValidation::InvalidRequest(err) => {
                assert_eq!(invalid_request_code(&err), INVALID_REQUEST as i64);
            }
            other => panic!("expected InvalidRequest, got {:?}", as_dbg(other)),
        }
    }

    /// Just for nicer panic output in the validate_frame tests.
    fn as_dbg(v: FrameValidation) -> String {
        match v {
            FrameValidation::Request { method, id, .. } => {
                format!("Request {{ method: {method}, id: {id} }}")
            }
            FrameValidation::Notification { method } => {
                format!("Notification {{ method: {method} }}")
            }
            FrameValidation::InvalidRequest(err) => format!("InvalidRequest({err})"),
        }
    }
}
