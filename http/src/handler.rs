//! Request handler with streaming request/response over SDK channels.
//!
//! Ported from the in-engine `iii-http` `dynamic_handler` (`views.rs`). For
//! every request the worker creates two SDK channels via
//! [`iii_sdk::helpers::create_channel`]:
//!
//! - **request-body channel**: the worker WRITES the request body; the target
//!   function READS it. The worker keeps `req.writer` and passes
//!   `req.reader_ref` to the function as [`HttpRequest::request_body`].
//! - **response channel**: the target function WRITES the response; the worker
//!   READS it. The worker keeps `res.reader` and passes `res.writer_ref` to the
//!   function as [`HttpRequest::response`].
//!
//! This mirrors the engine's `take_sender`/`take_receiver` semantics exactly
//! (engine `views.rs:391-466`).
//!
//! The worker then races the response channel against the function invocation:
//! a function that writes to the response channel streams its body (control
//! Text messages set status/headers, Binary frames carry the body); a function
//! that returns a non-null value without touching the channel produces a
//! buffered response (backward-compatible path).

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use axum::{
    body::Body,
    extract::Query,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    Extension,
};
use colored::Colorize;
use iii_helpers::observability::opentelemetry::metrics::Counter;
use iii_helpers::observability::opentelemetry::KeyValue;
use iii_sdk::helpers::create_channel;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::Instrument;

use crate::condition::check_condition;
use crate::configuration::ConfigCell;
use crate::middleware::{self, error_body, generate_error_id, MiddlewareOutcome};
use crate::trigger::RouteTable;
use crate::types::{ControlMessage, HttpRequest, HttpResponse, TriggerMetadata};

/// Shared state injected into the handler via an axum `Extension`.
pub struct AppState {
    pub routes: Arc<RwLock<RouteTable>>,
    pub iii: Arc<IIIClient>,
    /// Live config cell, read once per request so `middleware`/`default_timeout`
    /// changes hot-reload (see [`crate::configuration`]).
    pub config: ConfigCell,
}

/// Maximum request body size read into memory before it is streamed into the
/// request-body channel.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Channel buffer size for both request-body and response channels (matches the
/// engine default in `views.rs`).
const CHANNEL_BUFFER: usize = 64;

/// Map an HTTP response status code to an OTel span status string.
///
/// Only responses with status >= 400 are treated as errors; 1xx/2xx/3xx are
/// not (a 3xx redirect is a normal outcome, not a failure). Mirrors the
/// engine's `otel_status_for` (`views.rs`).
fn otel_status_for(status_code: u16) -> &'static str {
    if status_code < 400 {
        "OK"
    } else {
        "ERROR"
    }
}

/// Per-request OTEL request counter (`iii.http.requests`), created once from the
/// global meter. Incremented one-per-request in the `record_status` closure.
///
/// Uses `opentelemetry::global::meter`, so the instrument binds to whatever
/// meter provider is installed when this `OnceLock` first initializes (the SDK's
/// `init_otel` sets one on connect; otherwise it is a cheap no-op). The counter
/// is the standalone-worker analogue of the engine's internal `track_api_request`
/// atomic (`engine/.../telemetry/collector.rs`), but exportable via OTLP.
fn request_counter() -> &'static Counter<u64> {
    static COUNTER: OnceLock<Counter<u64>> = OnceLock::new();
    COUNTER.get_or_init(|| {
        iii_helpers::observability::opentelemetry::global::meter("iii-http")
            .u64_counter("iii.http.requests")
            .with_description("Count of HTTP requests handled by the iii http worker")
            .build()
    })
}

/// Standardized error envelope, identical in shape to the engine's:
/// `{"error": {"code", "message"}}`.
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

/// Apply a parsed control message to the accumulated response status/headers.
///
/// Field-tolerant by construction: a `SetHeaders` with non-string values keeps
/// the valid entries and skips the rest. Malformed messages never reach this
/// function -- [`ControlMessage::parse`] rejects them and the caller logs+skips,
/// carrying the current status/headers forward (engine `apply_control_message`
/// tolerance).
fn apply_control(
    msg: ControlMessage,
    status_code: &mut u16,
    headers: &mut HashMap<String, String>,
) {
    match msg {
        ControlMessage::SetStatus { status_code: sc } => *status_code = sc,
        ControlMessage::SetHeaders { headers: h } => {
            if let Some(obj) = h.as_object() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        headers.insert(k.clone(), s.to_string());
                    }
                }
            }
        }
    }
}

