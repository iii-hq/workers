//! JSON-RPC dispatch for the MCP bridge.
//!
//! Direct port of `src/mcp/handler.ts`.

use std::sync::Arc;

use iii_sdk::III;
use serde_json::{json, Value};

use crate::config::McpConfig;
use crate::jsonrpc::{
    is_json_rpc_request, json_rpc_error, json_rpc_success, parse_json_rpc_request,
    response_id_from_body, JsonRpcRequest, JsonRpcResponse, JsonRpcResponseId, INVALID_PARAMS,
    INVALID_REQUEST, METHOD_NOT_FOUND,
};
use crate::skills_bridge::{
    mcp_prompts_get, mcp_prompts_list, mcp_resources_list, mcp_resources_read,
    mcp_resources_templates_list,
};
use crate::tools::{mcp_tools_call, mcp_tools_list, McpToolsCallError};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug)]
pub enum Outcome {
    Notification,
    Response(JsonRpcResponse),
}

fn handle_initialize(_params: &Value) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {},
        },
        "serverInfo": {
            "name": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

async fn dispatch_notification(_req: &JsonRpcRequest, _iii: &Arc<III>, _cfg: &Arc<McpConfig>) {
    /* notifications/initialized and unknown notifications: no response body */
}

async fn dispatch_request(
    req: &JsonRpcRequest,
    iii: &Arc<III>,
    cfg: &Arc<McpConfig>,
    id: JsonRpcResponseId,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => json_rpc_success(id, handle_initialize(&req.params)),
        "ping" => json_rpc_success(id, json!({})),
        "tools/list" => match mcp_tools_list(iii, cfg).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, crate::jsonrpc::INTERNAL_ERROR, e.to_string()),
        },
        "tools/call" => match mcp_tools_call(iii, cfg, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(McpToolsCallError::InvalidParams(e)) => {
                json_rpc_error(id, INVALID_PARAMS, e.to_string())
            }
        },
        "resources/list" => match mcp_resources_list(iii, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, crate::jsonrpc::INTERNAL_ERROR, e.to_string()),
        },
        "resources/read" => match mcp_resources_read(iii, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, crate::jsonrpc::INTERNAL_ERROR, e.to_string()),
        },
        "resources/templates/list" => match mcp_resources_templates_list(iii, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, crate::jsonrpc::INTERNAL_ERROR, e.to_string()),
        },
        "prompts/list" => match mcp_prompts_list(iii, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, crate::jsonrpc::INTERNAL_ERROR, e.to_string()),
        },
        "prompts/get" => match mcp_prompts_get(iii, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, crate::jsonrpc::INTERNAL_ERROR, e.to_string()),
        },
        other => json_rpc_error(id, METHOD_NOT_FOUND, format!("Method not found: {other}")),
    }
}

pub async fn handle_mcp_body(body: &Value, iii: &Arc<III>, cfg: &Arc<McpConfig>) -> Outcome {
    if body.is_array() {
        return Outcome::Response(json_rpc_error(
            JsonRpcResponseId::Null,
            INVALID_REQUEST,
            "Batch requests are not supported",
        ));
    }
    if !is_json_rpc_request(body) {
        return Outcome::Response(json_rpc_error(
            response_id_from_body(body),
            INVALID_REQUEST,
            "Invalid JSON-RPC request",
        ));
    }
    let Some(req) = parse_json_rpc_request(body) else {
        return Outcome::Response(json_rpc_error(
            response_id_from_body(body),
            INVALID_REQUEST,
            "Invalid JSON-RPC request",
        ));
    };
    match req.id.clone() {
        None => {
            dispatch_notification(&req, iii, cfg).await;
            Outcome::Notification
        }
        Some(id) => {
            let response_id = JsonRpcResponseId::Id(id);
            let rpc = dispatch_request(&req, iii, cfg, response_id).await;
            Outcome::Response(rpc)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn handle_initialize_returns_negotiated_protocol() {
        let v = handle_initialize(&json!({}));
        assert_eq!(v["protocolVersion"], PROTOCOL_VERSION);
        assert!(v["capabilities"]["tools"].is_object());
        assert!(v["capabilities"]["resources"].is_object());
        assert!(v["capabilities"]["prompts"].is_object());
        assert_eq!(v["serverInfo"]["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(v["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn handle_initialize_ignores_client_protocol_version() {
        let v = handle_initialize(&json!({"protocolVersion": "1999-01-01"}));
        assert_eq!(v["protocolVersion"], PROTOCOL_VERSION);
    }
}
