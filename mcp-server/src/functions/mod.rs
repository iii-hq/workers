//! The worker's functions and routes.
//!
//! - `mcp-server::handler` serves the MCP Streamable HTTP endpoint: it observes
//!   `http::on-request` for `POST`, `GET` and `DELETE <api_path>`, runs
//!   [`handle_http`], and writes the reply as one SSE event to the request's
//!   `response` stream; a `GET` keeps it open as the session's resumable
//!   stream of SSE observations.
//! - `mcp-server::observed` is the target of every poll and SSE observer an MCP
//!   session creates; it admits deliveries through [`sink_guard::admit`] and
//!   hands them to [`Sessions::deliver`].

use std::sync::Arc;

use iii_helpers::observability::Logger;
use iii_sdk::{IIIClient, ObserverHandle, OutputStream, RegisterFunction, Source, errors::Error};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bus::Bus;
use crate::config::McpConfig;
use crate::engine::SdkEngine;
use crate::handler::{App, HttpReply, handle_http};
use crate::http_guard::{self, RouteAction};
use crate::rbac_guard::RbacMode;
use crate::sessions::Sessions;
use crate::sessions::StreamItem;
use crate::sink_guard;
use crate::transport::{STREAM_OPEN, encode_sse_gap, encode_sse_jsonrpc, encode_sse_observation};

/// The methods the endpoint serves on `api_path`.
pub const ROUTE_METHODS: [&str; 3] = ["POST", "GET", "DELETE"];

pub const FUNCTION_ID: &str = "mcp-server::handler";
pub const SINK_FUNCTION_ID: &str = "mcp-server::observed";
pub const HTTP_ON_REQUEST_FN: &str = "http::on-request";
/// Called after the http worker applies a configuration change.
pub const HTTP_RELOAD_FN: &str = "http::on-config-change";
/// Re-checks the http worker's bind after each of its reloads.
pub const RELOAD_FUNCTION_ID: &str = "mcp-server::on-http-reload";

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
    let root = schemars::r#gen::SchemaGenerator::default().into_root_schema_for::<T>();
    let mut v = serde_json::to_value(root).expect("schema serializes");
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
    }
    v
}

/// An observation delivered to [`SINK_FUNCTION_ID`] by an observer an MCP
/// session created; [`sink_guard::admit`] checks it before delivery.
fn sink_request_schema() -> Value {
    json!({
        "type": "object",
        "description": "An observation from one of an MCP session's poll or SSE observers.",
    })
}

/// The `http::on-config-change` observation; only its arrival matters.
fn reload_request_schema() -> Value {
    json!({
        "type": "object",
        "description": "The http worker's configuration-change observation; its content is not read.",
    })
}

/// Both internal functions answer an empty object.
fn empty_response_schema() -> Value {
    json!({ "type": "object", "additionalProperties": false })
}

