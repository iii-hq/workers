//! Connect-or-fail wrapper around the iii SDK.
//!
//! These are e2e tests: they only make sense with a running engine, so if the
//! engine is unreachable `get_or_init` PANICS (the test fails) rather than
//! silently passing. One engine connection per test binary process via
//! `OnceCell` — the WebSocket handshake dwarfs per-test overhead.
//!
//! Point the tests at an engine with `III_ENGINE_WS_URL` (default
//! `ws://127.0.0.1:49134`).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{InitOptions, IIIClient, register_worker};
use serde_json::json;
use tokio::sync::OnceCell;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Arc<IIIClient>> = OnceCell::const_new();

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

async fn try_connect_raw() -> Option<Arc<IIIClient>> {
    let url = ws_url();
    let iii = Arc::new(register_worker(&url, InitOptions::default()));

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        match probe {
            Ok(_) => return Some(iii),
            Err(Error::NotConnected) => continue,
            Err(_) => continue,
        }
    }

    iii.shutdown_async().await;
    None
}

/// Open a fresh, dedicated engine connection (NOT the shared `OnceCell` one).
///
/// Needed when a test registers a function id that another test in the same
/// binary also registers (e.g. the `http::on-config-change` reload function):
/// registering the same id twice on one client panics, so such a test needs its
/// own client. Panics (fails the test) if no engine is reachable. The caller
/// should `shutdown_async()` the returned client when done.
#[allow(dead_code)]
pub async fn connect_fresh() -> Arc<IIIClient> {
    try_connect_raw().await.unwrap_or_else(|| {
        panic!(
            "e2e requires a running iii engine at {} — start `iii` or set \
             III_ENGINE_WS_URL to point at one",
            ws_url()
        )
    })
}

/// Get-or-init the shared engine handle for this test binary.
///
/// Panics (fails the test) if no engine is reachable — an e2e test without an
/// engine tests nothing, so it must not pass silently.
pub async fn get_or_init() -> Arc<IIIClient> {
    ENGINE
        .get_or_init(|| async {
            try_connect_raw().await.unwrap_or_else(|| {
                panic!(
                    "e2e requires a running iii engine at {} — start `iii` or set \
                     III_ENGINE_WS_URL to point at one",
                    ws_url()
                )
            })
        })
        .await
        .clone()
}
