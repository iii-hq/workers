//! Smoke + bus-level integration tests for `session-tree::list` and
//! `session-tree::messages`.
//!
//! The bus tests boot a real `iii` engine and the `iii-session-tree` worker
//! against a transient in-memory store, then drive each function through
//! `iii.trigger`. They skip silently when the `iii` CLI is not on PATH so
//! `cargo test` stays green in plain dev environments.
//!
//! To run them locally:
//!   cargo build --bin iii-session-tree
//!   cargo test --test integration

mod common;

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

#[test]
fn in_memory_store_constructs() {
    let _store = session_tree::InMemoryStore::default();
}

struct Harness {
    iii: Child,
    worker: Child,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
    }
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = common::session_tree_executable();
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/tests/integration_memory.yaml");

    let worker = Command::new(worker_bin)
        .args(["--url", ENGINE_WS, "--config", &config_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(1500)).await;

    Some(Harness { iii, worker })
}

async fn trigger(
    client: &iii_sdk::III,
    function_id: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("{function_id} timed out"))
    .unwrap_or_else(|e| panic!("{function_id} failed: {e}"))
}

fn user_message(text: &str, ts: i64) -> serde_json::Value {
    json!({
        "role": "user",
        "content": [{ "type": "text", "text": text }],
        "timestamp": ts,
    })
}

#[tokio::test]
async fn bus_list_returns_envelope_with_total_and_sessions() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    // Use unique display_names so we can find our two created sessions even
    // when other parallel tests in this binary share the same engine.
    let tag_a = format!("list-test-a-{}", uuid_like_nonce());
    let tag_b = format!("list-test-b-{}", uuid_like_nonce());
    let create_a = trigger(
        &client,
        "session-tree::create",
        json!({ "display_name": tag_a }),
    )
    .await;
    let create_b = trigger(
        &client,
        "session-tree::create",
        json!({ "display_name": tag_b }),
    )
    .await;
    let id_a = create_a["session_id"].as_str().expect("session_id a");
    let id_b = create_b["session_id"].as_str().expect("session_id b");

    let result = trigger(&client, "session-tree::list", json!({})).await;

    let total = result["total"]
        .as_u64()
        .unwrap_or_else(|| panic!("total should be a u64; got {result}"));
    let sessions = result["sessions"]
        .as_array()
        .unwrap_or_else(|| panic!("sessions should be an array; got {result}"));
    assert!(
        total >= 2,
        "total should reflect at least the two created sessions; got {result}",
    );
    assert_eq!(
        sessions.len() as u64,
        total,
        "sessions array length should equal total when no pagination is requested; got {result}",
    );
    let saw_a = sessions
        .iter()
        .any(|s| s["session_id"].as_str() == Some(id_a));
    let saw_b = sessions
        .iter()
        .any(|s| s["session_id"].as_str() == Some(id_b));
    assert!(saw_a, "list should include session a; got {result}");
    assert!(saw_b, "list should include session b; got {result}");

    client.shutdown_async().await;
}

fn uuid_like_nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}")
}

#[tokio::test]
async fn bus_messages_returns_entry_ids() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let create_resp = trigger(
        &client,
        "session-tree::create",
        json!({ "display_name": "msg-test" }),
    )
    .await;
    let session_id = create_resp["session_id"]
        .as_str()
        .expect("session_id in create response")
        .to_string();

    let append_resp = trigger(
        &client,
        "session-tree::append",
        json!({
            "session_id": session_id,
            "message": user_message("hello bus", 1),
        }),
    )
    .await;
    let appended_entry_id = append_resp["entry_id"]
        .as_str()
        .expect("entry_id in append response")
        .to_string();

    let messages_resp = trigger(
        &client,
        "session-tree::messages",
        json!({ "session_id": session_id }),
    )
    .await;

    let messages = messages_resp["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("messages should be an array; got {messages_resp}"));
    assert_eq!(
        messages.len(),
        1,
        "messages should have one row; got {messages_resp}"
    );

    let row = &messages[0];
    let entry_id = row["entry_id"]
        .as_str()
        .unwrap_or_else(|| panic!("row.entry_id should be a string; got {row}"));
    assert_eq!(
        entry_id, appended_entry_id,
        "entry_id should match the appended id"
    );
    assert!(
        row.get("message").is_some(),
        "row should carry a message field; got {row}"
    );

    client.shutdown_async().await;
}
