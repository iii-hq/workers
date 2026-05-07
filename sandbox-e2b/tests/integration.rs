//! Integration tests for sandbox-e2b. These run the handlers against a
//! local wiremock that mimics the E2B REST surface, so the suite covers
//! the wired client paths (create/stop/list) without touching the
//! production E2B API. Tests stay offline-safe.

use std::sync::Arc;

use sandbox_e2b::client::E2bClient;
use sandbox_e2b::config::Config;
use sandbox_e2b::handler::{do_create, do_exec, do_list, do_stop, HandlerCtx};
use sandbox_e2b::SCode;
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
    let client = Arc::new(E2bClient::new(cfg.api_base.clone(), "test-key".to_string()));
    HandlerCtx::new(Arc::new(cfg), client)
}

#[tokio::test]
async fn create_rejects_image_not_in_allowlist() {
    let server = MockServer::start().await;
    // No upstream mock needed — allowlist check happens before HTTP.
    let ctx = ctx(&server, 10, vec!["python".to_string()]).await;
    let err = do_create(&ctx, json!({ "image": "node" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::ImageNotAllowed);
}

#[tokio::test]
async fn create_rolls_back_in_flight_on_upstream_5xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 2, vec![]).await;
    let res = do_create(&ctx, json!({ "image": "base" })).await;
    assert!(matches!(
        res.err().map(|e| e.code()),
        Some(SCode::ProviderUnavailable)
    ));

    let list = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(list["in_flight"], 0, "in_flight must roll back on failure");
    assert_eq!(list["cap"], 2);
    assert_eq!(list["remaining"], 2);
}

#[tokio::test]
async fn create_maps_401_to_auth_invalid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let err = do_create(&ctx, json!({ "image": "base" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::AuthInvalid);
}

#[tokio::test]
async fn create_maps_429_to_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let err = do_create(&ctx, json!({ "image": "base" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::RateLimited);
}

#[tokio::test]
async fn create_happy_path_returns_capabilities() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "sandboxID": "sbx-abc",
            "templateID": "rki5dems9wqfm4r03t7g",
            "alias": "base",
            "clientID": "test",
            "envdVersion": "0.5.14"
        })))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 5, vec![]).await;
    let resp = do_create(&ctx, json!({ "image": "base" })).await.unwrap();
    assert_eq!(resp["sandbox_id"], "sbx-abc");
    assert_eq!(resp["image"], "base");
    let caps = resp["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|v| v == "snapshot"));
    assert!(caps.iter().any(|v| v == "expose_port"));
    assert!(caps.iter().any(|v| v == "fs"));
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
    // Regression: previously a sandbox that auto-died before the caller
    // got a chance to stop it surfaced as S502, leaking the in-flight
    // counter and confusing the caller. DELETE is idempotent — 404 means
    // the desired post-state is already in place.
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/sbx-gone"))
        .respond_with(ResponseTemplate::new(404).set_body_string("sandbox gone"))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 3, vec![]).await;
    // Pretend a create slot was already taken so we can prove stop releases it.
    ctx.in_flight.store(1, std::sync::atomic::Ordering::SeqCst);

    let res = do_stop(&ctx, json!({ "sandbox_id": "sbx-gone" }))
        .await
        .unwrap();
    assert_eq!(res, json!({}));
    assert_eq!(
        ctx.in_flight.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "404 stop must still release the in-flight slot"
    );
}

#[tokio::test]
async fn stop_maps_5xx_to_provider_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/sbx-1"))
        .respond_with(ResponseTemplate::new(502).set_body_string("upstream"))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    let err = do_stop(&ctx, json!({ "sandbox_id": "sbx-1" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::ProviderUnavailable);
}

#[tokio::test]
async fn list_reconciles_in_flight_to_upstream_count() {
    // E2B reaps idle sandboxes without notifying us. After reconciliation,
    // in_flight must match what the upstream actually has, regardless of
    // what creates have run on this worker.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "sandboxID": "live-1", "templateID": "tpl-1", "alias": "base", "startedAt": "2026-05-07T11:00:00Z" }
        ])))
        .mount(&server)
        .await;

    let ctx = ctx(&server, 5, vec![]).await;
    // Simulate drift: worker thinks it has 4 outstanding when reality is 1.
    ctx.in_flight.store(4, std::sync::atomic::Ordering::SeqCst);

    let res = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(
        res["in_flight"], 1,
        "list must reconcile drift to upstream count"
    );
    assert_eq!(res["cap"], 5);
    assert_eq!(res["remaining"], 4);
    assert_eq!(res["reconciled"], true);
    assert_eq!(res["sandboxes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn list_falls_back_to_local_counter_when_upstream_down() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let ctx = ctx(&server, 5, vec![]).await;
    ctx.in_flight.store(2, std::sync::atomic::Ordering::SeqCst);

    let res = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(
        res["in_flight"], 2,
        "must keep local counter when upstream is down"
    );
    assert_eq!(res["reconciled"], false);
    assert_eq!(res["sandboxes"].as_array().unwrap().len(), 0);
}
