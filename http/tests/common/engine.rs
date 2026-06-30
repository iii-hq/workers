//! Connect-or-skip wrapper around the iii SDK.
//!
//! Ported from `iii-directory/tests/common/engine.rs`. One engine connection
//! per test binary process via `OnceCell` — the WebSocket handshake dwarfs
//! per-test overhead. When the engine is unreachable, `get_or_init` returns
//! `None` so each test can early-return (skip) rather than fail.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{InitOptions, IIIClient, register_worker};
use serde_json::json;
use tokio::sync::OnceCell;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Option<Arc<IIIClient>>> = OnceCell::const_new();

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

    eprintln!(
        "[skip] iii engine not reachable at {url}; \
         set III_ENGINE_WS_URL or start `iii` to enable engine-bound e2e tests"
    );
    iii.shutdown_async().await;
    None
}

/// Get-or-init the shared engine handle for this test binary.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async { try_connect_raw().await })
        .await
        .clone()
}
