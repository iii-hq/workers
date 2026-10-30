//! A devin::run turn's frames through the owned feeds, exercised without a
//! live engine: the run loop delivers each CLI stdout line on
//! `devin::on-raw-event` (as `stdout_frame`) and the terminal frames on
//! `devin::on-agent-event` (as `terminal_frames`), each call made only for an
//! observed session. A capturing sender stands in for the engine.

use std::sync::Mutex;

use devin::agent_feed::{Feed, FeedPayload, Feeds};
use devin::cli::{stdout_frame, terminal_frames};
use devin::config::Config;
use serde_json::{json, Value};

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
    let stdout = [
        "Working on it",
        r#"{"id":"devin-abc","url":"https://app.devin.ai/sessions/abc"}"#,
        "done",
    ];
    let feeds = Feeds::new("epoch-turn");
    let raw_sink = Mutex::new(Vec::new());
    let agent_sink = Mutex::new(Vec::new());
    for line in stdout {
        push(&feeds.raw, "s1", "s1", stdout_frame(line), &raw_sink).await;
    }
    let terminal = terminal_frames(&stdout.join("\n"), "swe-1.6", "end");
    for frame in terminal.clone() {
        push(&feeds.agent, "s1", "s1", frame, &agent_sink).await;
    }
    // Another session's frames never reach the s1 observer.
    push(&feeds.raw, "s1", "s2", stdout_frame("other"), &raw_sink).await;

    let raw = raw_sink.into_inner().unwrap();
    let agent = agent_sink.into_inner().unwrap();
    let events = |sent: &[FeedPayload]| sent.iter().map(|p| p.event.clone()).collect::<Vec<_>>();
    let seqs = |sent: &[FeedPayload]| sent.iter().map(|p| p.seq).collect::<Vec<_>>();
    assert_eq!(
        events(&raw),
        stdout
            .iter()
            .map(|l| json!({ "type": "stdout", "line": l }))
            .collect::<Vec<_>>()
    );
    assert_eq!(events(&agent), terminal.to_vec());
    assert_eq!(agent[0].event["type"], "turn_end");
    assert_eq!(
        agent[0].event["message"]["content"][0]["text"],
        stdout.join("\n")
    );
    assert_eq!(agent[1].event["type"], "agent_end");
    assert_eq!(seqs(&raw), vec![0, 1, 2]);
    assert_eq!(seqs(&agent), vec![0, 1]);
    for payload in raw.iter().chain(agent.iter()) {
        assert_eq!(payload.session_id, "s1");
        assert_eq!(payload.source, "devin");
        assert_eq!(
            payload.event_id,
            format!("s1-epoch-turn-{:08}", payload.seq)
        );
    }
}

#[test]
fn stream_name_keys_load_and_are_ignored() {
    let stored = json!({
        "devin_executable": "/opt/devin",
        "events_stream": "agent::events",
        "raw_events_stream": "devin::events",
    });
    let cfg = Config::from_json(&stored).unwrap();
    assert_eq!(cfg.devin_executable, "/opt/devin");
    let written = cfg.to_json();
    assert!(written.get("events_stream").is_none());
    assert!(written.get("raw_events_stream").is_none());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "events_stream: agent::events\nraw_events_stream: devin::events\n",
    )
    .unwrap();
    assert!(Config::load(path.to_str().unwrap()).is_ok());
}

#[test]
fn schema_manifest_and_seed_carry_no_stream_keys() {
    let schema = Config::json_schema().to_string();
    assert!(!schema.contains("events_stream"), "{schema}");
    let manifest = serde_json::to_value(devin::manifest::build_manifest()).unwrap();
    let defaults = manifest["default_config"].as_object().unwrap();
    assert!(!defaults.contains_key("events_stream"));
    assert!(!defaults.contains_key("raw_events_stream"));
    assert!(!manifest["description"]
        .as_str()
        .unwrap()
        .contains("agent::events"));
    for file in ["config.yaml", "iii.worker.yaml"] {
        let path = format!("{}/{file}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("events_stream:"), "{file} still declares it");
    }
    let seed = Config::load(concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml")).unwrap();
    assert!(seed.to_json().get("events_stream").is_none());
}
