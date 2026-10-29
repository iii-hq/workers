//! Replays the deterministic detectors without the monitor's queue or models.
//!
//! Offline, over entries exported with `session::messages`
//! (`include_custom: true`, every page):
//!
//! ```sh
//! cargo run --example replay -- <session_id>=<entries.json> [...]
//! ```
//!
//! Live and read-only, against a running engine (`III_URL`, `III_NAMESPACE`):
//! runs the real collection stage for each session's current turn and prints
//! the snapshot. Nothing is written and no model is called.
//!
//! ```sh
//! III_NAMESPACE=my-project cargo run --example replay -- --live <session_id> [...]
//! ```

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let report = if arguments.first().map(String::as_str) == Some("--live") {
        live(&arguments[1..]).await
    } else {
        arguments.iter().map(|argument| offline(argument)).collect()
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

fn offline(argument: &str) -> Value {
    let (session_id, path) = argument
        .split_once('=')
        .unwrap_or_else(|| panic!("expected <session_id>=<path>, got {argument}"));
    let raw: Value = serde_json::from_str(
        &std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}")),
    )
    .unwrap_or_else(|error| panic!("parse {path}: {error}"));
    let entries = match raw {
        Value::Array(entries) => entries,
        mut page => page["messages"]
            .take()
            .as_array()
            .cloned()
            .unwrap_or_default(),
    };
    let detection = eval::diagnostics::detect(session_id, &entries);
    json!({
        "session_id": session_id,
        "entries": entries.len(),
        "unreadable_messages": detection.unreadable_messages,
        "diagnostics": detection.diagnostics,
    })
}

async fn live(sessions: &[String]) -> Vec<Value> {
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = Arc::new(iii_sdk::register_worker(
        &url,
        iii_sdk::InitOptions::default(),
    ));
    tokio::time::timeout(Duration::from_secs(15), async {
        while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("engine connection");
    let deps = eval::runtime::Deps::new(iii.clone(), eval::events::EvalEvents::detached(&iii));
    let mut report = Vec::new();
    for session_id in sessions {
        report.push(match eval::runtime::capture(&deps, session_id).await {
            Ok(snapshot) => json!({
                "session_id": session_id,
                "turn_id": snapshot.source_turn_id,
                "status": snapshot.source_status,
                "observed_model": snapshot.observed_model,
                "observed_provider": snapshot.observed_provider,
                "window_turn_ids": snapshot.window_turn_ids,
                "sessions": snapshot.sessions.iter().map(|session| json!({
                    "session_id": session.session_id,
                    "in_scope": session.in_scope,
                    "entries": session.entries,
                    "preview": session.preview.len(),
                    "omitted": session.omitted_entries,
                    "reduced": session.reduced_entries,
                })).collect::<Vec<_>>(),
                "metrics_complete": snapshot.metrics.complete,
                "totals": snapshot.metrics.totals,
                "diagnostics": snapshot.diagnostics.iter().map(|diagnostic| json!({
                    "rule_id": diagnostic.rule_id,
                    "correlation": diagnostic.correlation,
                    "turn_id": diagnostic.turn_id,
                    "evidence": diagnostic.evidence.len(),
                })).collect::<Vec<_>>(),
                "coverage": snapshot.coverage,
                "model_context_bytes": serde_json::to_vec(&eval::runtime::model_context(&snapshot)).unwrap().len(),
            }),
            Err(error) => json!({ "session_id": session_id, "error": error.to_string() }),
        });
    }
    iii.shutdown_async().await;
    report
}
