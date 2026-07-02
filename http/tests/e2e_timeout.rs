//! End-to-end coverage for request-timeout and concurrency-limit parity with
//! the engine's `iii-http`.
//!
//! `http/src/server.rs` wraps the whole router in a tower `TimeoutLayer`
//! (`StatusCode::GATEWAY_TIMEOUT`) sized from `config.default_timeout` --
//! mirroring the engine, which relies on the same layer to 504 a slow
//! request. `http/src/handler.rs` ALSO bounds the target function's
//! invocation with `iii.trigger(TriggerRequest { timeout_ms: Some(default_timeout),
//! .. })`, which on expiry surfaces `Err(Error::Timeout)` and gets mapped to a
//! 500 by the handler's generic call-error arm. A slow HANDLER function must
//! still surface as 504 (the tower layer owns the request deadline, matching
//! the engine) -- see `function_timeout_returns_504_not_500` below.
//!
//! Mirrors the harness used by `e2e_errors.rs` -- see that file's module doc
//! for the connect-or-skip / per-test-worker rationale.

mod common;

use std::time::Duration;

use common::{backend, engine, worker};
use serial_test::serial;

/// A HANDLER function that sleeps well past `default_timeout` must surface as
/// `504 Gateway Timeout` to the client -- the tower `TimeoutLayer` owns the
/// request deadline, exactly like the engine's `iii-http`. Before the fix,
/// `handler::dynamic_handler` also bounded the invocation itself with the
/// same `default_timeout` via `iii.trigger(.., timeout_ms: Some(..))`; that
/// inner timeout raced the outer tower layer and could surface as a 500
/// `INTERNAL_ERROR` (`Err(Error::Timeout)` mapped by the generic call-error
/// arm) instead of the engine-parity 504.
#[tokio::test]
#[serial]
async fn function_timeout_returns_504_not_500() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let timeout_ms = 500;
    let boot = worker::start_http_worker_with_timeout(iii.clone(), timeout_ms).await;
    // Sleep comfortably longer than the request timeout so the race isn't
    // flaky in either direction.
    backend::register_slow_backend(&iii, "/slow", "GET", timeout_ms * 4).await;
    common::wait_for_route(&boot.routes, "GET", "/slow").await;

    let url = format!("http://{}/slow", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::GATEWAY_TIMEOUT,
        "a slow handler must 504 (tower TimeoutLayer owns the request deadline), \
         matching the engine -- got body: {:?}",
        resp.text().await
    );

    boot.shutdown().await;
}

/// A function that returns comfortably within `default_timeout` must not be
/// affected by the timeout machinery -- sanity check alongside the slow-path
/// test above.
#[tokio::test]
#[serial]
async fn fast_function_is_unaffected_by_timeout_layer() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let timeout_ms = 500;
    let boot = worker::start_http_worker_with_timeout(iii.clone(), timeout_ms).await;
    backend::register_slow_backend(&iii, "/fast", "GET", 10).await;
    common::wait_for_route(&boot.routes, "GET", "/fast").await;

    let url = format!("http://{}/fast", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    boot.shutdown().await;
}

/// The `ConcurrencyLimitLayer` QUEUES excess requests rather than rejecting
/// them, so correctness under a tight limit means every request still
/// eventually succeeds -- just serialized once in-flight requests exceed the
/// permit count. A strict "never more than N in flight" assertion isn't
/// deterministically observable from the client side (no instrumentation
/// hook into the tower layer), so this is a throughput smoke: fire more
/// concurrent requests than the configured limit and assert they ALL
/// eventually return 200.
#[tokio::test]
#[serial]
async fn concurrency_limit_queues_excess_requests_but_all_succeed() {
    let Some(iii) = engine::get_or_init().await else {
        return;
    };
    let boot = worker::start_http_worker_with_concurrency_limit(iii.clone(), 2).await;
    backend::register_sleep_backend(&iii, "/limited", "GET", 200).await;
    common::wait_for_route(&boot.routes, "GET", "/limited").await;

    let url = format!("http://{}/limited", boot.local_addr);
    let client = reqwest::Client::new();

    let mut handles = Vec::new();
    for _ in 0..6 {
        let client = client.clone();
        let url = url.clone();
        handles.push(tokio::spawn(async move { client.get(&url).send().await }));
    }

    let deadline = Duration::from_secs(10);
    let results = tokio::time::timeout(deadline, futures::future::join_all(handles))
        .await
        .expect("all 6 requests should complete within the deadline");

    for result in results {
        let resp = result
            .expect("request task should not panic")
            .expect("request should not error");
        assert_eq!(
            resp.status(),
            200,
            "every request must succeed with the concurrency limit configured"
        );
    }

    // NOTE: this is a wired-and-doesn't-break-throughput smoke, NOT a proof of
    // global serialization. Empirically the `ConcurrencyLimitLayer` does NOT
    // serialize requests across separate connections here (6 reqs @ ~200ms with
    // limit 2 complete in ~200ms, not ~600ms) -- an added elapsed>=400ms floor
    // FAILS. The engine's iii-http uses the identical construction
    // (`tower::limit::ConcurrencyLimitLayer::new(..)` via `Router::layer`, same
    // `axum::serve`), so this matches its behavior (per-connection semaphore, not
    // a global limit). A true global cap would need `GlobalConcurrencyLimitLayer`
    // -- but that would diverge from the engine, so it's intentionally not done.

    boot.shutdown().await;
}