/// `(function_id, request_schema, response_schema)` of every function the
/// worker registers.
pub fn function_schemas() -> Vec<(&'static str, Value, Value)> {
    vec![
        (
            FUNCTION_ID,
            schema_value::<McpRequest>(),
            json!({ "type": "null" }),
        ),
        (
            SINK_FUNCTION_ID,
            sink_request_schema(),
            empty_response_schema(),
        ),
        (
            RELOAD_FUNCTION_ID,
            reload_request_schema(),
            empty_response_schema(),
        ),
    ]
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

/// The endpoint's `http::on-request` routes: served at boot, withdrawn when
/// the http worker rebinds somewhere the endpoint must not be reachable,
/// restored when it is back on loopback.
pub struct Routes {
    iii: Arc<IIIClient>,
    api_path: String,
    handles: tokio::sync::Mutex<Option<Vec<ObserverHandle>>>,
    /// Held across a whole [`recheck_bind`]: one reads `http::status` only
    /// after the previous one acted, so a stale read cannot undo a newer one.
    rechecking: tokio::sync::Mutex<()>,
    /// Bumped by every check a reload starts: a recovery run started by an
    /// earlier one stops (see [`recheck_and_recover`]).
    recovery: std::sync::atomic::AtomicU64,
}

impl Routes {
    pub fn new(iii: Arc<IIIClient>, api_path: String) -> Arc<Self> {
        Arc::new(Self {
            iii,
            api_path,
            handles: tokio::sync::Mutex::new(None),
            rechecking: tokio::sync::Mutex::new(()),
            recovery: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub async fn serving(&self) -> bool {
        self.handles.lock().await.is_some()
    }

    /// Observe the routes, unless already served. Reports whether they
    /// became active without blocking.
    pub async fn serve(&self) {
        let mut handles = self.handles.lock().await;
        if handles.is_some() {
            return;
        }
        let routes = observe_routes(&self.iii, &self.api_path);
        let (watch, api_path) = (routes.clone(), self.api_path.clone());
        tokio::spawn(async move {
            for route in watch {
                match tokio::time::timeout(std::time::Duration::from_secs(30), route.ready()).await
                {
                    Ok(Ok(())) => {
                        tracing::info!(api_path = %api_path, observer_id = %route.id(), "http::on-request route active")
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, api_path = %api_path, "http::on-request route rejected")
                    }
                    Err(_) => {
                        tracing::warn!(api_path = %api_path, "http::on-request route still pending after 30s; is the http worker running?")
                    }
                }
            }
        });
        *handles = Some(routes);
    }

    /// Stop serving: remove the routes from the http worker.
    pub async fn withdraw(&self) {
        if let Some(routes) = self.handles.lock().await.take() {
            for route in routes {
                let _ = route.dispose().await;
            }
        }
    }
}

/// Re-check the bind after every reload of the http worker (it can rebind
/// at runtime). No polling: the http worker's reload handler is observed,
/// and each time it ran, `http::status` is read again and the routes are
/// withdrawn or restored. Calling the target directly only re-checks.
pub fn watch_http_reloads(iii: &Arc<IIIClient>, ctx: &Ctx, routes: Arc<Routes>) -> ObserverHandle {
    let (ready_ctx, ready_routes) = (ctx.clone(), routes.clone());
    let ctx = ctx.clone();
    iii.register_function(
        RELOAD_FUNCTION_ID,
        RegisterFunction::new_async(move |_: Value| {
            let (ctx, routes) = (ctx.clone(), routes.clone());
            async move {
                recheck_and_recover(&ctx, &routes).await;
                Ok::<Value, Error>(json!({}))
            }
        })
        .request_format(reload_request_schema())
        .response_format(empty_response_schema())
        .metadata(json!({ "internal": true, "trace_hidden": true }))
        .description("Internal: re-checks the http worker's bind after it reloads, and withdraws or restores the MCP routes."),
    );
    let handle = iii.observe(Source::function(HTTP_RELOAD_FN), RELOAD_FUNCTION_ID);
    // The routes are first served here, once this observer is active, by
    // checking the bind again: a rebind since the boot check is caught, and
    // none goes unseen while the routes are up. While the observer is not
    // active (slow, or refused), they are not served: a rebind to a public
    // address would go unseen. The public-http override serves anyway.
    let (watch, ctx, routes) = (handle.clone(), ready_ctx, ready_routes);
    tokio::spawn(async move {
        // Warn if activation is slow, but keep waiting: the SDK keeps the
        // observer, and it can still become active (or be refused) later.
        let ready = watch.ready();
        tokio::pin!(ready);
        let result = match tokio::time::timeout(std::time::Duration::from_secs(30), &mut ready)
            .await
        {
            Ok(result) => result,
            Err(_) => {
                if unwatched_override() {
                    tracing::warn!(
                        "observing {HTTP_RELOAD_FN} is still pending after 30s; serving the MCP routes anyway ({}=true), and until it is active, a rebind of the http worker is not seen",
                        http_guard::PUBLIC_HTTP_ENV
                    );
                    recheck_and_recover(&ctx, &routes).await;
                } else {
                    tracing::warn!(
                        "observing {HTTP_RELOAD_FN} is still pending after 30s; the MCP routes are served once it is active, so that a rebind of the http worker is seen"
                    );
                }
                ready.await
            }
        };
        match result {
            Ok(()) => {
                tracing::info!(
                    "watching {HTTP_RELOAD_FN}: the MCP routes follow the http worker's bind"
                );
                recheck_and_recover(&ctx, &routes).await;
            }
            Err(e) if unwatched_override() => {
                tracing::warn!(
                    error = %e,
                    "observing {HTTP_RELOAD_FN} was refused; serving the MCP routes anyway ({}=true), so a later rebind of the http worker to a public address will not withdraw them",
                    http_guard::PUBLIC_HTTP_ENV
                );
                recheck_and_recover(&ctx, &routes).await;
            }
            Err(e) => {
                let message = format!(
                    "mcp-server: not serving {}: observing {HTTP_RELOAD_FN} was refused ({e}), so a later rebind of the http worker to a public address would go unseen.\n  Fix: expose {HTTP_RELOAD_FN} in rbac-proxy (rbac.expose_functions), then restart mcp-server.\n  To serve without it:\n    {}=true",
                    ctx.cfg.api_path,
                    http_guard::PUBLIC_HTTP_ENV
                );
                tracing::error!("{message}");
                eprintln!("{message}");
            }
        }
    });
    handle
}

/// The public-http override, read the way the bind check reads it.
fn unwatched_override() -> bool {
    http_guard::serve_unwatched(http_guard::parse_flag(
        std::env::var(http_guard::PUBLIC_HTTP_ENV).ok().as_deref(),
    ))
}

/// Re-check the bind; when `http::status` cannot be read, the routes stay
/// withdrawn and recovery checks follow after each of
/// [`http_guard::RECOVERY_DELAYS`], until one reads the bind or a newer
/// check starts its own.
pub async fn recheck_and_recover(ctx: &Ctx, routes: &Arc<Routes>) {
    use std::sync::atomic::Ordering;
    let generation = routes.recovery.fetch_add(1, Ordering::SeqCst) + 1;
    if recheck_bind(ctx, routes).await {
        return;
    }
    tracing::warn!(
        "could not read http::status; the MCP routes stay withdrawn, re-checking after 2, 5, 10, 30 and 60 s, then on the next http reload"
    );
    let (ctx, routes) = (ctx.clone(), routes.clone());
    tokio::spawn(async move {
        let still = routes.clone();
        http_guard::recover(
            &http_guard::RECOVERY_DELAYS,
            move || still.recovery.load(Ordering::SeqCst) == generation,
            || {
                let (ctx, routes) = (ctx.clone(), routes.clone());
                async move { recheck_bind(&ctx, &routes).await }
            },
        )
        .await;
    });
}

/// Check the bind once and withdraw or restore the routes; `false` when
/// `http::status` could not be read (the routes are then withdrawn).
pub async fn recheck_bind(ctx: &Ctx, routes: &Routes) -> bool {
    let _one_at_a_time = routes.rechecking.lock().await;
    let status = Bus::call(&ctx.engine, "http::status", json!({}), Some(10_000))
        .await
        .map_err(|e| e.to_string());
    let flag = http_guard::parse_flag(std::env::var(http_guard::PUBLIC_HTTP_ENV).ok().as_deref());
    let known = !http_guard::status_unknown(&status);
    let bind = http_guard::decide(status, flag, &ctx.cfg.api_path);
    match http_guard::transition(routes.serving().await, &bind) {
        RouteAction::Keep => {}
        RouteAction::Withdraw(why) => {
            routes.withdraw().await;
            tracing::error!(
                "{why}\n  The MCP routes are withdrawn until the http worker is back on loopback."
            );
        }
        RouteAction::Restore => {
            routes.serve().await;
            tracing::info!(api_path = %ctx.cfg.api_path, "the http worker's bind is acceptable; MCP routes served");
        }
    }
    if let http_guard::Bind::Serve { warnings } = bind {
        for w in warnings {
            tracing::warn!("{w}");
        }
    }
    known
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
                            "mcp-server: invalid http::on-request payload",
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
                    tracing::warn!("mcp-server: dropped a delivery to {SINK_FUNCTION_ID} from a worker that does not host the observed function");
                }
                Ok(json!({}))
            }
        })
        .request_format(sink_request_schema())
        .response_format(empty_response_schema())
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
                "mcp-server: failed to send the response status and headers",
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
                "mcp-server: failed to write the response body",
                Some(json!({ "error": err.to_string() })),
            );
        }
    }
    // End the reply cleanly so the client reads its end, not an abort. (The
    // SDK would close it on return too; closing twice does nothing.)
    let _ = response.close().await;
    Ok(Value::Null)
}

