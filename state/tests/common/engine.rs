#![allow(dead_code)]
//! Connect-or-skip wrapper around the iii SDK.
//!
//! These are e2e tests: they only make sense with a running engine. By
//! default, when the engine is unreachable, `get_or_init`/`connect_fresh`
//! return `None` and the caller skips the test — this keeps CI (which runs
//! with no engine) and casual local runs green. Set `III_E2E_REQUIRE` to any
//! value to fail loudly instead (for intentional e2e runs where a missing
//! engine should be treated as a bug).
//!
//! One engine connection per test binary process via `OnceCell` — the
//! WebSocket handshake dwarfs per-test overhead.
//!
//! Point the tests at an engine with `III_ENGINE_WS_URL` (default
//! `ws://127.0.0.1:49134`).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, InitOptions, register_worker};
use serde_json::json;
use tokio::sync::OnceCell;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Option<Arc<IIIClient>>> = OnceCell::const_new();

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

/// Turn a failed connection attempt into either a panic (when
/// `III_E2E_REQUIRE` is set — an intentional e2e run that must fail loudly
/// on a missing engine) or a skip (the default — logs and returns `None` so
/// the caller can bail out of the test without failing it).
fn require_or_skip(connected: Option<Arc<IIIClient>>) -> Option<Arc<IIIClient>> {
    if connected.is_some() {
        return connected;
    }

    if std::env::var("III_E2E_REQUIRE").is_ok() {
        panic!(
            "e2e requires a running iii engine at {} — start `iii` or set \
             III_ENGINE_WS_URL to point at one",
            ws_url()
        );
    }

    eprintln!(
        "[skip] no iii engine reachable at {} — skipping e2e test (set III_E2E_REQUIRE=1 to \
         fail instead)",
        ws_url()
    );
    None
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
/// binary also registers (e.g. the state worker's fixed `state::*` function
/// ids, re-registered on every `boot::start` call): registering the same id
/// twice on one client panics, so such a test needs its own client. Returns
/// `None` (the caller should skip the test) if no engine is reachable, unless
/// `III_E2E_REQUIRE` is set, in which case it panics. The caller should
/// `shutdown_async()` the returned client when done.
pub async fn connect_fresh() -> Option<Arc<IIIClient>> {
    require_or_skip(try_connect_raw().await)
}

/// Get-or-init the shared engine handle for this test binary.
///
/// Returns `None` (the caller should skip the test) if no engine is
/// reachable, unless `III_E2E_REQUIRE` is set, in which case it panics.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async { require_or_skip(try_connect_raw().await) })
        .await
        .clone()
}
