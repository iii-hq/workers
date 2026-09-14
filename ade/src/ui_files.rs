//! `GET /ui-files/:worker/*path` — documents and assets a worker serves for
//! its own iframes (the stories worker's built story pages and their
//! chunks). The console pulls the bytes through `<worker>::ui-file { path }`
//! → `{ content_base64 | content, content_type, cache? }` and answers with
//! them. The route exists so an injected page can embed a worker-served
//! document without the worker opening a port: every request goes over the
//! engine, and the document may only be framed by the console itself.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use iii_sdk::protocol::TriggerRequest;
use serde_json::{json, Value};

use crate::server::AppState;

const FETCH_TIMEOUT_MS: u64 = 15_000;
/// Story bundles include the project's React and CSS; 32 MiB leaves room
/// for an unminified chunk graph while bounding console memory.
const MAX_BYTES: usize = 32 * 1024 * 1024;

pub fn valid_worker(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.contains('\\')
                && !segment.chars().any(char::is_control)
        })
}

/// `content_base64` wins; `content` is taken as UTF-8 text.
pub fn decode_content(response: &Value) -> Option<Vec<u8>> {
    if let Some(encoded) = response.get("content_base64").and_then(Value::as_str) {
        return base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .ok();
    }
    response
        .get("content")
        .and_then(Value::as_str)
        .map(|text| text.as_bytes().to_vec())
}

pub async fn ui_file_handler(
    State(state): State<AppState>,
    Path((worker, path)): Path<(String, String)>,
) -> Response {
    let Some(iii) = &state.iii else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    if !valid_worker(&worker) || !valid_path(&path) {
        return (StatusCode::BAD_REQUEST, "bad worker or path").into_response();
    }
    let function_id = format!("{worker}::ui-file");
    let response = match iii
        .trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload: json!({ "path": path }),
            action: None,
            timeout_ms: Some(FETCH_TIMEOUT_MS),
        })
        .await
    {
        Ok(value) => value,
        Err(error) => {
            tracing::debug!(function_id, error = %error, "ui-file fetch failed");
            return (StatusCode::NOT_FOUND, "no such file").into_response();
        }
    };
    let Some(bytes) = decode_content(&response) else {
        return (StatusCode::BAD_GATEWAY, "worker returned no content").into_response();
    };
    if bytes.len() > MAX_BYTES {
        return (StatusCode::PAYLOAD_TOO_LARGE, "file too large").into_response();
    }
    let content_type = response
        .get("content_type")
        .and_then(Value::as_str)
        .and_then(|value| HeaderValue::from_str(value).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    let cache = if response.get("cache").and_then(Value::as_str) == Some("immutable") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        // The console's defaults deny framing everywhere; these documents
        // exist to be framed by the console's own injected pages.
        .header(header::X_FRAME_OPTIONS, "SAMEORIGIN")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "frame-ancestors 'self'; object-src 'none'",
        )
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_and_path_validation() {
        assert!(valid_worker("stories"));
        assert!(valid_worker("my-worker_2"));
        assert!(!valid_worker("Stories"));
        assert!(!valid_worker("a/b"));
        assert!(!valid_worker(""));
        assert!(valid_path("ws/worktree/app/node_modules/.stories/x.html"));
        assert!(valid_path("ws/abc/assets/chunk-B1.js"));
        assert!(!valid_path("ws/../x"));
        assert!(!valid_path("ws//x"));
        assert!(!valid_path(""));
        assert!(!valid_path("a\\b"));
    }

    #[test]
    fn content_decoding_prefers_base64() {
        assert_eq!(
            decode_content(&json!({ "content_base64": "aGk=", "content": "no" })).unwrap(),
            b"hi"
        );
        assert_eq!(
            decode_content(&json!({ "content": "text" })).unwrap(),
            b"text"
        );
        assert!(decode_content(&json!({ "content_base64": "%%%" })).is_none());
        assert!(decode_content(&json!({})).is_none());
    }

    #[tokio::test]
    async fn route_is_absent_without_an_engine_client() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let state = AppState::new(std::sync::Arc::new("ws://x".to_string()), None, None, None);
        let app = crate::server::router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ui-files/stories/ws/worktree/x.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
