//! `worktree::rest`, the target of the REST routes, in-process: each routed
//! path runs its function with the request's JSON body and answers
//! `{ status_code, body }`; a `W###` error answers its status with
//! `{ error: { code, message } }`; a body that is not JSON is a 400 `W001`;
//! a path no routed function has is a 404.

mod support;

use serde_json::{json, Value};
use support::{create_request, init_repo, make_env, test_config};
use worktree::functions::create;
use worktree::functions::rest::{self, RestRequest, ROUTED};

/// What the http worker sends a route's target: the whole `HttpRequest`.
fn http_request(path: &str, body: Value) -> RestRequest {
    let length = match &body {
        Value::Null => 0,
        body => body.to_string().len(),
    };
    http_request_with(
        path,
        json!({ "content-type": "application/json", "content-length": length.to_string() }),
        body,
    )
}

/// An `HttpRequest` with these headers.
fn http_request_with(path: &str, headers: Value, body: Value) -> RestRequest {
    serde_json::from_value(json!({
        "query_params": {},
        "path_params": {},
        "headers": headers,
        "path": path,
        "method": "POST",
        "body": body,
        "request_body": { "channel_id": "c1", "access_key": "k", "direction": "read" },
        "response": { "channel_id": "c2", "access_key": "k", "direction": "write" },
    }))
    .unwrap()
}

#[tokio::test]
async fn each_route_runs_its_function_with_the_body() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let created = create::handle(&env.deps, create_request(&repo))
        .await
        .unwrap();

    let listed = rest::handle(&env.deps, http_request("/worktree/list", json!({}))).await;
    assert_eq!(listed.status_code, 200, "{:?}", listed.body);
    let worktrees = listed.body["worktrees"].as_array().unwrap();
    assert_eq!(worktrees.len(), 1, "{:?}", listed.body);
    assert_eq!(worktrees[0]["worktree_id"], created.worktree_id.as_str());

    let got = rest::handle(
        &env.deps,
        http_request(
            "/worktree/get",
            json!({ "worktree_id": created.worktree_id }),
        ),
    )
    .await;
    assert_eq!(got.status_code, 200, "{:?}", got.body);
    assert_eq!(got.body["worktree_id"], created.worktree_id.as_str());
    assert_eq!(got.body["path"], created.path.as_str());

    let status = rest::handle(
        &env.deps,
        http_request(
            "/worktree/status",
            json!({ "worktree_id": created.worktree_id }),
        ),
    )
    .await;
    assert_eq!(status.status_code, 200, "{:?}", status.body);
    assert_eq!(status.body["worktree_id"], created.worktree_id.as_str());
    assert_eq!(status.body["clean"], true);

    let valid = rest::handle(
        &env.deps,
        http_request("/worktree/validate", json!({ "path": created.path })),
    )
    .await;
    assert_eq!(valid.status_code, 200, "{:?}", valid.body);
    assert_eq!(valid.body["valid"], true);
    assert_eq!(valid.body["worktree_id"], created.worktree_id.as_str());
}

#[tokio::test]
async fn an_empty_body_is_an_empty_input() {
    let tmp = tempfile::tempdir().unwrap();
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let listed = rest::handle(&env.deps, http_request("/worktree/list", Value::Null)).await;
    assert_eq!(listed.status_code, 200, "{:?}", listed.body);
    assert_eq!(listed.body["worktrees"], json!([]));
}

/// The http worker hands a body that is not JSON over as `null`; its
/// `content-length` (or `transfer-encoding`) says bytes were sent. Such a
/// request answers 400 `W001` and runs no function.
#[tokio::test]
async fn a_body_that_is_not_json_answers_400_w001() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    create::handle(&env.deps, create_request(&repo))
        .await
        .unwrap();

    for headers in [
        json!({ "content-type": "application/json", "content-length": "17" }),
        json!({ "content-type": "application/json", "transfer-encoding": "chunked" }),
    ] {
        for function_id in ROUTED {
            let response = rest::handle(
                &env.deps,
                http_request_with(&rest::route_path(function_id), headers.clone(), Value::Null),
            )
            .await;
            assert_eq!(response.status_code, 400, "{function_id} {headers}");
            assert_eq!(response.body["error"]["code"], "W001", "{function_id}");
            assert!(
                response.body["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("not a JSON object"),
                "{:?}",
                response.body
            );
        }
    }

    // A path no routed function has stays a 404 whatever its body.
    let response = rest::handle(
        &env.deps,
        http_request_with(
            &rest::route_path("worktree::remove"),
            json!({ "content-length": "17" }),
            Value::Null,
        ),
    )
    .await;
    assert_eq!(response.status_code, 404);
    assert_eq!(response.body["error"]["code"], "not_found");
}

#[tokio::test]
async fn errors_answer_with_a_status_and_a_code() {
    let tmp = tempfile::tempdir().unwrap();
    let env = make_env(tmp.path(), test_config(tmp.path()));

    let unknown = rest::handle(
        &env.deps,
        http_request("/worktree/get", json!({ "worktree_id": "wt_absent" })),
    )
    .await;
    assert_eq!(unknown.status_code, 404, "{:?}", unknown.body);
    assert_eq!(unknown.body["error"]["code"], "W200");

    let missing_field = rest::handle(&env.deps, http_request("/worktree/status", json!({}))).await;
    assert_eq!(missing_field.status_code, 400, "{:?}", missing_field.body);
    assert_eq!(missing_field.body["error"]["code"], "W001");
    assert!(
        missing_field.body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("worktree_id"),
        "{:?}",
        missing_field.body
    );

    let relative = rest::handle(
        &env.deps,
        http_request("/worktree/validate", json!({ "path": "relative/dir" })),
    )
    .await;
    assert_eq!(relative.status_code, 400, "{:?}", relative.body);
    assert_eq!(relative.body["error"]["code"], "W111");
}

#[tokio::test]
async fn only_the_routed_functions_have_a_route() {
    let tmp = tempfile::tempdir().unwrap();
    let env = make_env(tmp.path(), test_config(tmp.path()));
    // A string fits no request type: each routed function decodes it and
    // refuses, so none of them runs.
    for function_id in ROUTED {
        let response = rest::handle(
            &env.deps,
            http_request(&rest::route_path(function_id), json!("not an object")),
        )
        .await;
        assert_eq!(response.status_code, 400, "{function_id}");
        assert_eq!(response.body["error"]["code"], "W001", "{function_id}");
    }
    for function_id in [
        "worktree::create",
        "worktree::remove",
        "worktree::land",
        "worktree::land-step",
        rest::REST_ID,
    ] {
        let response = rest::handle(
            &env.deps,
            http_request(&rest::route_path(function_id), json!({})),
        )
        .await;
        assert_eq!(response.status_code, 404, "{function_id} has a route");
        assert_eq!(response.body["error"]["code"], "not_found");
    }
}
