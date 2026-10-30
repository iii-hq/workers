//! The REST surface: `POST /memory/...` through the http worker.
//!
//! The worker observes `http::on-request` once per public function, with
//! args `{ method: "POST", path }`, the path being the function id with
//! `::` as `/` ([`route_path`]). Every route targets [`REST_ID`]. It
//! receives the http worker's `HttpRequest`, runs the function the route's
//! path names with the request's JSON `body` as its input (an empty body is
//! `{}`), and returns the `HttpResponse` `{ status_code, body }`: `200` with
//! the function's result, or an error status with
//! `{ error: { code, message } }`:
//!
//! | Status | `code` |
//! |---|---|
//! | 400 | `invalid_input` (also a body that does not fit the function's input) |
//! | 404 | `bank_not_found`, `memory_not_found`, `not_found` (a path no function has) |
//! | 500 | `storage` |
//! | 503 | `sibling_unavailable` |

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::MemoryError;

pub const REST_ID: &str = "memory::rest";
pub const REST_DESC: &str =
    "Internal: target of the memory REST routes, the http::on-request observers with args \
     { method: POST, path: /memory/... }. Receives the HttpRequest, runs the memory::* \
     function the route's path names with the request's JSON body, and returns the \
     HttpResponse { status_code, body }: 200 with the function's result, or 400/404/500/503 \
     with { error: { code, message } }.";

/// The `HttpRequest` fields the adapter reads. The http worker sends more
/// (`method`, `query_params`, `path_params`, `headers` and the stream
/// channels); they are ignored.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestRequest {
    /// The matched route's path: `/memory/...`, the function id with `::`
    /// as `/`.
    pub path: String,
    /// The request body parsed as JSON; `null` when it is empty or not JSON.
    #[serde(default)]
    pub body: Value,
}

/// The `HttpResponse` the http worker answers with.
#[derive(Debug, PartialEq, Serialize, JsonSchema)]
pub struct RestResponse {
    pub status_code: u16,
    pub body: Value,
}

/// The REST path of a public function: the function id with `::` as `/`.
pub fn route_path(function_id: &str) -> String {
    format!("/{}", function_id.replace("::", "/"))
}

/// Run the function `request.path` names and answer as HTTP.
pub async fn handle(deps: &Deps, request: RestRequest) -> RestResponse {
    let function_id = request
        .path
        .strip_prefix('/')
        .unwrap_or(&request.path)
        .replace('/', "::");
    let input = match request.body {
        Value::Null => json!({}),
        body => body,
    };
    match super::call_json(deps, &function_id, input).await {
        Some(Ok(body)) => RestResponse {
            status_code: 200,
            body,
        },
        Some(Err(e)) => error_response(&e),
        None => RestResponse {
            status_code: 404,
            body: error_body("not_found", &format!("no memory function at {}", request.path)),
        },
    }
}

fn error_response(error: &MemoryError) -> RestResponse {
    let (status_code, code, message) = match error {
        MemoryError::InvalidInput(m) => (400, "invalid_input", m),
        MemoryError::BankNotFound(m) => (404, "bank_not_found", m),
        MemoryError::MemoryNotFound(m) => (404, "memory_not_found", m),
        MemoryError::Storage(m) => (500, "storage", m),
        MemoryError::SiblingUnavailable(m) => (503, "sibling_unavailable", m),
    };
    RestResponse {
        status_code,
        body: error_body(code, message),
    }
}

fn error_body(code: &str, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message } })
}

