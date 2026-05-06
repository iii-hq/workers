//! Engine-backed test for approval-gate. Connects to an in-process /
//! local iii engine, registers the gate, fires a `before_tool_call`
//! envelope on a per-test topic, posts `approval::resolve`, and asserts
//! the subscriber unblocks under 1 s.
//!
//! Skips cleanly when no engine is reachable so `cargo test` stays green
//! in CI without a running engine.

use std::time::Duration;

use approval_gate::{register, Config, FN_RESOLVE, STATE_SCOPE};
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
const ENGINE_PROBE_TIMEOUT_MS: u64 = 500;

#[tokio::test]
async fn round_trip_allow_unblocks_under_one_second() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = register_worker(&url, InitOptions::default());

    // Probe the engine with a short-timeout state::get; if it errors,
    // assume no engine is running locally and skip cleanly.
    let probe = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": "__probe__" }),
            action: None,
            timeout_ms: Some(ENGINE_PROBE_TIMEOUT_MS),
        })
        .await;
    if probe.is_err() {
        eprintln!("skipping: no engine at {url}");
        return;
    }

    // Use a unique topic per run so concurrent test runs don't collide,
    // and so we don't race the production approval-gate worker if one is
    // already subscribed to the default topic.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let topic = format!("agent::before_tool_call::it_{nonce}");
    let session_id = format!("approval-it-{nonce}");
    let tool_call_id = format!("tc-it-{nonce}");
    let event_id = format!("evt-it-{nonce}");
    let reply_stream = format!("rs-it-{nonce}");

    let _refs = register(
        &iii,
        Config {
            topic: topic.clone(),
            timeout_ms: 5_000,
        },
    )
    .expect("register approval-gate");

    let envelope = json!({
        "event_id": event_id,
        "reply_stream": reply_stream,
        "payload": {
            "session_id": session_id,
            "tool_call": {
                "id": tool_call_id,
                "name": "shell::filesystem::write",
                "arguments": {},
            },
            "approval_required": ["shell::filesystem::write"],
        }
    });

    // Drive the subscriber by directly triggering its function id.
    let subscriber_call = tokio::spawn({
        let iii = iii.clone();
        async move {
            iii.trigger(TriggerRequest {
                function_id: "policy::approval_gate".into(),
                payload: envelope,
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
        }
    });

    // Wait for the gate to write the pending record before we resolve.
    let key = format!("{session_id}/{tool_call_id}");
    let mut tries = 0;
    loop {
        let v = iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": STATE_SCOPE, "key": key }),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await
            .unwrap_or(json!(null));
        if v.get("status").and_then(|s| s.as_str()) == Some("pending") {
            break;
        }
        tries += 1;
        assert!(tries < 40, "pending entry never appeared (key={key})");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Post the allow decision and time the unblock.
    let started = std::time::Instant::now();
    let resolve = iii
        .trigger(TriggerRequest {
            function_id: FN_RESOLVE.into(),
            payload: json!({
                "session_id": session_id,
                "tool_call_id": tool_call_id,
                "decision": "allow",
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("resolve trigger");
    assert_eq!(resolve["ok"], true, "resolve response: {resolve}");

    let reply = subscriber_call
        .await
        .expect("subscriber task join")
        .expect("subscriber returned ok");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(1_000),
        "allow round-trip took {elapsed:?}, expected <1s",
    );
    assert_eq!(reply["block"], false, "subscriber reply: {reply}");
}
