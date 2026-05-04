//! Function + trigger registration. Single function (`mcp::handler`)
//! plus the HTTP trigger that routes `POST /<api_path>` into it.

pub mod handler;

use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunctionMessage, RegisterTriggerInput, III};
use serde_json::{json, Value};

use crate::config::McpConfig;

/// Register `mcp::handler` and bind the HTTP trigger. Called once from
/// `main.rs` after the engine handle is established. Errors registering
/// the trigger are logged and swallowed — the function itself stays
/// registered, so direct `iii.trigger("mcp::handler", body)` calls (used
/// by the BDD harness) keep working even if another worker has already
/// claimed the same `api_path`.
pub fn register_all(iii: &Arc<III>, cfg: &Arc<McpConfig>) {
    register_handler(iii, cfg);
    register_http_trigger(iii, cfg);
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

fn register_http_trigger(iii: &Arc<III>, cfg: &Arc<McpConfig>) {
    let api_path = cfg.api_path.clone();
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
            tracing::info!(api_path = %cfg.api_path, "MCP HTTP trigger registered: POST /{}", cfg.api_path)
        }
        Err(e) => {
            tracing::warn!(error = %e, api_path = %cfg.api_path, "failed to register MCP HTTP trigger")
        }
    }
}
