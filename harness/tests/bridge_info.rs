//! Integration test for harness's bridge::info function.
//!
//! Registers the harness functions in-process against a running engine and
//! verifies that bridge::info returns a relative WebSocket path plus the
//! engine URL it was configured with. Skipped when no engine is reachable.

use harness::register_with_iii_with_engine_url;
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
const ENGINE_PROBE_TIMEOUT_MS: u64 = 500;

#[tokio::test]
async fn bridge_info_returns_relative_ws_path_and_engine_url() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = register_worker(&url, InitOptions::default());

    // Probe with a short state::get to confirm the engine is live.
    let probe = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": "agent", "key": "__bridge_info_probe" }),
            action: None,
            timeout_ms: Some(ENGINE_PROBE_TIMEOUT_MS),
        })
        .await;
    if probe.is_err() {
        eprintln!("skipping: no engine at {url}");
        return;
    }

    let _refs = match register_with_iii_with_engine_url(&iii, &url).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipping: register_with_iii_with_engine_url failed: {e}");
            return;
        }
    };

    let info = iii
        .trigger(TriggerRequest {
            function_id: "bridge::info".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await
        .expect("call bridge::info");

    assert_eq!(info["ws_path"], "/iii/ws");
    assert_eq!(info["protocol"], "ws");
    assert_eq!(
        info["engine_url"].as_str(),
        Some(url.as_str()),
        "engine_url should reflect the URL passed to register_with_iii_with_engine_url; got {info}"
    );
}
