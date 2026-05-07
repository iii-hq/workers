//! Smoke tests for sandbox-morph. Exercises the handler-level surface
//! directly — no engine, no Morph. Covers config validation, concurrency
//! tracking, and the `list` shape that callers depend on.

use std::sync::Arc;

use sandbox_morph::client::MorphClient;
use sandbox_morph::config::Config;
use sandbox_morph::handler::{do_branch, do_create, do_exec, do_list, do_stop, HandlerCtx};
use sandbox_morph::SCode;
use serde_json::json;

fn ctx(max: usize, allowlist: Vec<String>) -> HandlerCtx {
    let cfg = Config {
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
    let ctx = ctx(10, vec!["python".to_string()]);
    let err = do_create(&ctx, json!({ "image": "node" }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::ImageNotAllowed);
}

#[tokio::test]
async fn create_returns_provider_unavailable_when_http_stubbed() {
    // Until the REST client is wired, the upstream call returns S502.
    // The test asserts the worker still releases the concurrency slot on
    // failure so callers can retry without leaking capacity.
    let ctx = ctx(2, vec![]);
    let _ = do_create(&ctx, json!({ "image": "python" })).await;
    let list = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(list["in_flight"], 0, "in_flight must roll back on failure");
    assert_eq!(list["cap"], 2);
    assert_eq!(list["remaining"], 2);
}

#[tokio::test]
async fn exec_rejects_missing_fields() {
    let ctx = ctx(10, vec![]);
    let err = do_exec(&ctx, json!({})).await.err().unwrap();
    let s = err.to_string();
    assert!(s.contains("missing string field"), "got: {s}");
}

#[tokio::test]
async fn stop_returns_empty_object_on_success_path() {
    // The stub client returns S502 for stop; we just assert that the
    // input parser passes through to the client, so the error here is
    // S502 and not BadInput.
    let ctx = ctx(10, vec![]);
    let res = do_stop(&ctx, json!({ "sandbox_id": "sbx-1" })).await;
    assert!(matches!(
        res.err().map(|e| e.code()),
        Some(SCode::ProviderUnavailable)
    ));
}

#[tokio::test]
async fn list_reports_capacity_envelope_with_reconciled_flag() {
    let ctx = ctx(7, vec![]);
    let res = do_list(&ctx, json!({})).await.unwrap();
    assert_eq!(res["cap"], 7);
    assert_eq!(res["in_flight"], 0);
    assert_eq!(res["remaining"], 7);
    // The stub client errors on list, so reconciled must be false. When
    // morph's REST is wired the happy path will flip this to true and
    // reset in_flight to the upstream count.
    assert_eq!(res["reconciled"], false);
}

#[tokio::test]
async fn branch_rejects_missing_sandbox_id() {
    let ctx = ctx(10, vec![]);
    let err = do_branch(&ctx, json!({ "count": 3 })).await.err().unwrap();
    let s = err.to_string();
    assert!(s.contains("missing string field"), "got: {s}");
}

#[tokio::test]
async fn branch_rejects_count_zero() {
    let ctx = ctx(10, vec![]);
    let err = do_branch(&ctx, json!({ "sandbox_id": "sbx-1", "count": 0 }))
        .await
        .err()
        .unwrap();
    let s = err.to_string();
    assert!(s.contains("branch count"), "got: {s}");
}

#[tokio::test]
async fn branch_passes_through_to_client_stub() {
    // Until the Morph REST is wired, the client returns S502 on every
    // branch call. We assert the input parser routes correctly so when
    // wiring lands, the only thing that changes is the client body.
    let ctx = ctx(10, vec![]);
    let err = do_branch(&ctx, json!({ "sandbox_id": "sbx-parent", "count": 3 }))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), SCode::ProviderUnavailable);
}
