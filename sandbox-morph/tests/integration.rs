//! Integration tests for sandbox-morph. Runs the handlers against a
//! local wiremock that mimics Morph Cloud's REST surface (`/api/instance`
//! family) so the suite exercises wired client paths without touching
//! production morph.so. Tests stay offline-safe.

use std::sync::Arc;

use sandbox_morph::client::MorphClient;
use sandbox_morph::config::Config;
use sandbox_morph::handler::{
    do_branch, do_create, do_exec, do_list, do_snapshot, do_stop, HandlerCtx,
};
use sandbox_morph::SCode;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn ctx(server: &MockServer, max: usize, allowlist: Vec<String>) -> HandlerCtx {
    let cfg = Config {
        api_base: server.uri(),
        max_concurrent_sandboxes: max,
        image_allowlist: allowlist,
        ..Config::default()
    };
    let client = Arc::new(MorphClient::new(
        cfg.api_base.clone(),
        "test-key".to_string(),
    ));
    HandlerCtx::new(Arc::new(cfg), client)
}

#[tokio::test]
async fn create_rejects_image_not_in_allowlist() {
    let server = MockServer::start().await;
    let ctx = ctx(&server, 10, vec!["snapshot_a".to_string()]).await;
    let err = do_create(&ctx, json!({ "image": "snapshot_b" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::ImageNotAllowed);
}

#[tokio::test]
async fn create_rejects_empty_image() {
    let server = MockServer::start().await;
    let ctx = ctx(&server, 5, vec![]).await;
    let err = do_create(&ctx, json!({ "image": "" })).await.err().unwrap();
    let s = err.to_string();
    assert!(s.contains("morph requires a non-empty"), "got: {s}");
}

#[tokio::test]
async fn create_happy_path_returns_branch_capability() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "morphvm_abc",
            "status": "ready",
            "refs": { "snapshot_id": "snapshot_xyz" },
            "createdAt": "2026-05-07T11:00:00Z"
        })))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 5, vec![]).await;
    let resp = do_create(&ctx, json!({ "image": "snapshot_xyz" }))
        .await
        .unwrap();
    assert_eq!(resp["sandbox_id"], "morphvm_abc");
    let caps = resp["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|v| v == "branch"));
    assert!(caps.iter().any(|v| v == "snapshot"));
    assert!(caps.iter().any(|v| v == "expose_port"));
}

#[tokio::test]
async fn create_rolls_back_on_5xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/instance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 2, vec![]).await;
    let res = do_create(&ctx, json!({ "image": "snap_a" })).await;
    assert!(matches!(
        res.err().map(|e| e.code()),
        Some(SCode::ProviderUnavailable)
    ));
    let list = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(list["in_flight"], 0);
}

#[tokio::test]
async fn create_maps_401_to_auth_invalid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let err = do_create(&ctx, json!({ "image": "snap_a" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::AuthInvalid);
}

#[tokio::test]
async fn exec_passes_command_array() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance/morphvm_abc/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "stdout": "hello\n",
            "stderr": "",
            "exit_code": 0
        })))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let res = do_exec(
        &ctx,
        json!({
            "sandbox_id": "morphvm_abc",
            "cmd": "echo",
            "args": ["hello"]
        }),
    )
    .await
    .unwrap();
    assert_eq!(res["stdout"], "hello\n");
    assert_eq!(res["exit_code"], 0);
}

#[tokio::test]
async fn exec_rejects_missing_fields() {
    let server = MockServer::start().await;
    let ctx = ctx(&server, 10, vec![]).await;
    let err = do_exec(&ctx, json!({})).await.err().unwrap();
    let s = err.to_string();
    assert!(s.contains("missing string field"), "got: {s}");
}

#[tokio::test]
async fn stop_treats_404_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance/morphvm_gone/stop"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 3, vec![]).await;
    ctx.in_flight.store(1, std::sync::atomic::Ordering::SeqCst);
    let res = do_stop(&ctx, json!({ "sandbox_id": "morphvm_gone" }))
        .await
        .unwrap();
    assert_eq!(res, json!({}));
    assert_eq!(ctx.in_flight.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stop_treats_409_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance/morphvm_busy/stop"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 3, vec![]).await;
    ctx.in_flight.store(1, std::sync::atomic::Ordering::SeqCst);
    let res = do_stop(&ctx, json!({ "sandbox_id": "morphvm_busy" }))
        .await
        .unwrap();
    assert_eq!(res, json!({}));
}

#[tokio::test]
async fn stop_maps_5xx_to_provider_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance/morphvm_abc/stop"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let err = do_stop(&ctx, json!({ "sandbox_id": "morphvm_abc" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::ProviderUnavailable);
}

#[tokio::test]
async fn list_reconciles_in_flight_to_upstream_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/instance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": "morphvm_a",
                "status": "ready",
                "refs": { "snapshot_id": "snap_a" }
            }
        ])))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 5, vec![]).await;
    ctx.in_flight.store(4, std::sync::atomic::Ordering::SeqCst);
    let res = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(res["in_flight"], 1);
    assert_eq!(res["reconciled"], true);
    assert_eq!(res["sandboxes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn list_falls_back_when_upstream_down() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/instance"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    ctx.in_flight.store(2, std::sync::atomic::Ordering::SeqCst);
    let res = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(res["in_flight"], 2);
    assert_eq!(res["reconciled"], false);
}

#[tokio::test]
async fn branch_returns_sibling_ids() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance/morphvm_abc/branch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "snapshot": { "id": "snap_branch" },
            "instances": [
                { "id": "morphvm_b1" },
                { "id": "morphvm_b2" }
            ]
        })))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 10, vec![]).await;
    let res = do_branch(&ctx, json!({ "sandbox_id": "morphvm_abc", "count": 2 }))
        .await
        .unwrap();
    let ids = res["sandbox_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], "morphvm_b1");
    assert_eq!(ids[1], "morphvm_b2");
}

#[tokio::test]
async fn branch_rejects_count_zero() {
    let server = MockServer::start().await;
    let ctx = ctx(&server, 10, vec![]).await;
    let err = do_branch(&ctx, json!({ "sandbox_id": "morphvm_abc", "count": 0 }))
        .await
        .err()
        .unwrap();
    let s = err.to_string();
    assert!(s.contains("branch count"), "got: {s}");
}

#[tokio::test]
async fn snapshot_returns_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/instance/morphvm_abc/snapshot"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "snapshot_xyz"
        })))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let res = do_snapshot(&ctx, json!({ "sandbox_id": "morphvm_abc" }))
        .await
        .unwrap();
    assert_eq!(res["snapshot_id"], "snapshot_xyz");
}
