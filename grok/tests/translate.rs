//! Full per-turn translation: feed a scripted Grok streaming-json sequence
//! through the pure stepper and assert the grok::on-agent-event frame sequence +
//! accumulated turn state (session id, result, error). This is the
//! orchestration the stream loop runs, exercised without a live engine; the
//! last test pushes the turn through the owned feeds with a capturing sender.
//! Event shapes captured from Grok CLI 0.2.77.

use std::sync::Mutex;

use grok::agent_feed::{Feed, FeedPayload, Feeds};
use grok::grok::events_types::GrokEvent;
use grok::grok::translate::{step, TurnState};
use serde_json::{json, Value};

fn run(events: &[Value]) -> (TurnState, Vec<Value>) {
    let mut state = TurnState::new("grok-4.20-0309-non-reasoning".into());
    let mut frames = Vec::new();
    for ev in events {
        let parsed: GrokEvent = serde_json::from_value(ev.clone()).unwrap();
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
fn text_deltas_accumulate_and_end_emits_message_complete() {
    let (state, frames) = run(&[
        json!({ "type": "text", "data": "Hello" }),
        json!({ "type": "text", "data": " world" }),
        json!({ "type": "end", "stopReason": "EndTurn", "sessionId": "sess-1", "requestId": "req-1" }),
    ]);

    assert_eq!(types(&frames), vec!["message_complete"]);
    assert_eq!(frames[0]["message"]["content"][0]["text"], "Hello world");
    assert_eq!(
        frames[0]["message"]["model"],
        "grok-4.20-0309-non-reasoning"
    );
    assert_eq!(frames[0]["message"]["provider"], "grok");
    assert_eq!(frames[0]["message"]["stop_reason"], "end");

    assert_eq!(state.thread_id.as_deref(), Some("sess-1"));
    assert_eq!(state.result_text, "Hello world");
    assert!(!state.is_error);
    assert_eq!(state.stop_reason, "end");
}

#[test]
fn text_only_before_end_emits_no_frames() {
    let (state, frames) = run(&[json!({ "type": "text", "data": "partial" })]);
    assert!(frames.is_empty());
    assert_eq!(state.result_text, "partial");
    assert!(state.thread_id.is_none());
}

#[test]
fn error_event_sets_error_state_and_emits_no_frame() {
    let (state, frames) = run(&[
        json!({ "type": "text", "data": "ignored" }),
        json!({ "type": "error", "message": "model exploded" }),
    ]);
    assert!(frames.is_empty());
    assert!(state.is_error);
    assert_eq!(state.stop_reason, "error");
    assert_eq!(state.result_text, "model exploded");
}

#[test]
fn unknown_event_type_is_skipped() {
    let (state, frames) = run(&[
        json!({ "type": "tool_call", "name": "shell" }),
        json!({ "type": "text", "data": "ok" }),
        json!({ "type": "end", "stopReason": "EndTurn", "sessionId": "s2", "requestId": "r2" }),
    ]);
    assert_eq!(types(&frames), vec!["message_complete"]);
    assert_eq!(state.result_text, "ok");
    assert_eq!(state.thread_id.as_deref(), Some("s2"));
}

#[test]
fn thought_deltas_render_as_thinking_block_before_text() {
    let (state, frames) = run(&[
        json!({ "type": "thought", "data": "let me " }),
        json!({ "type": "thought", "data": "think" }),
        json!({ "type": "text", "data": "answer" }),
        json!({ "type": "end", "stopReason": "EndTurn", "sessionId": "s4", "requestId": "r4" }),
    ]);
    assert_eq!(types(&frames), vec!["message_complete"]);
    let content = &frames[0]["message"]["content"];
    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["text"], "let me think");
    assert_eq!(content[1]["type"], "text");
    assert_eq!(content[1]["text"], "answer");
    // the returned answer excludes the reasoning
    assert_eq!(state.result_text, "answer");
    assert_eq!(state.thought_text, "let me think");
}

#[test]
fn non_endturn_stop_reason_maps() {
    let (state, _frames) = run(&[
        json!({ "type": "end", "stopReason": "MaxTokens", "sessionId": "s3", "requestId": "r3" }),
    ]);
    assert_eq!(state.stop_reason, "max_tokens");
}

/// Emit one frame on `feed`, with only `observed` observed, capturing the
/// call's envelope.
async fn push(
    feed: &Feed,
    observed: &str,
    session: &str,
    event: Value,
    sink: &Mutex<Vec<FeedPayload>>,
) {
    feed.emit_with(
        session,
        || event,
        |session_id| session_id == observed,
        |payload| {
            sink.lock().unwrap().push(payload);
            std::future::ready(Ok::<Value, iii_sdk::Error>(Value::Null))
        },
    )
    .await;
}

#[tokio::test]
async fn turn_frames_flow_unchanged_through_the_owned_feeds() {
    // The stream loop's order: each raw line goes to grok::on-raw-event, then the
    // frames it translates to go to grok::on-agent-event.
    let script = vec![
        json!({ "type": "text", "data": "Hello" }),
        json!({ "type": "text", "data": " world" }),
        json!({ "type": "end", "stopReason": "EndTurn", "sessionId": "sess-1", "requestId": "req-1" }),
    ];
    let (_, expected_frames) = run(&script);

    let feeds = Feeds::new("epoch-turn");
    let raw_sink = Mutex::new(Vec::new());
    let agent_sink = Mutex::new(Vec::new());
    let mut translated = Vec::new();
    let mut state = TurnState::new("grok-4.20-0309-non-reasoning".into());
    for line in &script {
        push(&feeds.raw, "s1", "s1", line.clone(), &raw_sink).await;
        let parsed: GrokEvent = serde_json::from_value(line.clone()).unwrap();
        let frames = step(&mut state, parsed);
        translated.extend(frames.iter().cloned());
        for frame in frames {
            push(&feeds.agent, "s1", "s1", frame, &agent_sink).await;
        }
    }
    // A different session's frames never reach the s1 observer.
    push(
        &feeds.agent,
        "s1",
        "s2",
        json!({ "type": "agent_end" }),
        &agent_sink,
    )
    .await;

    let raw = raw_sink.into_inner().unwrap();
    let agent = agent_sink.into_inner().unwrap();
    let events = |sent: &[FeedPayload]| sent.iter().map(|p| p.event.clone()).collect::<Vec<_>>();
    let seqs = |sent: &[FeedPayload]| sent.iter().map(|p| p.seq).collect::<Vec<_>>();
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
    for payload in raw.iter().chain(agent.iter()) {
        assert_eq!(payload.session_id, "s1");
        assert_eq!(payload.source, "grok");
        assert_eq!(payload.epoch, "epoch-turn");
    }
}
