//! Full per-turn translation: feed a scripted Codex event sequence through the
//! pure stepper and assert the codex::agent-event frame sequence + accumulated
//! turn state (thread id, usage, result, error). This is the orchestration the
//! stream loop runs, exercised without a live engine; the last test pushes the
//! turn through the owned feeds with a capturing sender.

use std::sync::Mutex;

use codex::agent_feed::{Delivery, Feed, Feeds};
use codex::codex::events_types::ThreadEvent;
use codex::codex::translate::{step, TurnState};
use iii_sdk::trigger::TriggerConfig;
use serde_json::{json, Value};

fn run(events: &[Value]) -> (TurnState, Vec<Value>) {
    let mut state = TurnState::new("gpt-5.2-codex".into());
    let mut frames = Vec::new();
    for ev in events {
        let parsed: ThreadEvent = serde_json::from_value(ev.clone()).unwrap();
        frames.extend(step(&mut state, parsed));
    }
    (state, frames)
}

fn types(frames: &[Value]) -> Vec<String> {
    frames
        .iter()
        .map(|f| f["type"].as_str().unwrap_or("?").to_string())
        .collect()
}

#[test]
fn full_command_turn_produces_ordered_frames_and_state() {
    let (state, frames) = run(&[
        json!({ "type": "thread.started", "thread_id": "th-1" }),
        json!({ "type": "turn.started" }),
        json!({ "type": "item.started", "item": { "id": "i1", "type": "command_execution", "command": "ls", "aggregated_output": "", "status": "in_progress" } }),
        json!({ "type": "item.completed", "item": { "id": "i1", "type": "command_execution", "command": "ls", "aggregated_output": "files", "exit_code": 0, "status": "completed" } }),
        json!({ "type": "item.completed", "item": { "id": "i2", "type": "agent_message", "text": "done" } }),
        json!({ "type": "turn.completed", "usage": { "input_tokens": 5, "cached_input_tokens": 3, "output_tokens": 2, "reasoning_output_tokens": 1 } }),
    ]);

    assert_eq!(
        types(&frames),
        vec![
            "function_execution_start",
            "function_execution_end",
            "message_complete",
        ]
    );
    // exec frames carry the mapped function id + args/result
    assert_eq!(frames[0]["function_id"], "codex::shell");
    assert_eq!(frames[0]["args"]["command"], "ls");
    assert_eq!(frames[1]["is_error"], false);
    assert_eq!(frames[1]["result"]["content"][0]["text"], "files");
    // message_complete stamps the model
    assert_eq!(frames[2]["message"]["model"], "gpt-5.2-codex");
    assert_eq!(frames[2]["message"]["provider"], "codex");

    assert_eq!(state.thread_id.as_deref(), Some("th-1"));
    assert_eq!(state.result_text, "done");
    assert!(!state.is_error);
    let u = state.usage.unwrap();
    assert_eq!(u.input_tokens, 5);
    assert_eq!(u.cache_read_tokens, 3);
    assert_eq!(u.reasoning_tokens, 1);
}

#[test]
fn started_then_completed_emits_single_start() {
    // item.started already emitted the start; item.completed must not repeat it.
    let (_s, frames) = run(&[
        json!({ "type": "item.started", "item": { "id": "i1", "type": "command_execution", "command": "x", "status": "in_progress" } }),
        json!({ "type": "item.completed", "item": { "id": "i1", "type": "command_execution", "command": "x", "aggregated_output": "ok", "exit_code": 0, "status": "completed" } }),
    ]);
    assert_eq!(
        types(&frames),
        vec!["function_execution_start", "function_execution_end"]
    );
}

#[test]
fn completed_without_prior_start_synthesizes_start() {
    // a completed exec item with no preceding started event still gets a start.
    let (_s, frames) = run(&[
        json!({ "type": "item.completed", "item": { "id": "i9", "type": "command_execution", "command": "x", "aggregated_output": "", "exit_code": 1, "status": "failed" } }),
    ]);
    assert_eq!(
        types(&frames),
        vec!["function_execution_start", "function_execution_end"]
    );
    assert_eq!(frames[1]["is_error"], true);
}

#[test]
fn reasoning_maps_to_thinking_message() {
    let (_s, frames) = run(&[
        json!({ "type": "item.completed", "item": { "id": "r1", "type": "reasoning", "text": "hmm" } }),
    ]);
    assert_eq!(types(&frames), vec!["message_complete"]);
    assert_eq!(frames[0]["message"]["content"][0]["type"], "thinking");
}

