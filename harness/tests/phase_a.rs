//! Phase A end-to-end acceptance test:
//!   1. dual-write via [`turn_orchestrator::persistence::save_messages`]
//!   2. fork from the first message via `session-tree::fork`
//!   3. simulate drift: write a third message only to `state::*`
//!   4. `session-tree::reconcile` repairs the gap
//!
//! Skips gracefully when the `iii` binary or the prebuilt
//! `iii-session-tree` worker binary is missing.

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
async fn phase_a_dual_write_then_fork_then_reconcile() {
    let Some(harness) = Harness::boot().await else {
        return;
    };
    let sid = format!("phase-a-acceptance-{}", common::nonce());

    // 1. Dual-write produces tree contents with entry_ids.
    let messages = vec![user_msg("msg-1", 1), user_msg("msg-2", 2)];
    turn_orchestrator::persistence::save_messages(&harness.iii, &sid, &messages).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let tree_msgs = trigger(
        &harness.iii,
        "session-tree::messages",
        json!({ "session_id": sid }),
    )
    .await;
    let pairs = tree_msgs["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("session-tree::messages should return array; got {tree_msgs}"));
    assert_eq!(
        pairs.len(),
        2,
        "expected 2 mirrored messages; got {tree_msgs}"
    );
    let first_entry_id = pairs[0]["entry_id"]
        .as_str()
        .unwrap_or_else(|| panic!("first mirrored row should carry entry_id; got {tree_msgs}"))
        .to_string();

    // 2. Fork from the first message.
    let fork_res = trigger(
        &harness.iii,
        "session-tree::fork",
        json!({ "source_session_id": sid, "from_entry_id": first_entry_id }),
    )
    .await;
    let new_sid = fork_res["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("fork should return session_id; got {fork_res}"));
    assert_ne!(new_sid, sid, "fork should yield a new session id");

    // 3. Simulate drift: write a third message only to state::*.
    trigger(
        &harness.iii,
        "state::set",
        json!({
            "scope": "agent",
            "key": format!("session/{sid}/messages"),
            "value": [
                {"role": "user", "content": [{"type": "text", "text": "msg-1"}], "timestamp": 1},
                {"role": "user", "content": [{"type": "text", "text": "msg-2"}], "timestamp": 2},
                {"role": "user", "content": [{"type": "text", "text": "msg-3"}], "timestamp": 3}
            ]
        }),
    )
    .await;

    // 4. Reconcile — pass the state snapshot back to session-tree.
    let snapshot = trigger(
        &harness.iii,
        "state::get",
        json!({ "scope": "agent", "key": format!("session/{sid}/messages") }),
    )
    .await;

    let reconcile_res = trigger(
        &harness.iii,
        "session-tree::reconcile",
        json!({ "session_id": sid, "state_snapshot": snapshot }),
    )
    .await;
    assert_eq!(
        reconcile_res["repaired"], 1,
        "should have repaired 1 missing message; got {reconcile_res}"
    );
    assert_eq!(
        reconcile_res["tree_count_after"], 3,
        "tree should now have 3 messages; got {reconcile_res}"
    );

    harness.iii.shutdown_async().await;
}