/// Register [`REST_ID`], the target of every REST route.
pub fn register(iii: &IIIClient, deps: &Arc<Deps>) {
    let deps = deps.clone();
    iii.register_function(
        REST_ID,
        RegisterFunction::new_async(move |request: RestRequest| {
            let deps = deps.clone();
            async move { Ok::<_, Error>(handle(&deps, request).await) }
        })
        .description(REST_DESC)
        .metadata(json!({ "internal": true })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;

    /// Deps over a store in `dir` and no engine.
    fn offline_deps(dir: &std::path::Path) -> Deps {
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        let store = crate::store::Store::open(dir.to_path_buf()).unwrap();
        let cfg = WorkerConfig {
            embeddings_enabled: false,
            ..WorkerConfig::default()
        };
        Deps {
            emitter: Arc::new(crate::events::Emitter::new(iii.clone())),
            iii,
            store: Arc::new(tokio::sync::RwLock::new(Arc::new(store))),
            config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
        }
    }

    /// What the http worker sends a route's target: the whole `HttpRequest`.
    fn http_request(path: &str, body: Value) -> RestRequest {
        serde_json::from_value(json!({
            "query_params": {},
            "path_params": {},
            "headers": { "content-type": "application/json" },
            "path": path,
            "method": "POST",
            "body": body,
            "request_body": { "channel_id": "c1", "access_key": "k", "direction": "read" },
            "response": { "channel_id": "c2", "access_key": "k", "direction": "write" },
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn every_public_function_runs_from_its_route_and_nothing_else_does() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path());
        // A string fits no request type: each public function decodes it
        // and refuses, so none of them runs.
        for spec in super::super::catalog() {
            let response = handle(
                &deps,
                http_request(&route_path(spec.function_id), json!("not an object")),
            )
            .await;
            assert_eq!(response.status_code, 400, "{}", spec.function_id);
            assert_eq!(response.body["error"]["code"], "invalid_input");
        }
        for internal in [
            crate::hooks::PRE_GENERATE_FN,
            crate::hooks::EXTRACT_JOB_FN,
            crate::events::ON_ITEM_CHANGED_FN,
            REST_ID,
        ] {
            assert!(
                super::super::call_json(&deps, internal, json!({}))
                    .await
                    .is_none(),
                "{internal} has a route"
            );
        }
    }

    #[test]
    fn routes_are_the_function_ids_with_slashes() {
        assert_eq!(route_path("memory::save"), "/memory/save");
        assert_eq!(route_path("memory::bank::create"), "/memory/bank/create");
    }

    #[tokio::test]
    async fn a_save_route_reads_the_body_and_answers_with_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path());
        let response = handle(
            &deps,
            http_request(
                "/memory/save",
                json!({ "bank": "blog", "text": "The blog deploys on Fridays" }),
            ),
        )
        .await;
        assert_eq!(response.status_code, 200, "{:?}", response.body);
        assert_eq!(response.body["created"], true);
        assert_eq!(response.body["memory"]["text"], "The blog deploys on Fridays");

        let listed = handle(&deps, http_request("/memory/list", json!({ "bank": "blog" }))).await;
        assert_eq!(listed.status_code, 200, "{:?}", listed.body);
        assert_eq!(listed.body["memories"][0]["text"], "The blog deploys on Fridays");
    }

    #[tokio::test]
    async fn an_empty_body_is_an_empty_input() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path());
        let response = handle(&deps, http_request("/memory/bank/list", Value::Null)).await;
        assert_eq!(response.status_code, 200, "{:?}", response.body);
        assert!(response.body["banks"].is_array(), "{:?}", response.body);
    }

    #[tokio::test]
    async fn errors_answer_with_a_status_and_a_code() {
        let dir = tempfile::tempdir().unwrap();
        let deps = offline_deps(dir.path());

        let missing_text = handle(&deps, http_request("/memory/save", json!({}))).await;
        assert_eq!(missing_text.status_code, 400);
        assert_eq!(missing_text.body["error"]["code"], "invalid_input");

        let no_bank = handle(
            &deps,
            http_request("/memory/get", json!({ "bank": "absent", "id": "nope" })),
        )
        .await;
        assert_eq!(no_bank.status_code, 404, "{:?}", no_bank.body);
        assert_eq!(no_bank.body["error"]["code"], "bank_not_found");
        assert_eq!(no_bank.body["error"]["message"], "absent");

        handle(
            &deps,
            http_request("/memory/bank/create", json!({ "name": "blog" })),
        )
        .await;
        let no_memory = handle(
            &deps,
            http_request("/memory/get", json!({ "bank": "blog", "id": "nope" })),
        )
        .await;
        assert_eq!(no_memory.status_code, 404, "{:?}", no_memory.body);
        assert_eq!(no_memory.body["error"]["code"], "memory_not_found");

        let no_route = handle(&deps, http_request("/memory/hook/pre-generate", json!({}))).await;
        assert_eq!(no_route.status_code, 404);
        assert_eq!(no_route.body["error"]["code"], "not_found");
    }
}
