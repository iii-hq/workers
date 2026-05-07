//! Dual-write integration tests for [`turn_orchestrator::persistence::save_messages`].
//!
//! Spawns an iii engine + the `iii-session-tree` worker, then calls the
//! turn-orchestrator persistence module as an in-process Rust API and
//! asserts both `state::*` and `session-tree::*` agree on the transcript.
//!
//! Tests skip gracefully when the `iii` binary or the prebuilt
//! `iii-session-tree` worker binary is missing (CI without iii).
//
// TODO(phase-a-followup): D-4 from spec ("session-tree forced to fail, state::*
// completes, warn logged, no panic") deferred — needs a fault-injection harness
// that swaps the iii-session-tree binary with one that returns Err on append.
// The structural guarantee (no `?`, no `.unwrap()` on session-tree calls in
// `mirror_messages_to_session_tree`) is enforced by code review for now.

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

#[tokio::test]
#[serial]
async fn delta_append_only_does_not_duplicate_existing_messages() {
    let Some(harness) = Harness::boot().await else {
        return;
    };
    let sid = format!("delta-test-{}", common::nonce());

    // First save: 1 msg
    let msgs1 = vec![user_msg("1", 1)];
    turn_orchestrator::persistence::save_messages(&harness.iii, &sid, &msgs1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Second save: 3 msgs total (2 new beyond what was already mirrored)
    let msgs2 = vec![user_msg("1", 1), user_msg("2", 2), user_msg("3", 3)];
    turn_orchestrator::persistence::save_messages(&harness.iii, &sid, &msgs2).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let tree_msgs = trigger(
        &harness.iii,
        "session-tree::messages",
        json!({ "session_id": sid }),
    )
    .await;

    let arr = tree_msgs["messages"].as_array().unwrap_or_else(|| {
        panic!("session-tree::messages should return an array; got {tree_msgs}")
    });
    assert_eq!(
        arr.len(),
        3,
        "delta should have appended 2 new, total 3 (not 4) — got {}",
        arr.len()
    );

    harness.iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn first_save_creates_session_tree_session_lazily() {
    let Some(harness) = Harness::boot().await else {
        return;
    };
    let sid = format!("lazy-create-{}", common::nonce());

    // No prior session-tree::create / ensure call from outside.
    let msgs = vec![user_msg("first", 1)];
    turn_orchestrator::persistence::save_messages(&harness.iii, &sid, &msgs).await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let list_res = trigger(&harness.iii, "session-tree::list", json!({})).await;
    let sessions = list_res["sessions"].as_array().unwrap_or_else(|| {
        panic!("session-tree::list should return a sessions array; got {list_res}")
    });
    assert!(
        sessions
            .iter()
            .any(|s| s["session_id"].as_str() == Some(&sid)),
        "session-tree::list should include the lazily-created session; got {list_res}"
    );

    harness.iii.shutdown_async().await;
}
