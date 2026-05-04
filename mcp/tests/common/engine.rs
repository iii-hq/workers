//! Connect-or-skip wrapper around the iii SDK.
//!
//! Lifted from skills/tests/common/engine.rs. One engine connection per
//! test binary process via OnceCell — the cost of each WebSocket
//! handshake dwarfs cucumber's per-scenario overhead.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{register_worker, IIIError, InitOptions, TriggerRequest, III};
use serde_json::json;
use tokio::sync::OnceCell;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Option<Arc<III>>> = OnceCell::const_new();

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

pub async fn try_connect_raw() -> Option<Arc<III>> {
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
            Err(IIIError::NotConnected) => continue,
            Err(_) => continue,
        }
    }

    eprintln!(
        "[skip] iii engine not reachable at {url}; \
         set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
    );
    iii.shutdown_async().await;
    None
}

/// Get-or-init the shared engine handle. Registers the production
/// `mcp::handler` plus the skills::* / prompts::* stubs the dispatcher
/// delegates into, so BDD scenarios drive the same code paths the
/// production binary would.
pub async fn get_or_init() -> Option<Arc<III>> {
    ENGINE
        .get_or_init(|| async {
            let iii = try_connect_raw().await?;
            crate::common::workers::register_all(&iii).await.ok()?;
            Some(iii)
        })
        .await
        .clone()
}
