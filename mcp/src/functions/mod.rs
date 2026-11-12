//! The worker's functions and routes.
//!
//! - `mcp::handler` serves the MCP Streamable HTTP endpoint: it observes
//!   `http::on-request` for `POST <api_path>` and `DELETE <api_path>`, runs
//!   [`handle_http`], and streams the reply as one SSE event.
//! - `mcp::observed` is the target of every observer an MCP session creates
//!   with `iii_observe`; it hands each delivery to [`Sessions::deliver`].

use std::sync::Arc;

use iii_helpers::observability::Logger;
use iii_sdk::{
    channels::{ChannelWriter, StreamChannelRef},
    errors::Error,
    IIIClient, ObserverHandle, RegisterFunction, Source,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::McpConfig;
use crate::engine::SdkEngine;
use crate::handler::{handle_http, App, HttpReply};
use crate::rbac_guard::RbacMode;
use crate::sessions::Sessions;
use crate::transport::encode_sse_jsonrpc;

pub const FUNCTION_ID: &str = "mcp::handler";
pub const SINK_FUNCTION_ID: &str = "mcp::observed";
pub const HTTP_ON_REQUEST_FN: &str = "http::on-request";

#[derive(Clone)]
pub struct Ctx {
    pub engine: SdkEngine,
    pub cfg: Arc<McpConfig>,
    pub sessions: Arc<Sessions>,
    pub mode: Arc<RbacMode>,
    log: Logger,
}

impl Ctx {
    pub fn new(
        engine: SdkEngine,
        cfg: Arc<McpConfig>,
        sessions: Arc<Sessions>,
        mode: Arc<RbacMode>,
    ) -> Self {
        Self {
            engine,
            cfg,
            sessions,
            mode,
            log: Logger::new(),
        }
    }
}

/// The parts of an `http::on-request` `HttpRequest` the bridge reads.
#[derive(Deserialize, schemars::JsonSchema)]
struct McpRequest {
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: Value,
    #[serde(default)]
    body: Value,
    response: StreamChannelRef,
}

fn default_method() -> String {
    "POST".to_string()
}

fn schema_value<T: schemars::JsonSchema>() -> Value {
    let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>();
    let mut v = serde_json::to_value(root).expect("schema serializes");
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
    }
    v
}

pub fn register_all(iii: &Arc<IIIClient>, ctx: &Ctx) {
    register_handler(iii, ctx);
    register_sink(iii, ctx);
    tracing::info!(
        function_id = FUNCTION_ID,
        sink = SINK_FUNCTION_ID,
        "functions registered"
    );
}

pub fn route_args(method: &str, api_path: &str) -> Value {
    json!({ "method": method, "path": api_path })
}

/// The `POST` and `DELETE` routes of the endpoint.
pub fn observe_routes(iii: &IIIClient, api_path: &str) -> Vec<ObserverHandle> {
    ["POST", "DELETE"]
        .iter()
        .map(|m| {
            iii.observe(
                Source::function(HTTP_ON_REQUEST_FN).args(route_args(m, api_path)),
                FUNCTION_ID,
            )
        })
        .collect()
}

fn register_handler(iii: &Arc<IIIClient>, ctx: &Ctx) {
    let ctx = ctx.clone();
    iii.register_function(
        FUNCTION_ID,
        RegisterFunction::new_async(move |req: Value| {
            let ctx = ctx.clone();
            async move {
                let envelope: McpRequest = match serde_json::from_value(req) {
                    Ok(e) => e,
                    Err(err) => {
                        ctx.log.warn(
                            "mcp: invalid http::on-request payload",
                            Some(json!({ "error": err.to_string() })),
                        );
                        return Ok(Value::Null);
                    }
                };
                let app = App {
                    bus: &ctx.engine,
                    cfg: &ctx.cfg,
                    sessions: &ctx.sessions,
                    mode: &ctx.mode,
                    sink: SINK_FUNCTION_ID,
                };
                let reply =
                    handle_http(&app, &envelope.method, &envelope.headers, &envelope.body).await;
                respond(&ctx, &envelope.response, reply).await
            }
        })
        .request_format(schema_value::<McpRequest>())
        .response_format(json!({ "type": "null" }))
        .description("MCP 2025-06-18 Streamable HTTP endpoint (gateway tools, sessions, observe)."),
    );
}

fn register_sink(iii: &Arc<IIIClient>, ctx: &Ctx) {
    let sessions = ctx.sessions.clone();
    // Not `private`: the engine refuses a private observer target. Anything
    // delivered here that is not one of a live session's observers is dropped.
    iii.register_function(
        SINK_FUNCTION_ID,
        RegisterFunction::new_async(move |event: Value| {
            let sessions = sessions.clone();
            async move {
                sessions.deliver(event);
                Ok(json!({}))
            }
        })
        .metadata(json!({ "internal": true, "trace_hidden": true }))
        .description("Receives invocations observed by MCP sessions (iii_observe); internal."),
    );
}

async fn respond(
    ctx: &Ctx,
    writer_ref: &StreamChannelRef,
    reply: HttpReply,
) -> Result<Value, Error> {
    let writer = ChannelWriter::new(ctx.engine.client.address(), writer_ref);
    let mut headers = json!({
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        "connection": "keep-alive",
    });
    if let Some(sid) = &reply.session_id {
        headers["mcp-session-id"] = json!(sid);
    }
    let control = [
        json!({ "type": "set_status", "status_code": reply.status }),
        json!({ "type": "set_headers", "headers": headers }),
    ];
    for frame in control {
        if let Err(err) = writer.send_message(&frame.to_string()).await {
            ctx.log.warn(
                "mcp: failed to send SSE control frames",
                Some(json!({ "error": err.to_string(), "status": reply.status, "channel_id": writer_ref.channel_id })),
            );
            return Ok(Value::Null);
        }
    }
    if let Some(rpc) = &reply.rpc {
        let frame = encode_sse_jsonrpc(rpc);
        if let Err(err) = writer.write(frame.as_bytes()).await {
            ctx.log.warn(
                "mcp: failed to write SSE body frame",
                Some(json!({ "error": err.to_string(), "channel_id": writer_ref.channel_id })),
            );
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_cover_post_and_delete_on_the_api_path() {
        assert_eq!(
            route_args("POST", "/mcp"),
            json!({ "method": "POST", "path": "/mcp" })
        );
        assert_eq!(
            route_args("DELETE", "/x"),
            json!({ "method": "DELETE", "path": "/x" })
        );
    }
}
