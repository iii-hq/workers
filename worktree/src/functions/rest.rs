//! The REST surface: `POST /worktree/...` through the http worker.
//!
//! The worker observes `http::on-request` once per read-only function
//! ([`ROUTED`]), with args `{ method: "POST", path }`, the path being the
//! function id with `::` as `/` ([`route_path`]). Every route targets
//! [`REST_ID`]. It receives the http worker's `HttpRequest`, runs the
//! function the route's path names with the request's JSON `body` as its
//! input (an empty body is `{}`), and returns the `HttpResponse`
//! `{ status_code, body }`: `200` with the function's result, or an error
//! status with `{ error: { code, message } }`, `code` being the function's
//! `W###` code ([`status_for`]). A body that is not JSON answers `400`
//! `W001` and runs no function: the http worker hands such a body over as
//! `null`, and the request's `content-length` above 0 or its
//! `transfer-encoding` tells it from an empty one ([`RestRequest::sent_a_body`]).
//!
//! | Status | `code` |
//! |---|---|
//! | 400 | `W001` (also a body that is not JSON or does not fit the function's input), `W110`, `W111`, `W112` |
//! | 403 | `W500`-`W504` |
//! | 404 | `W200`, `W201`, and `not_found` for a path no routed function has |
//! | 409 | `W120`, `W210`-`W222`, `W401`-`W413` |
//! | 503 | `W300` |
//! | 504 | `W101` |
//! | 500 | any other code |

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{codes, WError};
use crate::functions::{get, list, status, validate, Deps};

pub const REST_ID: &str = "worktree::rest";
pub const REST_DESC: &str =
    "Internal: target of the worktree REST routes, the http::on-request observers with args \
     { method: POST, path: /worktree/list | /worktree/get | /worktree/status | \
     /worktree/validate }. Receives the HttpRequest, runs the worktree::* function the \
     route's path names with the request's JSON body (an empty body is {}), and returns \
     the HttpResponse { status_code, body }: 200 with the function's result, or an error \
     status with { error: { code, message } }. A body that is not JSON answers 400 W001.";

/// The functions served over HTTP: the read-only surface.
pub const ROUTED: [&str; 4] = [
    "worktree::list",
    "worktree::get",
    "worktree::status",
    "worktree::validate",
];

/// The `HttpRequest` fields the adapter reads. The http worker sends more
/// (`method`, `query_params`, `path_params` and the stream channels); they
/// are ignored.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestRequest {
    /// The matched route's path: `/worktree/...`, the function id with `::`
    /// as `/`.
    pub path: String,
    /// The request headers. `content-length` and `transfer-encoding` tell a
    /// body that is not JSON from an empty one.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// The request body parsed as JSON; `null` when it is empty or not JSON.
    #[serde(default)]
    pub body: Value,
}

impl RestRequest {
    /// Whether the client sent body bytes: a `content-length` above 0, or a
    /// `transfer-encoding` (a chunked body carries no length). Header names
    /// match in any case.
    pub fn sent_a_body(&self) -> bool {
        self.headers.iter().any(|(name, value)| {
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<u64>().is_ok_and(|length| length > 0)
            } else {
                name.eq_ignore_ascii_case("transfer-encoding")
            }
        })
    }

    /// The routed function's input: the JSON body, `{}` for an empty body,
    /// and `W001` for a body that is not JSON (`null` although bytes were
    /// sent).
    fn take_input(&mut self, function_id: &str) -> Result<Value, WError> {
        match std::mem::take(&mut self.body) {
            Value::Null if self.sent_a_body() => Err(WError::new(
                codes::INVALID_REQUEST,
                format!("invalid {function_id} input: the request body is not a JSON object"),
            )),
            Value::Null => Ok(json!({})),
            body => Ok(body),
        }
    }
}

/// The `HttpResponse` the http worker answers with.
#[derive(Debug, PartialEq, Serialize, JsonSchema)]
pub struct RestResponse {
    pub status_code: u16,
    pub body: Value,
}

/// The REST path of a function: the function id with `::` as `/`.
pub fn route_path(function_id: &str) -> String {
    format!("/{}", function_id.replace("::", "/"))
}

/// Run the routed function `request.path` names and answer as HTTP.
pub async fn handle(deps: &Deps, mut request: RestRequest) -> RestResponse {
    let function_id = request
        .path
        .strip_prefix('/')
        .unwrap_or(&request.path)
        .replace('/', "::");
    let result = match request.take_input(&function_id) {
        Ok(input) => call_json(deps, &function_id, input).await,
        Err(error) if ROUTED.contains(&function_id.as_str()) => Some(Err(error)),
        Err(_) => None,
    };
    match result {
        Some(Ok(body)) => RestResponse {
            status_code: 200,
            body,
        },
        Some(Err(error)) => RestResponse {
            status_code: status_for(error.code),
            body: error_body(error.code, &error.message),
        },
        None => RestResponse {
            status_code: 404,
            body: error_body(
                "not_found",
                &format!("no worktree function at {}", request.path),
            ),
        },
    }
}

/// Run a routed function with a JSON input; `None` for any other id.
async fn call_json(deps: &Deps, function_id: &str, input: Value) -> Option<Result<Value, WError>> {
    let result = match function_id {
        "worktree::list" => json_call(function_id, input, |r| list::handle(deps, r)).await,
        "worktree::get" => json_call(function_id, input, |r| get::handle(deps, r)).await,
        "worktree::status" => json_call(function_id, input, |r| status::handle(deps, r)).await,
        "worktree::validate" => json_call(function_id, input, |r| validate::handle(deps, r)).await,
        _ => return None,
    };
    Some(result)
}