/// Catch-all (fallback) handler. Matches the request against the route table
/// and, on a hit, invokes the bound function and maps its return value (or its
/// streamed response channel output) to an HTTP response. Unmatched requests
/// get the stable `NOT_FOUND` envelope.
pub async fn dynamic_handler(
    Extension(state): Extension<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    body: Body,
) -> Response {
    let actual_path = uri.path().to_string();

    // Snapshot the live config once for the whole request, so `middleware` and
    // `default_timeout` reflect the latest hot-reloaded value (see
    // `crate::configuration`) while staying consistent within this request.
    let config = state.config.read().await.clone();

    let matched = {
        let table = state.routes.read().await;
        table.match_route(method.as_str(), &actual_path)
    };

    // Per-request OTEL/tracing HTTP span, mirroring the engine's iii-http
    // (`views.rs`) minus the request metric (a later phase). Trace-context
    // parent propagation is applied just below (after the span is built).
    // `route_path` is the matched route pattern (e.g.
    // `/users/:id`) when a route matches, else the actual request path (404).
    // The final status is recorded via `record_status` at every return point.
    let route_path = matched
        .as_ref()
        .map(|(route, _)| route.http_path.clone())
        .unwrap_or_else(|| actual_path.clone());

    // W3C trace-context parent propagation: read the inbound headers before the
    // span is built. `tracestate` is read for completeness but NOT propagated --
    // the SDK's `extract_context` takes only `traceparent` + `baggage` (unlike
    // the engine's with-state variant). See `set_parent` below.
    let header_str = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let traceparent = header_str("traceparent");
    let _tracestate = header_str("tracestate");
    let baggage = header_str("baggage");

    let span = tracing::info_span!(
        "HTTP",
        otel.name = %format!("{} {}", method, route_path),
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
        "http.request.method" = %method,
        "http.route" = %route_path,
        "url.path" = %actual_path,
        "http.response.status_code" = tracing::field::Empty,
    );

    // Make this span a child of the caller's trace when the request carries
    // `traceparent`/`baggage`. `set_parent` operates on the built span
    // regardless of its `is_contextual` flag, so it is applied here after
    // `info_span!` (mirrors the engine's `with_parent_headers`). On success we
    // emit a `traceparent.propagated` event carrying the parent trace id, so the
    // link is visible in the OTel export (field names identical to the engine).
    if traceparent.is_some() || baggage.is_some() {
        use iii_helpers::observability::opentelemetry::trace::TraceContextExt;
        use iii_helpers::observability::opentelemetry::KeyValue;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let parent_cx =
            iii_helpers::observability::extract_context(traceparent.as_deref(), baggage.as_deref());
        let parent_trace_id = parent_cx.span().span_context().trace_id();
        match span.set_parent(parent_cx) {
            Ok(()) => {
                span.add_event(
                    "traceparent.propagated",
                    vec![
                        KeyValue::new("parent.trace_id", format!("{parent_trace_id}")),
                        KeyValue::new("traceparent", traceparent.clone().unwrap_or_default()),
                    ],
                );
            }
            Err(err) => {
                use tracing_opentelemetry::SetParentError;
                match err {
                    // The "HTTP" info_span is filtered out by the active log
                    // level (e.g. RUST_LOG=warn). Ordinary operator config, not a
                    // misconfiguration -- keep it at debug to avoid per-request
                    // WARN spam when info spans are disabled.
                    SetParentError::SpanDisabled => tracing::debug!(
                        parent_trace_id = %parent_trace_id,
                        "HTTP span disabled by log filter; skipping trace-context parent"
                    ),
                    // LayerNotFound (OTel bridge missing) / AlreadyStarted (span
                    // entered before set_parent) are real wiring bugs -- surface them.
                    other => tracing::warn!(
                        error = %other,
                        parent_trace_id = %parent_trace_id,
                        "failed to set parent trace context on HTTP span"
                    ),
                }
            }
        }
    }

    // Record the final response status on the current per-request HTTP span
    // (`otel.status_code` + `http.response.status_code`) and increment the OTEL
    // request counter once, then return the response unchanged so it can wrap
    // any `return <response>` expression.
    //
    // Reading the status from the already-built response records+counts it in
    // exactly one place per return path -- the engine records the same pair
    // wherever the status becomes known (`views.rs`). The counter attributes
    // mirror the span tags: method + matched route (captured here) + status.
    let method_attr = method.as_str().to_string();
    let route_attr = route_path.clone();
    let called_function_id = matched.as_ref().map(|(route, _)| route.function_id.clone());
    let started = std::time::Instant::now();
    let record_status = move |response: Response| -> Response {
        let code = response.status().as_u16();
        let span = tracing::Span::current();
        span.record("otel.status_code", otel_status_for(code));
        span.record("http.response.status_code", code);
        request_counter().add(
            1,
            &[
                KeyValue::new("http.request.method", method_attr.clone()),
                KeyValue::new("http.route", route_attr.clone()),
                KeyValue::new("http.response.status_code", i64::from(code)),
            ],
        );

        let status = code.to_string();
        let status = match code {
            200..=299 => status.green(),
            300..=399 => status.cyan(),
            400..=499 => status.yellow(),
            _ => status.red(),
        };
        tracing::info!(
            "{} Endpoint {} → {} {} {}",
            "[CALLED]".blue(),
            format!("{} {}", method_attr, route_attr)
                .bright_yellow()
                .bold(),
            called_function_id
                .as_deref()
                .unwrap_or("<no route>")
                .purple(),
            status,
            format!("({}ms)", started.elapsed().as_millis()).dimmed()
        );
        response
    };

    async move {
    let Some((route, path_params)) = matched else {
        // Path matches a registered route but the method doesn't -> 405 (with
        // an `Allow` header listing the methods that would have matched),
        // mirroring axum's per-method routing in the engine's iii-http. A
        // path that matches no route at all stays a 404.
        let allowed = state.routes.read().await.allowed_methods(&actual_path);
        if allowed.is_empty() {
            return record_status(error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "Not Found"));
        }
        let mut response = error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
            "Method Not Allowed",
        );
        if let Ok(value) = axum::http::HeaderValue::from_str(&allowed.join(", ")) {
            response.headers_mut().insert(axum::http::header::ALLOW, value);
        }
        return record_status(response);
    };

    // Global middleware (config-driven, sorted by priority at config-load
    // time), run before the matched route's per-route middleware and handler.
    for mw in config.middleware.iter().filter(|mw| mw.phase == "preHandler") {
        let mw_input =
            middleware::build_middleware_input(&path_params, &query_params, &headers, method.as_str());
        match middleware::execute_middleware(
            &state.iii,
            &mw.function_id,
            mw_input,
            config.default_timeout,
        )
        .await
        {
            Ok(MiddlewareOutcome::Continue) => {}
            Err(response) => return record_status(response),
        }
    }

    // Read the full body. Empty/invalid bodies parse to null for the
    // backward-compatible `body` field; the raw bytes are streamed into the
    // request-body channel regardless.
    let body_bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return record_status(error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                "Request body too large",
            ));
        }
    };
    let parsed_body: Value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(Value::Null)
    };

    // Create the request-body and response channels. The worker keeps the
    // request-body writer and the response reader; the matching refs are handed
    // to the function (see module docs for the direction mapping).
    let req_channel = match create_channel(&state.iii, Some(CHANNEL_BUFFER)).await {
        Ok(c) => c,
        Err(e) => {
            return record_status(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                &format!("failed to create request channel: {e}"),
            ));
        }
    };
    let res_channel = match create_channel(&state.iii, Some(CHANNEL_BUFFER)).await {
        Ok(c) => c,
        Err(e) => {
            return record_status(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                &format!("failed to create response channel: {e}"),
            ));
        }
    };

    let req_writer = req_channel.writer;
    let req_reader_ref = req_channel.reader_ref;
    let res_reader = res_channel.reader;
    let res_writer_ref = res_channel.writer_ref;

    // Stream the request body into the request-body channel, then close it so
    // the function's reader observes EOF. Detached: the function reads it
    // concurrently and the response path must not block on it.
    {
        let bytes = body_bytes.clone();
        tokio::spawn(async move {
            if !bytes.is_empty() {
                let _ = req_writer.write(&bytes).await;
            }
            let _ = req_writer.close().await;
        });
    }

    let header_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_string(), v.to_string())))
        .collect();

    let request = HttpRequest {
        query_params,
        path_params,
        headers: header_map,
        path: route.http_path.clone(),
        method: method.as_str().to_string(),
        body: parsed_body,
        trigger: Some(TriggerMetadata {
            trigger_type: "http".to_string(),
            path: Some(actual_path),
            method: Some(method.as_str().to_string()),
        }),
        request_body: req_reader_ref,
        response: res_writer_ref,
    };

    let payload = match serde_json::to_value(&request) {
        Ok(v) => v,
        Err(e) => {
            return record_status(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                &format!("failed to serialize request: {e}"),
            ));
        }
    };

    // Conditional execution (trigger config-driven), runs after global
    // middleware and before per-route middleware/handler. The condition
    // function receives the serialized request minus the channel refs, which
    // carry no useful signal.
    if let Some(condition_id) = &route.condition_function_id {
        let mut condition_input = payload.clone();
        if let Some(obj) = condition_input.as_object_mut() {
            obj.remove("request_body");
            obj.remove("response");
        }

        match check_condition(&state.iii, condition_id, condition_input, config.default_timeout)
        .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    function_id = %route.function_id,
                    condition_function_id = %condition_id,
                    "Condition check failed, skipping handler"
                );
                drop(res_reader);
                let mut body = error_body("CONDITION_NOT_MET", "Request condition not met", None);
                body["skipped"] = json!(true);
                return record_status(
                    (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(body)).into_response(),
                );
            }
            Err(err) => {
                let error_id = generate_error_id();
                tracing::error!(
                    condition_function_id = %condition_id,
                    error = %err,
                    error_id = %error_id,
                    "Error invoking condition function"
                );
                drop(res_reader);
                return record_status(
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        axum::Json(error_body("INTERNAL_ERROR", &err.to_string(), Some(&error_id))),
                    )
                        .into_response(),
                );
            }
        }
    }

    // Per-route middleware (trigger config-driven), runs after the condition
    // check, before invoking the handler.
    for mw_fn_id in &route.middleware_function_ids {
        let mw_input = middleware::build_middleware_input(
            &request.path_params,
            &request.query_params,
            &headers,
            method.as_str(),
        );
        match middleware::execute_middleware(&state.iii, mw_fn_id, mw_input, config.default_timeout)
        .await
        {
            Ok(MiddlewareOutcome::Continue) => {}
            Err(response) => {
                drop(res_reader);
                return record_status(response);
            }
        }
    }

    // Accumulated response control state. Text control messages on the response
    // channel are dispatched (in order, ahead of the next Binary frame) to the
    // `on_message` callback, which updates this shared state. A malformed
    // control message is logged and skipped -- it never kills the stream
    // (carry-forward: the current status/headers stay in effect).
    let control = Arc::new(StdMutex::new((200u16, HashMap::<String, String>::new())));
    {
        let control = control.clone();
        res_reader
            .on_message(move |text| match ControlMessage::parse(&text) {
                Some(msg) => {
                    if let Ok(mut guard) = control.lock() {
                        let (status, headers) = &mut *guard;
                        apply_control(msg, status, headers);
                    }
                }
                None => {
                    tracing::warn!(msg = %text, "Response channel: skipping malformed control message");
                }
            })
            .await;
    }

    // Forward Binary frames off the response reader into an mpsc channel. This
    // keeps the reader off the `select!` (its `next_binary` future is not
    // cancellation-safe), so racing the channel against the invocation cannot
    // corrupt the reader. Text frames are consumed internally by `next_binary`
    // and applied via the callback above. The task ends (dropping the reader,
    // closing its socket) when the function closes the response channel.
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(CHANNEL_BUFFER);
    let forward = tokio::spawn(async move {
        while let Ok(Some(data)) = res_reader.next_binary().await {
            if tx.send(data).await.is_err() {
                break;
            }
        }
    });

    // Spawn the invocation so it runs concurrently with draining the response
    // channel.
    let mut call = {
        let iii = state.iii.clone();
        let function_id = route.function_id.clone();
        let timeout_ms = config.default_timeout;
        tokio::spawn(async move {
            iii.trigger(TriggerRequest {
                function_id,
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
        })
    };

    // Overall bound for the streaming drain. The engine bounds an open response
    // channel via `channel_mgr.remove_channel` (drops the origin sender, forcing
    // EOF). The SDK exposes no worker-side `remove_channel`, so the worker cannot
    // replicate that and would otherwise hang the client forever on a
    // null-returning function that never closes its writer. Instead we bound each
    // `rx.recv()` wait with the request timeout as an *idle* timeout (simplest
    // safe choice: total-timeout would penalize legitimate slow long streams).
    let stream_timeout = Duration::from_millis(config.default_timeout);

    // Race the response channel against the invocation result.
    //
    // - A Binary frame -> the function is streaming; begin building the body.
    // - The invocation completing with a non-null value -> buffered response
    //   (the function never touched the channel).
    // - The invocation completing with `null` -> the function streamed (or
    //   produced no body); drain the remaining channel frames into the body.
    // Biased: prefer a ready channel frame over the invocation result, so a
    // streaming function's first Binary frame wins the race even if its return
    // value arrives in the same poll.
    enum Race {
        /// The channel produced first: `Some(chunk)` body data, or `None` if it
        /// closed with no body. Control preceding this point is already applied.
        Chunk(Option<Vec<u8>>),
        /// The invocation returned `null` first: the body (and any control) may
        /// still be in flight on the channel socket -- drain before snapshotting.
        Streamed,
    }

    let race = tokio::select! {
        biased;
        item = rx.recv() => Race::Chunk(item),
        result = &mut call => {
            match result {
                Ok(Ok(value)) => {
                    if value.is_null() {
                        Race::Streamed
                    } else {
                        // Buffered response: the function returned a value
                        // without streaming. Abort the (idle) reader task.
                        forward.abort();
                        return record_status(
                            HttpResponse::from_function_return(value).into_axum_response(),
                        );
                    }
                }
                Ok(Err(err)) => {
                    forward.abort();
                    let error_id = generate_error_id();
                    tracing::error!(
                        function_id = %route.function_id,
                        error = %err,
                        error_id = %error_id,
                        "function invocation failed"
                    );
                    return record_status(
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(middleware::error_body_for_call_error(
                                &err,
                                Some(&error_id),
                            )),
                        )
                            .into_response(),
                    );
                }
                Err(join_err) => {
                    forward.abort();
                    let error_id = generate_error_id();
                    tracing::error!(
                        function_id = %route.function_id,
                        error = %join_err,
                        error_id = %error_id,
                        "function invocation task panicked"
                    );
                    return record_status(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "INTERNAL_ERROR",
                        "Internal Server Error",
                    ));
                }
            }
        }
    };

    // Bug-1 fix: when the invocation result wins the race, control messages
    // (`set_status`/`set_headers`) may still be travelling the separate channel
    // socket. Do NOT snapshot status/headers yet -- wait for the first body
    // chunk first. The forward task folds each control Text into `control`
    // synchronously (inside `next_binary`) before yielding the Binary that
    // follows it, so once a chunk is observed all preceding control is applied.
    // Bug-2 fix: bound this wait with `stream_timeout` (see above).
    let first_chunk: Option<Vec<u8>> = match race {
        Race::Chunk(item) => item,
        Race::Streamed => match tokio::time::timeout(stream_timeout, rx.recv()).await {
            Ok(item) => item,
            Err(_) => {
                tracing::warn!(
                    timeout_ms = config.default_timeout,
                    function_id = %route.function_id,
                    "Streaming drain timed out waiting for first frame; ending stream"
                );
                forward.abort();
                None
            }
        },
    };

    // Streaming response. The invocation handle is left to complete on its own
    // (the function's return value is irrelevant once it streams); the forward
    // task keeps feeding `rx` until the function closes the channel. Snapshot
    // status/headers now -- all control preceding `first_chunk` is applied.
    let (status_code, response_headers) = {
        let guard = control.lock().expect("control mutex poisoned");
        (guard.0, guard.1.clone())
    };

    // Abort handle so the unfold can stop the forward task on idle timeout (the
    // forward task may be parked in `next_binary` and would not observe `rx`
    // being dropped). Dropping the `forward` JoinHandle itself does not abort it.
    let forward_abort = forward.abort_handle();
    let stream = futures::stream::unfold(
        (first_chunk, rx, forward_abort),
        move |(pending, mut rx, abort)| async move {
            if let Some(chunk) = pending {
                return Some((Ok::<Vec<u8>, Infallible>(chunk), (None, rx, abort)));
            }
            // Bug-2 fix: idle-bound each subsequent frame wait.
            match tokio::time::timeout(stream_timeout, rx.recv()).await {
                Ok(Some(data)) => Some((Ok(data), (None, rx, abort))),
                Ok(None) => None,
                Err(_) => {
                    tracing::warn!("Streaming drain idle timeout; ending stream");
                    abort.abort();
                    None
                }
            }
        },
    );

    let mut builder = axum::http::Response::builder()
        .status(StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK));
    for (k, v) in &response_headers {
        match (
            axum::http::header::HeaderName::try_from(k.as_str()),
            axum::http::header::HeaderValue::try_from(v.as_str()),
        ) {
            (Ok(name), Ok(value)) => {
                builder = builder.header(name, value);
            }
            _ => {
                tracing::warn!(header_name = %k, "Skipping invalid response header");
            }
        }
    }

    record_status(match builder.body(Body::from_stream(stream)) {
        Ok(resp) => resp.into_response(),
        Err(err) => {
            let error_id = generate_error_id();
            tracing::error!(
                error = ?err,
                error_id = %error_id,
                "Failed to build streaming response"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "Internal Server Error",
            )
        }
    })
    }
    .instrument(span)
    .await
}
