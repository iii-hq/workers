//! Engine-backed test for approval-gate. Connects to an in-process /
//! local iii engine, registers the gate, fires a `before_function_call`
//! envelope on a per-test topic, verifies the pending record is visible,
//! posts `approval::resolve`, and asserts `approval::consume` returns the
//! resolved entry once.
//!
//! Skips cleanly when no engine is reachable so `cargo test` stays green
//! in CI without a running engine.

use approval_gate::{register, WorkerConfig, FN_CONSUME, FN_LIST_PENDING, FN_RESOLVE, STATE_SCOPE};
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
const ENGINE_PROBE_TIMEOUT_MS: u64 = 500;

#[tokio::test]
async fn pending_allow_resolves_and_consumes_once() {
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
    let topic = format!("agent::before_function_call::it_{nonce}");
    let session_id = format!("approval-it-{nonce}");
    let function_call_id = format!("tc-it-{nonce}");
    let event_id = format!("evt-it-{nonce}");
    let reply_stream = format!("rs-it-{nonce}");

    let _refs = register(
        &iii,
        &WorkerConfig {
            topic: topic.clone(),
            default_timeout_ms: 5_000,
            ..WorkerConfig::default()
        },
    )
    .expect("register approval-gate");

    let envelope = json!({
        "event_id": event_id,
        "reply_stream": reply_stream,
        "payload": {
            "session_id": session_id,
            "function_call": {
                "id": function_call_id,
                "function_id": "shell::filesystem::write",
                "arguments": {},
            },
            "approval_required": ["shell::filesystem::write"],
        }
    });

    let reply = iii
        .trigger(TriggerRequest {
            function_id: "policy::approval_gate".into(),
            payload: envelope,
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .expect("subscriber returned ok");
    assert_eq!(reply["block"], true, "subscriber reply: {reply}");
    assert_eq!(reply["status"], "pending", "subscriber reply: {reply}");
    assert_eq!(
        reply["subscriber"], "approval-gate",
        "subscriber reply: {reply}"
    );
    assert_eq!(reply["approval_gate"], true, "subscriber reply: {reply}");

    let key = format!("{session_id}/{function_call_id}");
    let stored = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": key }),
            action: None,
            timeout_ms: Some(1_000),
        })
        .await
        .expect("state::get pending record");
    assert_eq!(stored["status"], "pending", "stored record: {stored}");

    let pending = iii
        .trigger(TriggerRequest {
            function_id: FN_LIST_PENDING.into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("list pending trigger");
    let pending_items = pending["pending"].as_array().expect("pending array");
    assert_eq!(pending_items.len(), 1, "pending response: {pending}");
    assert_eq!(
        pending_items[0]["function_call_id"], function_call_id,
        "pending response: {pending}"
    );

    let resolve = iii
        .trigger(TriggerRequest {
            function_id: FN_RESOLVE.into(),
            payload: json!({
                "session_id": session_id,
                "function_call_id": function_call_id,
                "decision": "allow",
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("resolve trigger");
    assert_eq!(resolve["ok"], true, "resolve response: {resolve}");

    let consumed = iii
        .trigger(TriggerRequest {
            function_id: FN_CONSUME.into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("consume trigger");
    let entries = consumed["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 1, "consume response: {consumed}");
    assert_eq!(
        entries[0]["decision"], "allow",
        "consume response: {consumed}"
    );
    assert_eq!(
        entries[0]["function_call_id"], function_call_id,
        "consume response: {consumed}"
    );

    let consumed_again = iii
        .trigger(TriggerRequest {
            function_id: FN_CONSUME.into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("second consume trigger");
    assert_eq!(
        consumed_again["entries"].as_array().map(Vec::len),
        Some(0),
        "second consume response: {consumed_again}"
    );
}
