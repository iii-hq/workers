//! The REST surface: `POST /memory-consolidate/run` and
//! `POST /memory-consolidate/status` through the http worker.
//!
//! The worker observes `http::on-request` once per public function, with
//! args `{ method: "POST", path }`, the path being the function id with
//! `::` as `/` ([`route_path`]). Both routes target [`REST_ID`]. It receives
//! the http worker's `HttpRequest`, runs the function the route's path names
//! with the request's JSON `body` as its input (an empty body is `{}`), and
//! returns the `HttpResponse` `{ status_code, body }`: `200` with the
//! function's result, `400` when the body does not fit the function's input,
//! `404` for a path no function has, and `500` when the function fails, each
//! error as `{ error: message }`.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::functions::{self, Deps, RUN_ID, STATUS_ID};

pub const REST_ID: &str = "memory-consolidate::rest";
pub const REST_DESC: &str =
    "Internal: target of the memory-consolidate REST routes, the http::on-request observers \
     with args { method: POST, path: /memory-consolidate/run | /memory-consolidate/status }. \
     Receives the HttpRequest, runs the function the route's path names with the request's \
     JSON body, and returns the HttpResponse { status_code, body }: 200 with the function's \
     result, or 400/404/500 with { error }.";

/// The `HttpRequest` fields the adapter reads. The http worker sends more
/// (`method`, `query_params`, `path_params`, `headers` and the stream
/// channels); they are ignored.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestRequest {
    /// The matched route's path: `/memory-consolidate/run` or
    /// `/memory-consolidate/status`.
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
    let input = match request.body {
        Value::Null => json!({}),
        body => body,
    };
    let result = if request.path == route_path(RUN_ID) {
        json_call(input, |r| functions::run(deps, r)).await
    } else if request.path == route_path(STATUS_ID) {
        json_call(input, |r| functions::status(deps, r)).await
    } else {
        return response(
            404,
            json!({ "error": format!("no memory-consolidate function at {}", request.path) }),
        );
    };
    match result {
        Ok(body) => response(200, body),
        Err(Failure::Input(message)) => response(400, json!({ "error": message })),
        Err(Failure::Function(error)) => response(500, json!({ "error": error.to_string() })),
    }
}

enum Failure {
    /// The body does not fit the function's input.
    Input(String),
    /// The function failed.
    Function(Error),
}

async fn json_call<Req, Resp, F, Fut>(input: Value, handler: F) -> Result<Value, Failure>
where
    Req: DeserializeOwned,
    Resp: Serialize,
    F: FnOnce(Req) -> Fut,
    Fut: std::future::Future<Output = Result<Resp, Error>>,
{
    let request = serde_json::from_value(input).map_err(|e| Failure::Input(e.to_string()))?;
    let response = handler(request).await.map_err(Failure::Function)?;
    serde_json::to_value(response).map_err(|e| Failure::Function(e.into()))
}

fn response(status_code: u16, body: Value) -> RestResponse {
    RestResponse { status_code, body }
}

/// Register [`REST_ID`], the target of both REST routes.
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

    fn offline_deps(cfg: WorkerConfig) -> Deps {
        Deps {
            iii: Arc::new(IIIClient::new("ws://127.0.0.1:1")),
            config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
            run_gate: tokio::sync::Mutex::new(()),
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

    #[test]
    fn routes_are_the_function_ids_with_slashes() {
        assert_eq!(route_path(RUN_ID), "/memory-consolidate/run");
        assert_eq!(route_path(STATUS_ID), "/memory-consolidate/status");
    }

    /// The run route reads `bank` from the body: a bank outside the
    /// allowlist is refused before any pass starts, where an ignored body
    /// would have run a pass over every configured bank.
    #[tokio::test]
    async fn the_run_route_reads_the_body() {
        let deps = offline_deps(WorkerConfig {
            banks: vec!["main".into()],
            ..WorkerConfig::default()
        });
        let response = handle(
            &deps,
            http_request(
                "/memory-consolidate/run",
                json!({ "bank": "blog", "dry_run": true }),
            ),
        )
        .await;
        assert_eq!(response.status_code, 500, "{:?}", response.body);
        let error = response.body["error"].as_str().unwrap();
        assert!(
            error.contains("`blog` is outside the configured allowlist"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_body_that_does_not_fit_is_a_bad_request() {
        let deps = offline_deps(WorkerConfig::default());
        let response = handle(
            &deps,
            http_request("/memory-consolidate/run", json!({ "dry_run": "yes" })),
        )
        .await;
        assert_eq!(response.status_code, 400, "{:?}", response.body);
        assert!(response.body["error"].is_string());
    }

    #[tokio::test]
    async fn other_paths_are_not_found() {
        let deps = offline_deps(WorkerConfig::default());
        let response = handle(
            &deps,
            http_request("/memory-consolidate/on-tick", Value::Null),
        )
        .await;
        assert_eq!(response.status_code, 404);
    }
}
