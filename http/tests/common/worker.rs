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

use iii_sdk::IIIClient;
use iii_http::boot::{self, BootHandle};
use iii_http::config::RestApiConfig;

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
