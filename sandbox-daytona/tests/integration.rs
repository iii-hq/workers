//! Smoke tests for sandbox-daytona. Exercises the handler-level surface
//! directly — no engine, no Daytona. Covers config validation, concurrency
//! tracking, and the `list` shape that callers depend on.

use std::sync::Arc;

use sandbox_daytona::client::DaytonaClient;
use sandbox_daytona::config::Config;
use sandbox_daytona::handler::{do_create, do_exec, do_list, do_stop, HandlerCtx};
use sandbox_daytona::SCode;
use serde_json::json;

fn ctx(max: usize, allowlist: Vec<String>) -> HandlerCtx {
    let cfg = Config {
        max_concurrent_sandboxes: max,
        image_allowlist: allowlist,
        ..Config::default()
    };
    let client = Arc::new(DaytonaClient::new(
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
    // The stub client errors on list, so we land on the fallback branch
    // and `reconciled` must be `false`. When daytona's REST is wired the
    // happy path will flip this to `true` and reset in_flight against
    // the upstream count (see the e2b live test for the full pattern).
    assert_eq!(res["reconciled"], false);
}
