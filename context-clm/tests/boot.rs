//! Boot the real binary against a one-connection fake engine: it must seed
//! and read its configuration entry, register typed internal functions, bind
//! the reload trigger, answer the console's identity lookup, report
//! readiness and exit cleanly on SIGTERM.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Every function the worker registers at boot.
const FUNCTIONS: &[&str] = &[
    "context-clm::on-config-change",
    "context-clm::configuration-id",
];

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Accepts one worker, answers `configuration::ensure`/`get` like the
/// configuration worker (seed only when nothing is stored), invokes the
/// identity function once it is registered, and forwards every frame.
async fn fake_engine() -> (
    String,
    mpsc::UnboundedReceiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, frames) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let hello = json!({"type": "workerregistered", "worker_id": "boot-test"});
        socket
            .send(Message::Text(hello.to_string().into()))
            .await
            .unwrap();
        let mut stored = Value::Null;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let frame: Value = serde_json::from_str(&text).unwrap();
            let _ = tx.send(frame.clone());
            let mut replies = Vec::new();
            if frame["type"] == "invokefunction" && frame["invocation_id"].is_string() {
                let result = match frame["function_id"].as_str().unwrap() {
                    "configuration::ensure" => {
                        if stored.is_null() {
                            stored = frame["data"]["initial_value"].clone();
                        }
                        json!({"action": "seeded", "entry": {"id": frame["data"]["id"], "value": stored}})
                    }
                    "configuration::get" => json!({"value": stored}),
                    _ => json!({}),
                };
                replies.push(json!({
                    "type": "invocationresult",
                    "invocation_id": frame["invocation_id"],
                    "function_id": frame["function_id"],
                    "result": result,
                }));
            }
            if frame["type"] == "registerfunction" && frame["id"] == "context-clm::configuration-id"
            {
                replies.push(json!({
                    "type": "invokefunction",
                    "invocation_id": "00000000-0000-0000-0000-000000000001",
                    "function_id": "context-clm::configuration-id",
                    "data": {},
                }));
            }
            for reply in replies {
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
            }
        }
    });
    (url, frames, server)
}

#[tokio::test]
async fn boots_configures_registers_and_exits_cleanly_on_sigterm() {
    let (url, mut frames, server) = fake_engine().await;
    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_context-clm"))
            .env("III_URL", &url)
            .env_remove("III_CONFIG_NAME")
            .env_remove("III_WORKER_NAME")
            .env_remove("III_NAMESPACE")
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = worker.0.stdout.take().unwrap();
    let (log_tx, mut logs) = mpsc::unbounded_channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if log_tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });

    let expected: BTreeSet<String> = FUNCTIONS.iter().map(|id| id.to_string()).collect();
    timeout(Duration::from_secs(10), async {
        let (mut registered, mut seeded, mut reload, mut identity) =
            (false, false, false, false);
        let mut functions = BTreeSet::new();
        while !(registered && seeded && reload && identity && functions == expected) {
            let frame = frames.recv().await.expect("worker stays connected");
            match frame["type"].as_str().unwrap() {
                "invokefunction" if frame["function_id"] == "engine::workers::register" => {
                    assert_eq!(frame["data"]["name"], "context-clm");
                    registered = true;
                }
                "invokefunction" if frame["function_id"] == "configuration::ensure" => {
                    assert_eq!(frame["namespace"], "default");
                    assert_eq!(frame["data"]["id"], "context-clm");
                    assert_eq!(frame["data"]["metadata"]["ui_form"], "context-clm");
                    assert_eq!(frame["data"]["initial_value"]["budget_tokens"], 65536);
                    seeded = true;
                }
                "invokefunction" if frame["function_id"] == "configuration::register" => {
                    panic!("boot must never fall back to legacy configuration::register");
                }
                "registerfunction" => {
                    let id = frame["id"].as_str().unwrap();
                    assert!(expected.contains(id), "unexpected function {id}");
                    for field in ["request_format", "response_format"] {
                        let schema = frame[field].as_object().expect("schema is an object");
                        assert!(
                            ["type", "$ref", "oneOf", "anyOf", "allOf", "enum", "properties"]
                                .iter()
                                .any(|key| schema.contains_key(*key)),
                            "{id}.{field} must be typed"
                        );
                    }
                    assert_eq!(frame["metadata"]["internal"], true, "{id} must be internal");
                    functions.insert(id.to_owned());
                }
                "registertrigger" if frame["trigger_type"] == "configuration" => {
                    assert_eq!(frame["function_id"], "context-clm::on-config-change");
                    assert_eq!(
                        frame["config"],
                        json!({"configuration_id": "context-clm", "event_types": ["configuration:updated"]})
                    );
                    reload = true;
                }
                "invocationresult" if frame["function_id"] == "context-clm::configuration-id" => {
                    assert_eq!(frame["result"], json!({"id": "context-clm"}));
                    identity = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("worker boots against the fake engine");

    timeout(Duration::from_secs(10), async {
        while let Some(line) = logs.recv().await {
            if line.contains("context-clm ready") {
                return;
            }
        }
        panic!("worker exited before reporting readiness");
    })
    .await
    .expect("worker reports readiness");

    assert!(Command::new("kill")
        .args(["-TERM", &worker.0.id().to_string()])
        .status()
        .unwrap()
        .success());
    let status = timeout(Duration::from_secs(20), async {
        loop {
            if let Some(status) = worker.0.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("worker exits after SIGTERM");
    assert!(status.success(), "clean exit on SIGTERM: {status}");
    drop(worker);
    reader.join().unwrap();
    server.abort();
}
