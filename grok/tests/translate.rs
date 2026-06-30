//! Full per-turn translation: feed a scripted Grok streaming-json sequence
//! through the pure stepper and assert the agent::events frame sequence +
//! accumulated turn state (session id, result, error). This is the
//! orchestration the stream loop runs, exercised without a live engine.
//! Event shapes captured from Grok CLI 0.2.77.

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
