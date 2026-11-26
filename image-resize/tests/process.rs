//! The real binary against a scripted engine on an ephemeral loopback port.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

const CONFIG_NAME: &str = "resize-boot-test";

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

/// Answers `configuration::ensure`/`get` from one stored value, and on the
/// reload observer changes the value and calls the worker's reload hook.
async fn engine(fail_ensure: bool) -> (String, mpsc::UnboundedReceiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(text(json!({
                "type": "workerregistered",
                "worker_id": "boot-test",
                "features": ["observe.v1", "observe.v2"]
            })))
            .await
            .unwrap();
        let mut stored = Value::Null;
        while let Some(Ok(frame)) = socket.next().await {
            let Message::Text(frame) = frame else {
                continue;
            };
            let value: Value = serde_json::from_str(&frame).unwrap();
            let _ = tx.send(value.clone());
            if value["type"] == "invokefunction" && value["invocation_id"].is_string() {
                let reply = |outcome: (&str, Value)| {
                    let mut reply = json!({
                        "type": "invocationresult",
                        "invocation_id": value["invocation_id"],
                        "function_id": value["function_id"],
                    });
                    reply[outcome.0] = outcome.1;
                    text(reply)
                };
                let outcome = match value["function_id"].as_str().unwrap() {
                    "configuration::ensure" if fail_ensure => (
                        "error",
                        json!({"code": "ADAPTER_ERROR", "message": "store unavailable"}),
                    ),
                    "configuration::ensure" => {
                        let action = if stored.is_null() {
                            stored = value["data"]["initial_value"].clone();
                            "seeded"
                        } else {
                            "preserved"
                        };
                        (
                            "result",
                            json!({"action": action, "entry": {"id": value["data"]["id"], "value": stored}}),
                        )
                    }
                    "configuration::get" => ("result", json!({"value": stored})),
                    _ => ("result", json!({})),
                };
                socket.send(reply(outcome)).await.unwrap();
            }
            if value["type"] == "observe"
                && value["source"]["function_id"] == "configuration::on-updated"
            {
                stored["width"] = json!(64);
                socket
                    .send(text(json!({
                        "type": "invokefunction",
                        "invocation_id": "00000000-0000-0000-0000-000000000001",
                        "function_id": "image-resize::on-config-change",
                        "data": {"id": CONFIG_NAME}
                    })))
                    .await
                    .unwrap();
            }
        }
    });
    (url, rx)
}

/// `namespace` is the III_NAMESPACE the worker sees; `None` leaves it unset.
fn worker(url: &str, extra: &[&str], namespace: Option<&str>) -> Worker {
    let mut command = Command::new(env!("CARGO_BIN_EXE_image-resize"));
    command
        .args(extra)
        // The SDK writes an `iii ps` marker under $HOME/.iii/run.
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("III_CONFIG_NAME", CONFIG_NAME)
        .env("III_URL", url)
        .env("OTEL_ENABLED", "true")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match namespace {
        Some(namespace) => command.env("III_NAMESPACE", namespace),
        None => command.env_remove("III_NAMESPACE"),
    };
    Worker(command.spawn().unwrap())
}

async fn exit_status(worker: &mut Worker, limit: Duration) -> ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "worker still running after {limit:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn lines(worker: &mut Worker) -> mpsc::UnboundedReceiver<String> {
    let stdout = worker.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    rx
}

#[tokio::test]
async fn boot_registers_seeds_and_reloads_its_configuration() {
    let seed = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("boot-seed.yaml");
    std::fs::write(&seed, "width: 320\n").unwrap();
    let (url, mut frames) = engine(false).await;
    let mut worker = worker(&url, &["--config", seed.to_str().unwrap()], None);
    let mut logs = lines(&mut worker);

    timeout(Duration::from_secs(20), async {
        let (mut ensured, mut resize, mut hook, mut observed, mut reloaded) =
            (false, false, false, false, false);
        while !(ensured && resize && hook && observed && reloaded) {
            let value = frames.recv().await.expect("worker stays connected");
            match value["type"].as_str().unwrap_or_default() {
                "invokefunction" if value["function_id"] == "configuration::ensure" => {
                    assert_eq!(value["namespace"], "default");
                    assert_eq!(value["data"]["id"], CONFIG_NAME);
                    assert_eq!(value["data"]["metadata"]["ui_form"], "image-resize");
                    assert_eq!(
                        value["data"]["initial_value"],
                        json!({"width":320,"height":200,"strategy":"scale-to-fit","quality":{"jpeg":85,"webp":80}})
                    );
                    assert!(value["data"]["schema"]["properties"]["width"].is_object());
                    ensured = true;
                }
                "invokefunction" if value["function_id"] == "configuration::register" => {
                    panic!("boot must not fall back to configuration::register");
                }
                "registerfunction" if value["id"] == "image_resize::resize" => resize = true,
                "registerfunction" if value["id"] == "image-resize::on-config-change" => {
                    assert_eq!(value["metadata"]["internal"], true);
                    hook = true;
                }
                "observe" if value["source"]["function_id"] == "configuration::on-updated" => {
                    assert_eq!(
                        value["target"]["function_id"],
                        "image-resize::on-config-change"
                    );
                    assert_eq!(
                        value["source"]["filters"],
                        json!({
                            "input.id": {"$eq": CONFIG_NAME},
                            "input.event_type": {"$eq": "configuration:updated"}
                        })
                    );
                    observed = true;
                }
                "invocationresult" if value["function_id"] == "image-resize::on-config-change" => {
                    assert_eq!(value["result"], json!({"ok": true}));
                    reloaded = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the engine sees ensure, both functions, the observer and one reload");

    timeout(Duration::from_secs(20), async {
        while let Some(line) = logs.recv().await {
            if line.contains("image-resize configuration reloaded") {
                return;
            }
        }
        panic!("worker exited before logging the reload");
    })
    .await
    .expect("the worker logs the reload");
}

#[tokio::test]
async fn boot_fails_when_the_configuration_service_fails() {
    let (url, _frames) = engine(true).await;
    let mut worker = worker(&url, &[], None);
    let status = exit_status(&mut worker, Duration::from_secs(20)).await;
    assert!(!status.success(), "{status}");
    let mut stderr = String::new();
    worker
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.contains("image-resize configuration"), "{stderr}");
}

#[tokio::test]
async fn sigterm_exits_cleanly_with_blank_namespace() {
    let (url, _frames) = engine(false).await;
    let mut worker = worker(&url, &[], Some(""));
    let mut logs = lines(&mut worker);
    timeout(Duration::from_secs(20), async {
        while let Some(line) = logs.recv().await {
            if line.contains("waiting for invocations") {
                return;
            }
        }
        panic!("worker exited before reporting ready");
    })
    .await
    .expect("the worker reports ready");

    let kill = Command::new("kill")
        .args(["-TERM", &worker.0.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let status = exit_status(&mut worker, Duration::from_secs(10)).await;
    assert!(status.success(), "{status}");

    // The SDK logs this from its connection thread, so it only appears when main joins that thread.
    let mut otel_shut_down = false;
    while let Some(line) = logs.recv().await {
        otel_shut_down |= line.contains("OpenTelemetry shut down");
    }
    assert!(
        otel_shut_down,
        "main returned before the SDK shut down OpenTelemetry"
    );
}
