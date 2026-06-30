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

use crate::config::RestApiConfig;
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
