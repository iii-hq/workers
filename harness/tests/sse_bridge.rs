//! Engine-backed smoke test for /bridge/events.
//!
//! Skipped when no engine is reachable. To run locally:
//!   harness/scripts/demo.sh engine && cargo test -p harness --test sse_bridge

use harness::register_with_iii;
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;
use std::time::Duration;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
const ENGINE_PROBE_TIMEOUT_MS: u64 = 500;

#[tokio::test]
async fn sse_endpoint_emits_frames_and_resumes_from_cursor() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = register_worker(&url, InitOptions::default());

    // Probe with a short state::get to confirm the engine is live.
    let probe = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": "agent", "key": "__sse_probe" }),
            action: None,
            timeout_ms: Some(ENGINE_PROBE_TIMEOUT_MS),
        })
        .await;
    if probe.is_err() {
        eprintln!("skipping: no engine at {url}");
        return;
    }

    let _refs = match register_with_iii(&iii).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipping: register_with_iii failed: {e}");
            return;
        }
    };

    let session_id = format!(
        "sse-it-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // Write three events, deterministic id order.
    for (i, ty) in ["agent_start", "turn_start", "turn_end"].iter().enumerate() {
        iii.trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
                "item_id": format!("{session_id}-{:08}", i),
                "data": { "type": ty, "id": format!("{session_id}-{:08}", i) },
            }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await
        .expect("write seed event");
    }

    // Hit the endpoint; expect three id: lines.
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "bridge::events".into(),
            payload: json!({
                "query_params": { "session_id": session_id },
                "headers": {},
            }),
            action: None,
            timeout_ms: Some(8_000),
        })
        .await
        .expect("call bridge::events");
    let body = resp["body"].as_str().unwrap_or_default().to_string();
    let id_lines = body.matches("id: ").count();
    assert!(
        id_lines >= 3,
        "expected >=3 frames, got {id_lines} body=\n{body}"
    );

    // Reconnect with Last-Event-ID of the second event; expect only the third frame.
    let cursor = format!("{session_id}-00000001");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let resp2 = iii
        .trigger(TriggerRequest {
            function_id: "bridge::events".into(),
            payload: json!({
                "query_params": { "session_id": session_id },
                "headers": { "Last-Event-ID": cursor },
            }),
            action: None,
            timeout_ms: Some(8_000),
        })
        .await
        .expect("call bridge::events with cursor");
    let body2 = resp2["body"].as_str().unwrap_or_default().to_string();
    assert!(
        body2.contains("turn_end"),
        "third event missing from replay: {body2}"
    );
    assert!(
        !body2.contains("agent_start"),
        "agent_start should not replay: {body2}"
    );
}
