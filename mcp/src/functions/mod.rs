use std::sync::Arc;

use iii_observability::Logger;
use iii_sdk::{
    channels::{ChannelWriter, StreamChannelRef},
    IIIError, RegisterFunction, III,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::McpConfig;
use crate::handler::{handle_mcp_body, Outcome};
use crate::transport::encode_sse_jsonrpc;

pub const FUNCTION_ID: &str = "mcp::handler";

/// Per-handler context: shared engine handle, config, and a logger.
#[derive(Clone)]
struct Ctx {
    iii: Arc<III>,
    cfg: Arc<McpConfig>,
    log: Logger,
}

/// Subset of `iii_sdk::types::ApiRequest` plus the engine-issued response
/// channel writer ref. We deserialize directly off `serde_json::Value` so we
/// can capture the `response` field that the typed `ApiRequest<T>` strips.
#[derive(Deserialize)]
struct McpRequest {
    #[serde(default)]
    body: Value,
    response: StreamChannelRef,
}

/// Body to render onto the SSE response channel.
enum RespondPayload {
    /// A pre-encoded JSON-RPC SSE frame (`event: message\ndata: ...\n\n`).
    Single(String),
    /// Notification-only requests get a 202 ack with no body.
    Notification,
}

pub fn register_all(iii: &Arc<III>, cfg: &Arc<McpConfig>) {
    register_handler(iii, cfg);
    tracing::info!(function_id = FUNCTION_ID, "all functions registered");
}

fn register_handler(iii: &Arc<III>, cfg: &Arc<McpConfig>) {
    let ctx = Ctx {
        iii: iii.clone(),
        cfg: cfg.clone(),
        log: Logger::new(),
    };
    iii.register_function(
        FUNCTION_ID,
        RegisterFunction::new_async(move |req: Value| {
            let ctx = ctx.clone();
            async move {
                let envelope: McpRequest = match serde_json::from_value(req) {
                    Ok(e) => e,
                    Err(err) => {
                        ctx.log.warn(
                            "mcp: invalid http trigger payload",
                            Some(json!({ "error": err.to_string() })),
                        );
                        return Ok(Value::Null);
                    }
                };

                let outcome = handle_mcp_body(&envelope.body, &ctx.iii, &ctx.cfg).await;
                let payload = match outcome {
                    Outcome::Notification => RespondPayload::Notification,
                    Outcome::Response(rpc) => RespondPayload::Single(encode_sse_jsonrpc(&rpc)),
                };

                respond_sse(&ctx, &envelope.response, payload).await
            }
        })
        .description("MCP 2025-06-18 Streamable HTTP bridge."),
    );
}

async fn respond_sse(
    ctx: &Ctx,
    writer_ref: &StreamChannelRef,
    payload: RespondPayload,
) -> Result<Value, IIIError> {
    let writer = ChannelWriter::new(ctx.iii.address(), writer_ref);

    let status: u16 = match payload {
        RespondPayload::Single(_) => 200,
        // MCP Streamable HTTP §"server response": notification-only
        // requests MUST get 202 Accepted with no body.
        RespondPayload::Notification => 202,
    };

    if let Err(err) = send_sse_control(&writer, status).await {
        ctx.log.warn(
            "mcp: failed to send SSE control frames",
            Some(json!({
                "error": err.to_string(),
                "status": status,
                "channel_id": writer_ref.channel_id,
            })),
        );
        return Ok(Value::Null);
    }

    if let RespondPayload::Single(frame) = payload {
        let bytes = frame.len();
        if let Err(err) = writer.write(frame.as_bytes()).await {
            ctx.log.warn(
                "mcp: failed to write SSE body frame",
                Some(json!({
                    "error": err.to_string(),
                    "bytes": bytes,
                    "channel_id": writer_ref.channel_id,
                })),
            );
        } else {
            ctx.log.debug(
                "mcp SSE body sent",
                Some(json!({
                    "bytes": bytes,
                    "status": status,
                    "channel_id": writer_ref.channel_id,
                })),
            );
        }
    } else {
        ctx.log.debug(
            "mcp SSE notification ack",
            Some(json!({
                "status": status,
                "channel_id": writer_ref.channel_id,
            })),
        );
    }

    if let Err(err) = writer.close().await {
        ctx.log.warn(
            "mcp: failed to close SSE response channel",
            Some(json!({
                "error": err.to_string(),
                "channel_id": writer_ref.channel_id,
            })),
        );
    }

    Ok(Value::Null)
}

/// Send the iii HTTP trigger control frames that configure the response
/// status line and headers. Always advertises `text/event-stream`; even
/// notification-only acks (which carry no body) keep the same content
/// type so a browser EventSource can attach without surprises.
async fn send_sse_control(writer: &ChannelWriter, status: u16) -> Result<(), IIIError> {
    writer
        .send_message(
            &serde_json::to_string(&json!({
                "type": "set_status",
                "status_code": status,
            }))
            .expect("set_status frame serializes"),
        )
        .await?;

    writer
        .send_message(
            &serde_json::to_string(&json!({
                "type": "set_headers",
                "headers": {
                    "content-type": "text/event-stream",
                    "cache-control": "no-cache",
                    "connection": "keep-alive",
                },
            }))
            .expect("set_headers frame serializes"),
        )
        .await?;

    Ok(())
}
