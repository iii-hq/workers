//! Dual-write integration tests for [`turn_orchestrator::persistence::save_messages`].
//!
//! Spawns an iii engine + the `iii-session-tree` worker, then calls the
//! turn-orchestrator persistence module as an in-process Rust API and
//! asserts both `state::*` and `session-tree::*` agree on the transcript.
//!
//! Tests skip gracefully when the `iii` binary or the prebuilt
//! `iii-session-tree` worker binary is missing (CI without iii).

#[path = "common/mod.rs"]
mod common;

use harness_types::{AgentMessage, ContentBlock, TextContent, UserMessage};
use serde_json::{json, Value};
use serial_test::serial;
use std::time::Duration;
use tokio::time::timeout;

use common::Harness;

fn user_msg(text: &str, ts: i64) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: vec![ContentBlock::Text(TextContent { text: text.into() })],
        timestamp: ts,
    })
}

async fn trigger(client: &iii_sdk::III, function_id: &str, payload: Value) -> Value {
    timeout(
        Duration::from_secs(10),
        client.trigger(iii_sdk::TriggerRequest {
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

#[tokio::test]
#[serial]
async fn happy_path_state_and_session_tree_match_after_save() {
    let Some(harness) = Harness::boot().await else {
        return;
    };
    let sid = format!("dual-write-happy-{}", common::nonce());

    let messages = vec![user_msg("hi", 1)];

    turn_orchestrator::persistence::save_messages(&harness.iii, &sid, &messages).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // state::* should hold the array verbatim.
    let state_msgs = trigger(
        &harness.iii,
        "state::get",
        json!({ "scope": "agent", "key": format!("session/{sid}/messages") }),
    )
    .await;
    let arr = state_msgs
        .as_array()
        .unwrap_or_else(|| panic!("state::get should return an array; got {state_msgs}"));
    assert_eq!(
        arr.len(),
        1,
        "state should have 1 message; got {state_msgs}"
    );

    // session-tree should have mirrored the message.
    let tree_msgs = trigger(
        &harness.iii,
        "session-tree::messages",
        json!({ "session_id": sid }),
    )
    .await;
    let mirrored = tree_msgs["messages"].as_array().unwrap_or_else(|| {
        panic!("session-tree::messages should return an array; got {tree_msgs}")
    });
    assert_eq!(
        mirrored.len(),
        1,
        "session-tree should have 1 mirrored message; got {tree_msgs}"
    );
    assert!(
        mirrored[0].get("entry_id").is_some(),
        "mirrored row should carry entry_id; got {tree_msgs}"
    );

    harness.iii.shutdown_async().await;
}
