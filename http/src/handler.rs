//! Buffered request handler.
//!
//! Ported from the in-engine `iii-http` `dynamic_handler` (`views.rs`),
//! reduced to the buffered path: read the whole request body, invoke the
//! target function over the bus, turn its return value into an HTTP response.
//!
//! **No streaming in this phase.** `HttpRequest` carries non-`Option`
//! `request_body` / `response` [`StreamChannelRef`] fields (wire fidelity with
//! the builtin). Here we fill them with `StreamChannelRef::default()`
//! placeholders -- the buffered backend never reads them. Fase 7 replaces this
//! with a real `create_channel` pair and the streaming dispatch path.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Extension,
    body::Body,
    extract::Query,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use iii_sdk::IIIClient;
use iii_sdk::channel::StreamChannelRef;
use iii_sdk::protocol::TriggerRequest;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::condition::check_condition;
use crate::config::RestApiConfig;
use crate::middleware::{self, MiddlewareOutcome, error_body, generate_error_id};
use crate::trigger::RouteTable;
use crate::types::{HttpRequest, HttpResponse, TriggerMetadata};

/// Shared state injected into the handler via an axum `Extension`.
pub struct AppState {
    pub routes: Arc<RwLock<RouteTable>>,
    pub iii: Arc<IIIClient>,
    pub config: RestApiConfig,
}

/// Maximum request body size read into memory (buffered path). Matches a
/// conservative default; streaming bodies arrive in Fase 7.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Standardized error envelope, identical in shape to the engine's:
/// `{"error": {"code", "message"}}`.
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

/// Catch-all (fallback) handler. Matches the request against the route table
/// and, on a hit, invokes the bound function and maps its return value to an
/// HTTP response. Unmatched requests get the stable `NOT_FOUND` envelope.
pub async fn dynamic_handler(
    Extension(state): Extension<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query_params): Query<HashMap<String, String>>,
    body: Body,
) -> Response {
    let actual_path = uri.path().to_string();

    let matched = {
        let table = state.routes.read().await;
        table.match_route(method.as_str(), &actual_path)
    };
    let Some((route, path_params)) = matched else {
        return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "Not Found");
    };

    // Global middleware (config-driven, sorted by priority at config-load
    // time), run before the matched route's per-route middleware and handler.
    for mw in state
        .config
        .middleware
        .iter()
        .filter(|mw| mw.phase == "preHandler")
    {
        let mw_input =
            middleware::build_middleware_input(&path_params, &query_params, &headers, method.as_str());
        match middleware::execute_middleware(
            &state.iii,
            &mw.function_id,
            mw_input,
            state.config.default_timeout,
        )
        .await
        {
            Ok(MiddlewareOutcome::Continue) => {}
            Err(response) => return response,
        }
    }

    // Read the full body and parse as JSON; empty/invalid bodies become null.
    let body = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) if bytes.is_empty() => Value::Null,
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        Err(_) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                "Request body too large",
            );
        }
    };

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
        body,
        trigger: Some(TriggerMetadata {
            trigger_type: "http".to_string(),
            path: Some(actual_path),
            method: Some(method.as_str().to_string()),
        }),
        // Buffered phase: placeholder channel refs (see module docs). The
        // backend does not read these; Fase 7 wires real channels.
        request_body: StreamChannelRef::default(),
        response: StreamChannelRef::default(),
    };

    let payload = match serde_json::to_value(&request) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                &format!("failed to serialize request: {e}"),
            );
        }
    };

    // Conditional execution (trigger config-driven), runs after global
    // middleware and before per-route middleware/handler. The condition
    // function receives the serialized request minus the buffered-phase
    // channel placeholders, which carry no useful signal.
    if let Some(condition_id) = &route.condition_function_id {
        let mut condition_input = payload.clone();
        if let Some(obj) = condition_input.as_object_mut() {
            obj.remove("request_body");
            obj.remove("response");
        }

        match check_condition(
            &state.iii,
            condition_id,
            condition_input,
            state.config.default_timeout,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    function_id = %route.function_id,
                    condition_function_id = %condition_id,
                    "Condition check failed, skipping handler"
                );
                let mut body = error_body("CONDITION_NOT_MET", "Request condition not met", None);
                body["skipped"] = json!(true);
                return (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(body)).into_response();
            }
            Err(err) => {
                let error_id = generate_error_id();
                tracing::error!(
                    condition_function_id = %condition_id,
                    error = %err,
                    error_id = %error_id,
                    "Error invoking condition function"
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(error_body("INTERNAL_ERROR", &err.to_string(), Some(&error_id))),
                )
                    .into_response();
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
        match middleware::execute_middleware(
            &state.iii,
            mw_fn_id,
            mw_input,
            state.config.default_timeout,
        )
        .await
        {
            Ok(MiddlewareOutcome::Continue) => {}
            Err(response) => return response,
        }
    }

    let result = state
        .iii
        .trigger(TriggerRequest {
            function_id: route.function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(state.config.default_timeout),
        })
        .await;

    match result {
        Ok(value) => HttpResponse::from_function_return(value).into_axum_response(),
        Err(e) => {
            tracing::error!(function_id = %route.function_id, error = %e, "function invocation failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "Internal Server Error",
            )
        }
    }
}
