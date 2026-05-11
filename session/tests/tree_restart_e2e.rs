//! E2E: create a session + append messages, kill session-tree worker, restart,
//! read messages back.
//!
//! Requires:
//! - `IIITEST_ENGINE_URL`: WebSocket URL of a running iii engine
//! - `IIITEST_WORKER_BIN`: absolute path to the iii-session binary
//!
//! Marked `#[ignore]` so cargo test default runs skip it. To execute:
//!     cargo build --release -p iii-session
//!     IIITEST_ENGINE_URL=ws://127.0.0.1:49134 \
//!       IIITEST_WORKER_BIN=$(pwd)/target/release/iii-session \
//!       cargo test -p iii-session --test restart_e2e -- --ignored
//!
//! Schema source (verified against session-tree/src/lib.rs `register_with_iii`):
//! - `session-tree::create` payload: `{ "display_name": str?, "cwd": str? }` →
//!   `{ "session_id": str }`
//! - `session-tree::append` payload: `{ "session_id": str, "parent_id": str?,
//!   "message": <AgentMessage> }` → `{ "entry_id": str }`
//! - `session-tree::messages` payload: `{ "session_id": str, "branch_leaf": str? }`
//!   → `{ "messages": [<AgentMessage>...] }`
//!
//! `AgentMessage` is `#[serde(tag = "role", rename_all = "snake_case")]`
//! (session-tree/crates/harness-types/src/agent_message.rs:10). `ContentBlock`
//! is `#[serde(tag = "type", rename_all = "camelCase")]`
//! (session-tree/crates/harness-types/src/content.rs:7). A user message with
//! a single text block looks like:
//!     { "role": "user",
//!       "content": [{ "type": "text", "text": "..." }],
//!       "timestamp": <i64> }

use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn spawn_worker(engine_url: &str, bin: &str) -> Child {
    let config_path = format!("{}/config.yaml", env!("CARGO_MANIFEST_DIR"));
    Command::new(bin)
        .args(["--url", engine_url, "--config", &config_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn iii-session")
}

async fn wait_for_ready() {
    tokio::time::sleep(Duration::from_secs(2)).await;
}

fn user_message(text: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "user",
        "content": [{ "type": "text", "text": text }],
        "timestamp": 0,
    })
}

#[tokio::test]
#[ignore = "requires live engine + IIITEST_WORKER_BIN; opt in via --ignored"]
async fn session_messages_survive_worker_restart() {
    let url = std::env::var("IIITEST_ENGINE_URL").expect("set IIITEST_ENGINE_URL");
    let bin = std::env::var("IIITEST_WORKER_BIN").expect("set IIITEST_WORKER_BIN");

    let mut worker = spawn_worker(&url, &bin);
    wait_for_ready().await;

    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());

    // 1. Create a session.
    let create_resp = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: "session-tree::create".into(),
            payload: serde_json::json!({
                "display_name": "e2e-restart",
            }),
            action: None,
            timeout_ms: None,
        })
        .await
        .expect("session-tree::create failed");
    let sid = create_resp
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .expect("session-tree::create response missing session_id")
        .to_string();

    // 2. Append two messages. Use distinctive text so we can grep for them
    //    in the post-restart response.
    let marker_first = format!("e2e-first-{}", std::process::id());
    let marker_second = format!("e2e-second-{}", std::process::id());
    let mut last_entry_id: Option<String> = None;
    for text in [&marker_first, &marker_second] {
        let mut payload = serde_json::json!({
            "session_id": &sid,
            "message": user_message(text),
        });
        if let Some(parent) = &last_entry_id {
            payload["parent_id"] = serde_json::Value::String(parent.clone());
        }
        let resp = iii
            .trigger(iii_sdk::TriggerRequest {
                function_id: "session-tree::append".into(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
            .expect("session-tree::append failed");
        last_entry_id = resp
            .get("entry_id")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
    }

    // 3. Kill + restart worker.
    worker.kill().expect("kill worker");
    worker.wait().expect("wait for worker exit");
    let mut worker = spawn_worker(&url, &bin);
    wait_for_ready().await;

    // 4. Load messages; assert both survived.
    let load_resp = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: "session-tree::messages".into(),
            payload: serde_json::json!({ "session_id": &sid }),
            action: None,
            timeout_ms: None,
        })
        .await
        .expect("session-tree::messages failed");

    // The function returns `{ "messages": [<AgentMessage>...] }`. Look for our
    // marker text inside the serialized response.
    let body = serde_json::to_string(&load_resp).expect("serialize load response");
    assert!(
        body.contains(&marker_first),
        "missing first marker {marker_first} in response body: {body}",
    );
    assert!(
        body.contains(&marker_second),
        "missing second marker {marker_second} in response body: {body}",
    );

    worker.kill().ok();
    let _ = worker.wait();
}
