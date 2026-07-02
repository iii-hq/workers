#![allow(dead_code)]
//! Boots the `iii-http` worker against the shared engine for e2e tests.
//!
//! One worker is booted per test: each `#[tokio::test]` runs on its own
//! runtime, and the server task is spawned on that runtime, so it must be
//! started (and live) within the test that uses it. The shared engine client
//! is runtime-independent and is reused across tests.
//!
//! Tests must boot the worker *before* registering a backend trigger so the
//! `http` trigger type exists when the binding arrives, making delivery to the
//! freshly-registered handler live (no reliance on replay into a new table).

use std::sync::Arc;

use iii_http::boot::{self, BootHandle};
use iii_http::config::{CorsConfig, MiddlewareConfig, RestApiConfig};
use iii_sdk::IIIClient;

/// Start the HTTP worker on an ephemeral loopback port. Returns the
/// [`BootHandle`] whose `local_addr` the test issues requests against and
/// whose `routes` it polls via [`crate::common::wait_for_route`].
pub async fn start_http_worker(iii: Arc<IIIClient>) -> BootHandle {
    boot::start(
        iii,
        RestApiConfig {
            port: 0,
            host: "127.0.0.1".to_string(),
            ..RestApiConfig::default()
        },
    )
    .await
    .expect("http worker should boot")
}

/// Same as [`start_http_worker`], but boots bound to an explicit `host`/`port`
/// (instead of the ephemeral `port: 0`). Used by the rebind test, which needs
/// deterministic ports to prove the listener moved from one to another.
#[allow(dead_code)]
pub async fn start_http_worker_on(iii: Arc<IIIClient>, host: &str, port: u16) -> BootHandle {
    boot::start(
        iii,
        RestApiConfig {
            port,
            host: host.to_string(),
            ..RestApiConfig::default()
        },
    )
    .await
    .expect("http worker should boot")
}

/// Same as [`start_http_worker`], but boots with `config.middleware` set to
/// `middleware_function_ids` (each entry defaulted to `preHandler`/priority
/// 0), so global middleware tests can exercise the config-driven path.
#[allow(dead_code)]
pub async fn start_http_worker_with_global_middleware(
    iii: Arc<IIIClient>,
    middleware_function_ids: &[&str],
) -> BootHandle {
    let middleware = middleware_function_ids
        .iter()
        .map(|function_id| MiddlewareConfig {
            function_id: function_id.to_string(),
            phase: "preHandler".to_string(),
            priority: 0,
        })
        .collect();

    boot::start(
        iii,
        RestApiConfig {
            port: 0,
            host: "127.0.0.1".to_string(),
            middleware,
            ..RestApiConfig::default()
        },
    )
    .await
    .expect("http worker should boot")
}

/// Same as [`start_http_worker`], but boots with `config.cors` set to `cors`,
/// so CORS-from-config tests can exercise the non-permissive path of
/// `build_cors_layer`.
#[allow(dead_code)]
pub async fn start_http_worker_with_cors(iii: Arc<IIIClient>, cors: CorsConfig) -> BootHandle {
    boot::start(
        iii,
        RestApiConfig {
            port: 0,
            host: "127.0.0.1".to_string(),
            cors: Some(cors),
            ..RestApiConfig::default()
        },
    )
    .await
    .expect("http worker should boot")
}

/// Same as [`start_http_worker`], but boots with `config.default_timeout` set
/// to `timeout_ms`, so timeout-parity tests can exercise the tower
/// `TimeoutLayer` (504) / invocation-timeout race with a small, test-scale
/// deadline instead of the 30s default.
#[allow(dead_code)]
pub async fn start_http_worker_with_timeout(iii: Arc<IIIClient>, timeout_ms: u64) -> BootHandle {
    boot::start(
        iii,
        RestApiConfig {
            port: 0,
            host: "127.0.0.1".to_string(),
            default_timeout: timeout_ms,
            ..RestApiConfig::default()
        },
    )
    .await
    .expect("http worker should boot")
}

/// Same as [`start_http_worker`], but boots with `config.concurrency_request_limit`
/// set to `limit`, so concurrency-limit tests can exercise the tower
/// `ConcurrencyLimitLayer` with a small, test-scale limit instead of the 1024
/// default.
#[allow(dead_code)]
pub async fn start_http_worker_with_concurrency_limit(
    iii: Arc<IIIClient>,
    limit: usize,
) -> BootHandle {
    boot::start(
        iii,
        RestApiConfig {
            port: 0,
            host: "127.0.0.1".to_string(),
            concurrency_request_limit: limit,
            ..RestApiConfig::default()
        },
    )
    .await
    .expect("http worker should boot")
}
