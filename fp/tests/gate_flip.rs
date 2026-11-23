//! A gate that registers while a pipe or batch runs must still refuse the
//! later steps it decides. Driven through an in-process fake engine: it
//! answers `engine::functions::info` for `approval::gate` from its current
//! state, and `probe::*` steps change that state while the run is under way.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use fp::{batch, pipe};
use futures_util::{SinkExt, StreamExt};
use iii_sdk::{IIIClient, InitOptions, register_worker};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy, PartialEq)]
enum Gate {
    Absent,
    Running,
    Unknown,
}

struct Engine {
    gate: Mutex<Gate>,
    info_calls: AtomicUsize,
    dispatched: Mutex<Vec<String>>,
}

impl Engine {
    fn answer(&self, function: &str) -> Result<Value, (&'static str, String)> {
        if function == "engine::functions::info" {
            self.info_calls.fetch_add(1, Ordering::SeqCst);
            return match *self.gate.lock().unwrap() {
                Gate::Running => Ok(json!({ "function_id": "approval::gate" })),
                Gate::Absent => Err((
                    "NOT_FOUND",
                    "Function 'approval::gate' is not registered.".into(),
                )),
                Gate::Unknown => Err(("TIMEOUT", "info timed out".into())),
            };
        }
        self.dispatched.lock().unwrap().push(function.to_string());
        match function {
            "probe::start-gate" => *self.gate.lock().unwrap() = Gate::Running,
            "probe::lose-gate" => *self.gate.lock().unwrap() = Gate::Unknown,
            _ => {}
        }
        Ok(json!({ "ran": function }))
    }

    fn dispatched(&self) -> Vec<String> {
        self.dispatched.lock().unwrap().clone()
    }
}

/// A fake engine with no gate, and a client connected to it.
async fn stack() -> (Arc<Engine>, Arc<IIIClient>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let engine = Arc::new(Engine {
        gate: Mutex::new(Gate::Absent),
        info_calls: AtomicUsize::new(0),
        dispatched: Mutex::new(Vec::new()),
    });
    let state = engine.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let state = state.clone();
            tokio::spawn(async move {
                let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut tx, mut rx) = ws.split();
                while let Some(Ok(message)) = rx.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let frame: Value = serde_json::from_str(text.as_str()).unwrap_or_default();
                    // Void calls (the worker's own declaration) need no answer.
                    let (Some("invokefunction"), Some(id), Some(function)) = (
                        frame["type"].as_str(),
                        frame["invocation_id"].as_str(),
                        frame["function_id"].as_str(),
                    ) else {
                        continue;
                    };
                    let mut reply = json!({
                        "type": "invocationresult",
                        "invocation_id": id,
                        "function_id": function,
                    });
                    match state.answer(function) {
                        Ok(result) => reply["result"] = result,
                        Err((code, message)) => {
                            reply["error"] = json!({ "code": code, "message": message })
                        }
                    }
                    if tx.send(Message::text(reply.to_string())).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (
        engine,
        Arc::new(register_worker(&url, InitOptions::default())),
    )
}

fn pipe_req(through: Value) -> pipe::PipeRequest {
    serde_json::from_value(json!({ "through": through })).unwrap()
}

#[tokio::test]
async fn a_gate_that_starts_mid_pipe_refuses_a_later_step_in_another_namespace() {
    let (engine, iii) = stack().await;
    let error = pipe::run(
        &iii,
        pipe_req(json!([
            { "function": "probe::start-gate" },
            { "function": "probe::target", "namespace": "laptop" },
        ])),
    )
    .await
    .expect_err("the gate started before step 2");
    assert!(error.contains("step 2 (probe::target)"), "{error}");
    assert!(
        error.contains("must be approved in that namespace"),
        "{error}"
    );
    assert_eq!(engine.dispatched(), ["probe::start-gate"]);
}

#[tokio::test]
async fn a_gate_that_starts_mid_pipe_refuses_a_later_database_write() {
    let (engine, iii) = stack().await;
    let error = pipe::run(
        &iii,
        pipe_req(json!([
            { "function": "probe::start-gate" },
            { "function": "database::execute", "payload": { "sql": "delete from t" } },
        ])),
    )
    .await
    .expect_err("the gate started before step 2");
    assert!(error.contains("step 2 (database::execute)"), "{error}");
    assert!(error.contains("database::query"), "{error}");
    assert_eq!(engine.dispatched(), ["probe::start-gate"]);
}

#[tokio::test]
async fn a_gate_lost_mid_pipe_keeps_the_lookup_error_for_a_database_write() {
    let (engine, iii) = stack().await;
    let error = pipe::run(
        &iii,
        pipe_req(json!([
            { "function": "probe::lose-gate" },
            { "function": "database::execute", "payload": { "sql": "delete from t" } },
        ])),
    )
    .await
    .expect_err("the gate's state is unknown at step 2");
    assert!(error.contains("step 2 (database::execute)"), "{error}");
    assert!(
        error.contains("could not inspect approval-gate availability"),
        "{error}"
    );
    assert_eq!(engine.dispatched(), ["probe::lose-gate"]);
}

#[tokio::test]
async fn a_pipe_with_no_gated_step_never_asks_for_the_gate() {
    let (engine, iii) = stack().await;
    pipe::run(
        &iii,
        pipe_req(json!([
            { "function": "probe::start-gate" },
            { "function": "probe::target" },
        ])),
    )
    .await
    .unwrap();
    assert_eq!(engine.info_calls.load(Ordering::SeqCst), 0);
    assert_eq!(engine.dispatched(), ["probe::start-gate", "probe::target"]);
}

#[tokio::test]
async fn a_gate_that_starts_mid_batch_refuses_a_later_call_in_another_namespace() {
    for options in [
        json!({ "concurrency": 1 }),
        json!({ "concurrency": 1, "stop_on_error": true }),
    ] {
        let (engine, iii) = stack().await;
        let mut req = json!({ "calls": [
            { "function": "probe::start-gate" },
            { "function": "probe::target", "namespace": "laptop" },
            { "function": "probe::after" },
        ] });
        req.as_object_mut()
            .unwrap()
            .extend(options.as_object().unwrap().clone());
        let response = batch::run(iii, serde_json::from_value(req).unwrap())
            .await
            .unwrap();
        let batch::BatchResult::Error { code, message, .. } = &response.results[1] else {
            panic!("{options}: {:?}", response.results[1]);
        };
        assert_eq!(code, "FORBIDDEN", "{options}");
        assert!(
            message.contains("must be approved in that namespace"),
            "{message}"
        );
        let ran_after = options["stop_on_error"] != json!(true);
        let mut expected = vec!["probe::start-gate".to_string()];
        if ran_after {
            expected.push("probe::after".into());
        }
        assert_eq!(engine.dispatched(), expected, "{options}");
    }
}