/// Keep a `GET` response open as the session's SSE stream: first the
/// backlog (what the client has not seen, after its `Last-Event-ID`), then
/// each observation as it arrives, after one [`STREAM_OPEN`] frame that
/// sends the headers. Nothing is written on a schedule. A write
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
    let Ok((backlog, mut live, generation)) =
        ctx.sessions.attach_stream(&session_id, last_event_id)
    else {
        return Ok(Value::Null);
    };
    response.keep_open();
    response.bind(&ctx.engine.client);
    let sessions = ctx.sessions.clone();
    tokio::spawn(async move {
        // Once, not on a schedule: the status and headers go out with it.
        if response.write(STREAM_OPEN.as_bytes()).await.is_err() {
            return;
        }
        for item in backlog {
            if !write_item(&response, &sessions, &session_id, generation, item).await {
                return;
            }
        }
        while let Some(item) = live.recv().await {
            if !write_item(&response, &sessions, &session_id, generation, item).await {
                return;
            }
        }
        let _ = response.close().await;
    });
    Ok(Value::Null)
}

/// Write one stream item; `false` once the client is gone or another `GET`
/// took the session's stream over (it sends the rest). The item is claimed
/// before it is written ([`Sessions::claim_stream_item`]), so a stream that
/// takes over meanwhile never sends it a second time.
async fn write_item(
    response: &OutputStream,
    sessions: &Sessions,
    session_id: &str,
    generation: u64,
    item: StreamItem,
) -> bool {
    let (frame, id) = match item {
        StreamItem::Observation { id, observation } => {
            (encode_sse_observation(id, &observation), Some(id))
        }
        StreamItem::Gap { after, through } => (encode_sse_gap(after, through), None),
    };
    if !sessions.claim_stream_item(session_id, generation, id) {
        return false;
    }
    let written = response.write(frame.as_bytes()).await.is_ok();
    if let Some(id) = id {
        // A failed write is unsent again (see `finish_stream_item`).
        sessions.finish_stream_item(session_id, generation, id, written);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys the interface collector (`collect_worker_interface.py`)
    /// accepts as a typed schema; anything else is "unknown" and fails the
    /// boot smoke and publication.
    fn is_typed(schema: &Value) -> bool {
        const KEYS: [&str; 9] = [
            "type",
            "properties",
            "$ref",
            "allOf",
            "anyOf",
            "oneOf",
            "enum",
            "items",
            "const",
        ];
        schema
            .as_object()
            .is_some_and(|o| KEYS.iter().any(|k| o.contains_key(*k)))
    }

    #[test]
    fn every_registered_function_has_typed_schemas() {
        let ids: Vec<&str> = function_schemas().iter().map(|(id, _, _)| *id).collect();
        assert_eq!(ids, [FUNCTION_ID, SINK_FUNCTION_ID, RELOAD_FUNCTION_ID]);
        for (id, request, response) in function_schemas() {
            assert!(
                is_typed(&request),
                "{id} request schema is untyped: {request}"
            );
            assert!(
                is_typed(&response),
                "{id} response schema is untyped: {response}"
            );
        }
    }

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
