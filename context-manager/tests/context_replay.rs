#[allow(dead_code)]
mod common;
mod replay_support;

use replay_support::{compare_decay_four, compare_directory, parse_session_lines};
use serde_json::json;

#[test]
fn keeps_the_last_revision_in_first_occurrence_order() {
    let lines = [
        r#"{"type":"entry","entry":{"kind":"message","id":"first","revision":0,"message":{"role":"user","content":[{"type":"text","text":"old"}],"timestamp":1}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"second","revision":0,"message":{"role":"user","content":[{"type":"text","text":"second"}],"timestamp":2}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"first","revision":1,"message":{"role":"user","content":[{"type":"text","text":"new"}],"timestamp":3}}}"#,
    ];

    let history = parse_session_lines("synthetic.jsonl", lines).expect("valid session records");

    assert_eq!(history.ids(), ["first", "second"]);
    assert_eq!(history.revisions(), [1, 0]);
    assert_eq!(history.text_at(0), Some("new"));
}

#[test]
fn finds_only_actual_user_turn_endpoints() {
    let lines = [
        r#"{"type":"entry","entry":{"kind":"message","id":"one","revision":0,"message":{"role":"user","content":[{"type":"text","text":"first"}],"timestamp":1}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"inline","revision":0,"message":{"role":"user","content":[{"type":"function_result","function_call_id":"call","content":[{"type":"text","text":"result"}]}],"timestamp":2}}}"#,
        r#"{"type":"entry","entry":{"kind":"custom","id":"notice","revision":0,"data":{}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"assistant","revision":0,"message":{"role":"assistant","content":[],"stop_reason":"end","model":"test","provider":"test","timestamp":3}}}"#,
        r#"{"type":"entry","entry":{"kind":"message","id":"two","revision":0,"message":{"role":"user","content":[{"type":"text","text":"second"}],"timestamp":4}}}"#,
    ];

    let history = parse_session_lines("synthetic.jsonl", lines).expect("valid session records");

    assert_eq!(history.user_turn_endpoints(), [0, 3]);
}

#[test]
fn rejects_malformed_records_without_echoing_their_contents() {
    let result = parse_session_lines(
        "synthetic.jsonl",
        [
            r#"{"type":"entry","entry":{"kind":"message","id":"one","revision":0,"message":{"role":"user","content":"private transcript text"}}}"#,
        ],
    );
    let Err(error) = result else {
        panic!("invalid message shape must be rejected");
    };

    assert_eq!(error, "synthetic.jsonl:1: message entry message is invalid");
    assert!(!error.contains("private transcript text"));
}

#[tokio::test]
async fn shrinks_a_long_medium_result_history_with_shipped_guards() {
    let mut lines = Vec::new();
    for turn in 0..130 {
        lines.push(
            json!({
                "type": "entry",
                "entry": {
                    "kind": "message",
                    "id": format!("user-{turn}"),
                    "revision": 0,
                    "message": {
                        "role": "user",
                        "content": [{ "type": "text", "text": "continue" }],
                        "timestamp": turn * 2
                    }
                }
            })
            .to_string(),
        );
        if turn < 129 {
            lines.push(
                json!({
                    "type": "entry",
                    "entry": {
                        "kind": "message",
                        "id": format!("result-{turn}"),
                        "revision": 0,
                        "message": {
                            "role": "function_result",
                            "function_call_id": format!("call-{turn}"),
                            "function_id": "read_file",
                            "content": [{ "type": "text", "text": "x".repeat(1_999) }],
                            "details": {},
                            "is_error": false,
                            "timestamp": turn * 2 + 1
                        }
                    }
                })
                .to_string(),
            );
        }
    }
    let history = parse_session_lines("synthetic-long.jsonl", lines.iter().map(String::as_str))
        .expect("valid session records");

    let comparison = compare_decay_four(&history)
        .await
        .expect("generous inline budget avoids emergency reduction");

    assert_eq!(comparison.turn_count(), 130);
    assert!(comparison.final_decay_tokens() < comparison.final_baseline_tokens());
}

#[tokio::test]
async fn compares_an_empty_history_without_dividing_by_zero() {
    let history = parse_session_lines("empty.jsonl", std::iter::empty())
        .expect("an empty JSONL history is valid");

    let comparison = compare_decay_four(&history)
        .await
        .expect("empty history needs no assembly calls");

    assert_eq!(comparison.turn_count(), 0);
    assert_eq!(comparison.final_baseline_tokens(), 0);
    assert_eq!(comparison.final_decay_tokens(), 0);
}

#[tokio::test]
#[ignore = "reads the explicitly selected session-manager corpus"]
async fn replays_an_explicit_session_manager_directory() {
    let directory = std::env::var("CONTEXT_REPLAY_DIR")
        .expect("set CONTEXT_REPLAY_DIR to the session-manager JSONL directory");
    let report = compare_directory(std::path::Path::new(&directory))
        .await
        .expect("corpus records and production assembly must succeed");

    print!("{}", report.render());
}
