//! Function + trigger registration. Single function (`mcp::handler`)
//! plus the HTTP trigger that routes `POST /<api_path>` into it.

pub mod handler;

use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunctionMessage, RegisterTriggerInput, III};
use serde_json::{json, Value};

use crate::config::McpConfig;

/// Register `mcp::handler` and bind the HTTP trigger. Called once from
/// `main.rs` after the engine handle is established.
///
/// `mcp::handler` registration is infallible (the function stays
/// registered regardless), so direct `iii.trigger("mcp::handler", body)`
/// callers (e.g. the BDD harness) always work. The HTTP trigger
/// registration *is* fallible — if it fails, the bridge is unreachable
/// over HTTP and the caller should *not* advertise readiness. Returns
/// `Err` in that case so the caller can decide what to log.
pub fn register_all(iii: &Arc<III>, cfg: &Arc<McpConfig>) -> Result<(), IIIError> {
    register_handler(iii, cfg);
    register_http_trigger(iii, cfg)
}

fn register_handler(iii: &Arc<III>, cfg: &Arc<McpConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "mcp::handler".to_string(),
            description: Some(
                "MCP JSON-RPC dispatcher. Wraps iii functions as MCP tools and the skills worker as MCP resources/prompts.".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "description": "iii HTTP-trigger envelope (`{ body, headers, ... }`) or a raw JSON-RPC frame for direct invocation."
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "status_code": { "type": "integer" },
                    "headers": { "type": "object" },
                    "body": {}
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let ctx = handler::Ctx {
                iii: iii_inner.clone(),
                cfg: cfg_inner.clone(),
            };
            Box::pin(async move { handler::handle(&ctx, payload).await })
        },
    );

    tracing::info!("registered mcp::handler");
}

fn register_http_trigger(iii: &Arc<III>, cfg: &Arc<McpConfig>) -> Result<(), IIIError> {
    // The iii engine prepends its own `/` to api_path during route
    // matching, so an operator-supplied "/mcp" would register as
    // "//mcp" and 404. Strip leading slashes defensively here so the
    // same McpConfig instance is safe to register from anywhere.
    let api_path = cfg.api_path.trim_start_matches('/').to_string();
    if api_path != cfg.api_path {
        tracing::warn!(
            original = %cfg.api_path,
            normalized = %api_path,
            "stripped leading '/' from api_path; engine prepends its own"
        );
    }

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "mcp::handler".to_string(),
        config: json!({
            "api_path": api_path,
            "http_method": "POST",
        }),
        metadata: None,
    }) {
        Ok(_) => {
            tracing::info!(api_path = %api_path, "MCP HTTP trigger registered: POST /{}", api_path);
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = %e, api_path = %api_path, "failed to register MCP HTTP trigger");
            Err(e)
        }
    }
}
