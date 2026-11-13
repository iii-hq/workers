//! The worker's functions and routes.
//!
//! - `mcp::handler` serves the MCP Streamable HTTP endpoint: it observes
//!   `http::on-request` for `POST`, `GET` and `DELETE <api_path>`, runs
//!   [`handle_http`], and writes the reply as one SSE event to the request's
//!   `response` stream; a `GET` keeps it open as the session's resumable
//!   stream of SSE observations.
//! - `mcp::observed` is the target of every poll and SSE observer an MCP
//!   session creates; it admits deliveries through [`sink_guard::admit`] and
//!   hands them to [`Sessions::deliver`].

use std::sync::Arc;

use iii_helpers::observability::Logger;
use iii_sdk::{errors::Error, IIIClient, ObserverHandle, OutputStream, RegisterFunction, Source};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::McpConfig;
use crate::engine::SdkEngine;
use crate::handler::{handle_http, App, HttpReply};
use crate::rbac_guard::RbacMode;
use crate::sessions::Sessions;
use crate::sessions::StreamItem;
use crate::sink_guard;
use crate::transport::{encode_sse_gap, encode_sse_jsonrpc, encode_sse_observation};

/// The methods the endpoint serves on `api_path`.
pub const ROUTE_METHODS: [&str; 3] = ["POST", "GET", "DELETE"];

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
    /// The HTTP response: status and headers as messages, the body as bytes.
    response: OutputStream,
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

/// The [`ROUTE_METHODS`] routes of the endpoint.
pub fn observe_routes(iii: &IIIClient, api_path: &str) -> Vec<ObserverHandle> {
    ROUTE_METHODS
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
                respond(&ctx, envelope.response, reply).await
            }
        })
        .request_format(schema_value::<McpRequest>())
        .response_format(json!({ "type": "null" }))
        .description("MCP 2025-06-18 Streamable HTTP endpoint (gateway tools, sessions, observe).")
        // A request body can carry a credential for iii_secret_store; an
        // observer would get a copy of every one.
        .observable(false),
    );
}

fn register_sink(iii: &Arc<IIIClient>, ctx: &Ctx) {
    let ctx = ctx.clone();
    // Not `private`: the engine refuses a private observer target. Deliveries
    // pass the caller check first; anything that is not one of a live
    // session's observers is dropped.
    iii.register_function(
        SINK_FUNCTION_ID,
        RegisterFunction::new_async(move |mut observation: Value| {
            let ctx = ctx.clone();
            async move {
                if sink_guard::admit(&ctx.engine, &mut observation).await {
                    ctx.sessions.deliver(observation);
                } else {
                    tracing::warn!("mcp: dropped a delivery to {SINK_FUNCTION_ID} from a worker that does not host the observed function");
                }
                Ok(json!({}))
            }
        })
        .metadata(json!({ "internal": true, "trace_hidden": true }))
        .description("Receives the observations of MCP sessions' poll and SSE observers; internal."),
    );
}

async fn respond(ctx: &Ctx, response: OutputStream, reply: HttpReply) -> Result<Value, Error> {
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
        if let Err(err) = response.send(&frame).await {
            ctx.log.warn(
                "mcp: failed to send the response status and headers",
                Some(json!({ "error": err.to_string(), "status": reply.status })),
            );
            return Ok(Value::Null);
        }
    }
    if let Some((session_id, last_event_id)) = reply.stream.clone() {
        return open_stream(ctx, response, session_id, last_event_id);
    }
    if let Some(rpc) = &reply.rpc {
        let frame = encode_sse_jsonrpc(rpc);
        if let Err(err) = response.write(frame.as_bytes()).await {
            ctx.log.warn(
                "mcp: failed to write the response body",
                Some(json!({ "error": err.to_string() })),
            );
        }
    }
    // Returning null ends the response.
    Ok(Value::Null)
}

/// Keep a `GET` response open as the session's SSE stream: first the
/// backlog (what the client has not seen, after its `Last-Event-ID`), then
/// each observation as it arrives. Nothing is written on a schedule. A write
/// that fails means the client left: the task ends, and since observations
/// stay in the session until the buffer cap drops them, the client's
/// reconnect with `Last-Event-ID` gets everything it missed. The stream also
/// ends when the session ends or another `GET` replaces it.
fn open_stream(
    ctx: &Ctx,
    response: OutputStream,
    session_id: String,
    last_event_id: Option<u64>,
) -> Result<Value, Error> {
    let Ok((backlog, mut live)) = ctx.sessions.attach_stream(&session_id, last_event_id) else {
        return Ok(Value::Null);
    };
    response.keep_open();
    response.bind(&ctx.engine.client);
    let sessions = ctx.sessions.clone();
    tokio::spawn(async move {
        for item in backlog {
            if !write_item(&response, &sessions, &session_id, item).await {
                return;
            }
        }
        while let Some(item) = live.recv().await {
            if !write_item(&response, &sessions, &session_id, item).await {
                return;
            }
        }
        let _ = response.close().await;
    });
    Ok(Value::Null)
}

/// Write one stream item; `false` once the client is gone.
async fn write_item(
    response: &OutputStream,
    sessions: &Sessions,
    session_id: &str,
    item: StreamItem,
) -> bool {
    let (frame, id) = match item {
        StreamItem::Observation { id, observation } => {
            (encode_sse_observation(id, &observation), Some(id))
        }
        StreamItem::Gap { after, through } => (encode_sse_gap(after, through), None),
    };
    if response.write(frame.as_bytes()).await.is_err() {
        return false;
    }
    if let Some(id) = id {
        sessions.mark_sent(session_id, id);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_serves_post_get_and_delete() {
        assert_eq!(ROUTE_METHODS, ["POST", "GET", "DELETE"]);
    }

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
