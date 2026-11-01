//! Helpers the live-engine tests share. Each test connects its own workers
//! to the engine at `III_ENGINE_WS_URL` (default `ws://127.0.0.1:49134`) and
//! runs in a namespace of its own; without an engine the tests skip, or fail
//! when `III_E2E_REQUIRE` is set.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{CallRequest, IIIClient, InitOptions, register_worker};
use serde_json::json;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

/// A worker named `name` in `namespace`.
pub fn client(namespace: &str, name: &str) -> Arc<IIIClient> {
    Arc::new(register_worker(
        &ws_url(),
        InitOptions {
            namespace: Some(namespace.to_string()),
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ))
}

/// Waits until `iii` reaches the engine; false after about 5 s.
pub async fn connected(iii: &IIIClient) -> bool {
    for _ in 0..20 {
        let probe = iii
            .call(
                CallRequest {
                    function_id: "engine::workers::list".to_string(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(800),
                }
                .namespace("default"),
            )
            .await;
        if probe.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

/// True when an engine answers; otherwise skips, or panics under
/// `III_E2E_REQUIRE`. `case` keeps concurrent probes apart.
pub async fn engine_or_skip(case: &str) -> bool {
    let probe = client(&format!("cui-{case}-probe-{}", std::process::id()), "probe");
    let up = connected(&probe).await;
    probe.shutdown_async().await;
    if up {
        return true;
    }
    if std::env::var("III_E2E_REQUIRE").is_ok() {
        panic!(
            "e2e requires a running iii engine at {} - set III_ENGINE_WS_URL",
            ws_url()
        );
    }
    eprintln!(
        "[skip] no iii engine reachable at {} - skipping (set III_E2E_REQUIRE=1 to fail instead)",
        ws_url()
    );
    false
}