#[test]
fn turn_failed_sets_error_state_and_emits_no_frame() {
    let (state, frames) = run(&[
        json!({ "type": "thread.started", "thread_id": "th-1" }),
        json!({ "type": "turn.failed", "error": { "message": "model exploded" } }),
    ]);
    assert!(frames.is_empty());
    assert!(state.is_error);
    assert_eq!(state.stop_reason, "error");
    assert_eq!(state.result_text, "model exploded");
}

#[test]
fn mcp_tool_call_maps_to_server_tool_id() {
    let (_s, frames) = run(&[
        json!({ "type": "item.completed", "item": { "id": "m1", "type": "mcp_tool_call", "server": "github", "tool": "create_issue", "arguments": { "title": "x" }, "status": "completed" } }),
    ]);
    assert_eq!(
        frames.last().unwrap()["function_id"],
        "github::create_issue"
    );
}

/// Bind one consumer on each feed for `session` and capture every delivery.
fn bound_feeds(session: &str) -> Feeds {
    let feeds = Feeds::new("epoch-turn");
    for feed in [&feeds.agent, &feeds.raw] {
        feed.bind(&TriggerConfig {
            id: format!("{}-consumer", feed.id()),
            function_id: "acp::__on_event::c1".into(),
            config: json!({ "session_id": session }),
            metadata: None,
            namespace: None,
        })
        .unwrap();
    }
    feeds
}

async fn push(feed: &Feed, session: &str, event: Value, sink: &Mutex<Vec<Delivery>>) {
    feed.emit_with(session, event, |delivery| {
        sink.lock().unwrap().push(delivery);
        std::future::ready(Ok::<Value, iii_sdk::Error>(Value::Null))
    })
    .await;
}

#[tokio::test]
async fn turn_frames_flow_unchanged_through_the_owned_feeds() {
    // The stream loop's order: each raw line goes to codex::raw-event, then the
    // frames it translates to go to codex::agent-event.
    let script = vec![
        json!({ "type": "thread.started", "thread_id": "th-1" }),
        json!({ "type": "turn.started" }),
        json!({ "type": "item.completed", "item": { "id": "i1", "type": "command_execution", "command": "ls", "aggregated_output": "files", "exit_code": 0, "status": "completed" } }),
        json!({ "type": "item.completed", "item": { "id": "i2", "type": "agent_message", "text": "done" } }),
        json!({ "type": "turn.completed", "usage": { "input_tokens": 5, "cached_input_tokens": 0, "output_tokens": 2 } }),
    ];
    let (_, expected_frames) = run(&script);

    let feeds = bound_feeds("s1");
    let raw_sink = Mutex::new(Vec::new());
    let agent_sink = Mutex::new(Vec::new());
    let mut translated = Vec::new();
    let mut state = TurnState::new("gpt-5.2-codex".into());
    for line in &script {
        push(&feeds.raw, "s1", line.clone(), &raw_sink).await;
        let parsed: ThreadEvent = serde_json::from_value(line.clone()).unwrap();
        let frames = step(&mut state, parsed);
        translated.extend(frames.iter().cloned());
        for frame in frames {
            push(&feeds.agent, "s1", frame, &agent_sink).await;
        }
    }
    // A different session's frames never reach the s1 consumer.
    push(
        &feeds.agent,
        "s2",
        json!({ "type": "agent_end" }),
        &agent_sink,
    )
    .await;

    let raw = raw_sink.into_inner().unwrap();
    let agent = agent_sink.into_inner().unwrap();
    let events = |sent: &[Delivery]| {
        sent.iter()
            .map(|d| d.payload["event"].clone())
            .collect::<Vec<_>>()
    };
    let seqs = |sent: &[Delivery]| {
        sent.iter()
            .map(|d| d.payload["seq"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(events(&raw), script, "raw frames are verbatim");
    assert_eq!(
        events(&agent),
        translated,
        "AgentEvent frames are unchanged"
    );
    assert_eq!(
        types(&events(&agent)),
        types(&expected_frames),
        "AgentEvent sequence unchanged"
    );
    assert_eq!(seqs(&raw), (0..script.len() as u64).collect::<Vec<_>>());
    assert_eq!(
        seqs(&agent),
        (0..expected_frames.len() as u64).collect::<Vec<_>>()
    );
    for delivery in raw.iter().chain(agent.iter()) {
        assert_eq!(delivery.function_id, "acp::__on_event::c1");
        assert_eq!(delivery.payload["session_id"], "s1");
        assert_eq!(delivery.payload["source"], "codex");
        assert_eq!(delivery.payload["epoch"], "epoch-turn");
    }
}
