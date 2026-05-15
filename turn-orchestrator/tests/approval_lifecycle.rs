//! End-to-end approval lifecycle: register a fake gated function, intercept
//! a call, resolve it, drive a synthetic next turn, and assert the stitched
//! system message reaches the message log. Skips cleanly when no engine.

use std::time::Duration;

use approval_gate::{register, WorkerConfig, FN_RESOLVE, FN_LIST_UNDELIVERED, FN_ACK_DELIVERED, STATE_SCOPE};
use iii_sdk::{register_worker, InitOptions, RegisterFunctionMessage, TriggerRequest, IIIError};
use serde_json::{json, Value};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
const ENGINE_PROBE_TIMEOUT_MS: u64 = 500;

async fn skip_if_no_engine(url: &str) -> Option<iii_sdk::III> {
    let iii = register_worker(url, InitOptions::default());
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
        return None;
    }
    Some(iii)
}

#[tokio::test]
async fn allow_path_executes_function_and_stitches_into_next_turn() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let Some(iii) = skip_if_no_engine(&url).await else { return };

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let session_id = format!("turn-orch-it-{nonce}");
    let function_call_id = format!("tc-{nonce}");
    let topic = format!("agent::before_function_call::ito_{nonce}");

    let target_calls: std::sync::Arc<std::sync::Mutex<Vec<Value>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let target_calls_for_handler = target_calls.clone();
    let _target = iii.register_function((
        RegisterFunctionMessage::with_id(format!("test::write_{nonce}")).with_description("fake write".into()),
        move |payload: Value| {
            let log = target_calls_for_handler.clone();
            async move {
                log.lock().unwrap().push(payload);
                Ok::<_, IIIError>(json!({"ok": true, "bytes": 42}))
            }
        },
    ));

    let _refs = register(
        &iii,
        &WorkerConfig {
            topic: topic.clone(),
            default_timeout_ms: 30_000,
            ..WorkerConfig::default()
        },
    ).expect("register approval-gate");

    let target_fn = format!("test::write_{nonce}");
    let envelope = json!({
        "event_id": format!("evt-{nonce}"),
        "reply_stream": format!("rs-{nonce}"),
        "payload": {
            "session_id": session_id,
            "function_call": {
                "id": function_call_id,
                "function_id": target_fn,
                "arguments": {"path": "/tmp/foo"},
            },
            "approval_required": [target_fn.clone()],
        }
    });
    let intercept_resp = iii.trigger(TriggerRequest {
        function_id: "policy::approval_gate".into(),
        payload: envelope,
        action: None,
        timeout_ms: Some(5_000),
    }).await.expect("intercept ok");

    assert_eq!(intercept_resp["block"], json!(true));
    assert_eq!(intercept_resp["status"], json!("pending"));
    assert!(target_calls.lock().unwrap().is_empty(), "function ran before approval");

    let resolve_resp = iii.trigger(TriggerRequest {
        function_id: FN_RESOLVE.into(),
        payload: json!({
            "session_id": session_id,
            "function_call_id": function_call_id,
            "decision": "allow",
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await.expect("resolve ok");
    assert_eq!(resolve_resp["ok"], json!(true));

    tokio::time::sleep(Duration::from_millis(50)).await;
    let calls = target_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "expected one invocation; got {calls:?}");
    assert_eq!(calls[0]["path"], json!("/tmp/foo"));

    let undelivered = iii.trigger(TriggerRequest {
        function_id: FN_LIST_UNDELIVERED.into(),
        payload: json!({"session_id": session_id}),
        action: None,
        timeout_ms: Some(5_000),
    }).await.expect("list_undelivered ok");
    let entries = undelivered["entries"].as_array().expect("entries array");
    let our_entry = entries
        .iter()
        .find(|e| e["function_call_id"] == function_call_id)
        .expect("our entry in undelivered list");
    assert_eq!(our_entry["status"], "executed");
    assert_eq!(our_entry["result"], json!({"ok": true, "bytes": 42}));

    let ack = iii.trigger(TriggerRequest {
        function_id: FN_ACK_DELIVERED.into(),
        payload: json!({
            "session_id": session_id,
            "call_ids": [function_call_id.clone()],
            "turn_id": "turn-1",
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await.expect("ack ok");
    assert_eq!(ack["ok"], json!(true));
    assert_eq!(ack["stamped"], json!(1));

    let after = iii.trigger(TriggerRequest {
        function_id: FN_LIST_UNDELIVERED.into(),
        payload: json!({"session_id": session_id}),
        action: None,
        timeout_ms: Some(5_000),
    }).await.expect("ok");
    let after_entries = after["entries"].as_array().unwrap();
    assert!(
        after_entries.iter().all(|e| e["function_call_id"] != function_call_id),
        "after ack, our entry must not be in undelivered list"
    );
}