async fn json_call<Req, Resp, F, Fut>(
    function_id: &str,
    input: Value,
    handler: F,
) -> Result<Value, WError>
where
    Req: DeserializeOwned,
    Resp: Serialize,
    F: FnOnce(Req) -> Fut,
    Fut: Future<Output = Result<Resp, WError>>,
{
    let request = serde_json::from_value(input).map_err(|e| {
        WError::new(
            codes::INVALID_REQUEST,
            format!("invalid {function_id} input: {e}"),
        )
    })?;
    let response = handler(request).await?;
    serde_json::to_value(response).map_err(|e| {
        WError::new(
            codes::STATE_CORRUPT,
            format!("the {function_id} response did not serialize: {e}"),
        )
    })
}

/// The HTTP status of a `W###` code.
pub fn status_for(code: &str) -> u16 {
    match code {
        codes::INVALID_REQUEST | codes::NOT_A_REPO | codes::BAD_PATH | codes::REF_NOT_FOUND => 400,
        codes::OP_DISABLED
        | codes::FORCE_DISABLED
        | codes::LAND_TARGET_NOT_ALLOWED
        | codes::REPO_NOT_ALLOWED
        | codes::WORKTREE_BUDGET_EXCEEDED => 403,
        codes::NOT_FOUND | codes::PATH_MISSING => 404,
        codes::BRANCH_EXISTS
        | codes::ALREADY_CLAIMED
        | codes::CLAIM_MISMATCH
        | codes::DIRTY
        | codes::UNMERGED_WORK
        | codes::WORKTREE_BUSY
        | codes::LAND_IN_PROGRESS
        | codes::LAND_CONFLICTED
        | codes::REBASE_CONFLICT
        | codes::TEST_FAILED
        | codes::TARGET_MOVED_EXHAUSTED
        | codes::TARGET_CHECKED_OUT_DIRTY => 409,
        codes::STATE_UNAVAILABLE => 503,
        codes::GIT_TIMEOUT => 504,
        _ => 500,
    }
}

fn error_body(code: &str, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message } })
}

/// Register [`REST_ID`], the target of every REST route.
pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
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

    #[test]
    fn routes_are_the_function_ids_with_slashes() {
        assert_eq!(route_path("worktree::list"), "/worktree/list");
        assert_eq!(route_path("worktree::validate"), "/worktree/validate");
    }

    #[test]
    fn every_code_has_a_status() {
        assert_eq!(status_for(codes::INVALID_REQUEST), 400);
        assert_eq!(status_for(codes::BAD_PATH), 400);
        assert_eq!(status_for(codes::NOT_FOUND), 404);
        assert_eq!(status_for(codes::PATH_MISSING), 404);
        assert_eq!(status_for(codes::ALREADY_CLAIMED), 409);
        assert_eq!(status_for(codes::REPO_NOT_ALLOWED), 403);
        assert_eq!(status_for(codes::STATE_UNAVAILABLE), 503);
        assert_eq!(status_for(codes::GIT_TIMEOUT), 504);
        assert_eq!(status_for(codes::GIT_NONZERO), 500);
        assert_eq!(status_for(codes::STATE_CORRUPT), 500);
    }

    #[test]
    fn the_request_reads_path_and_body_of_the_whole_http_request() {
        let request: RestRequest = serde_json::from_value(json!({
            "query_params": {},
            "path_params": {},
            "headers": { "content-type": "application/json" },
            "path": "/worktree/get",
            "method": "POST",
            "body": { "worktree_id": "wt_1" },
            "request_body": { "channel_id": "c1", "access_key": "k", "direction": "read" },
            "response": { "channel_id": "c2", "access_key": "k", "direction": "write" },
        }))
        .unwrap();
        assert_eq!(request.path, "/worktree/get");
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(request.body, json!({ "worktree_id": "wt_1" }));
    }

    fn request_with_headers(headers: Value) -> RestRequest {
        serde_json::from_value(json!({
            "path": "/worktree/list",
            "headers": headers,
            "body": null,
        }))
        .unwrap()
    }

    #[test]
    fn a_length_above_zero_or_a_transfer_encoding_is_a_sent_body() {
        assert!(request_with_headers(json!({ "content-length": "14" })).sent_a_body());
        assert!(request_with_headers(json!({ "Content-Length": "1" })).sent_a_body());
        assert!(request_with_headers(json!({ "transfer-encoding": "chunked" })).sent_a_body());
        assert!(!request_with_headers(json!({ "content-length": "0" })).sent_a_body());
        assert!(!request_with_headers(json!({ "content-type": "application/json" })).sent_a_body());
        assert!(!request_with_headers(json!({})).sent_a_body());
    }

    #[test]
    fn a_null_body_is_an_empty_input_unless_bytes_were_sent() {
        let mut empty = request_with_headers(json!({ "content-length": "0" }));
        assert_eq!(empty.take_input("worktree::list").unwrap(), json!({}));

        let mut not_json = request_with_headers(json!({ "content-length": "17" }));
        let error = not_json.take_input("worktree::list").unwrap_err();
        assert_eq!(error.code, codes::INVALID_REQUEST);
        assert!(
            error.message.contains("not a JSON object"),
            "{}",
            error.message
        );

        let mut object: RestRequest = serde_json::from_value(json!({
            "path": "/worktree/list",
            "headers": { "content-length": "2" },
            "body": {},
        }))
        .unwrap();
        assert_eq!(object.take_input("worktree::list").unwrap(), json!({}));
    }
}
