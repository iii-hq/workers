//! The real binary against a scripted engine on an ephemeral loopback port.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::{WebSocketStream, accept_async, accept_hdr_async, tungstenite::Message};

const CONFIG_NAME: &str = "resize-boot-test";
const RESIZE_CALL: &str = "00000000-0000-0000-0000-000000000002";

type Socket = WebSocketStream<TcpStream>;

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
/// With `resize`, it calls `image_resize::resize` once that registers and
/// hands back the input stream the worker dials for it; the output stream is
/// read to its end.
async fn engine(
    fail_ensure: bool,
    resize: bool,
) -> (
    String,
    mpsc::UnboundedReceiver<Value>,
    mpsc::UnboundedReceiver<Socket>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    let (input_tx, inputs) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        // Every later connection is a stream end the worker dials.
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut input = false;
                #[allow(clippy::result_large_err)] // tungstenite's Callback signature
                let callback = |request: &Request, response: Response| {
                    input = request.uri().path().ends_with("/input");
                    Ok(response)
                };
                let Ok(mut end) = accept_hdr_async(stream, callback).await else {
                    continue;
                };
                if input {
                    let _ = input_tx.send(end);
                } else {
                    tokio::spawn(async move { while let Some(Ok(_)) = end.next().await {} });
                }
            }
        });
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
            if resize
                && value["type"] == "registerfunction"
                && value["id"] == "image_resize::resize"
            {
                let end = |id, direction| json!({"channel_id": id, "access_key": "k", "direction": direction});
                socket
                    .send(text(json!({
                        "type": "invokefunction",
                        "invocation_id": RESIZE_CALL,
                        "function_id": "image_resize::resize",
                        "data": {"input_channel": end("input", "read"), "output_channel": end("output", "write")}
                    })))
                    .await
                    .unwrap();
            }
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
    (url, rx, inputs)
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

fn sigterm(worker: &Worker) {
    let kill = Command::new("kill")
        .args(["-TERM", &worker.0.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
}

/// Reads `logs` up to a line that contains `needle`.
async fn wait_for(logs: &mut mpsc::UnboundedReceiver<String>, needle: &str) {
    timeout(Duration::from_secs(20), async {
        while let Some(line) = logs.recv().await {
            if line.contains(needle) {
                return;
            }
        }
        panic!("worker exited before logging {needle:?}");
    })
    .await
    .unwrap_or_else(|_| panic!("the worker logs {needle:?}"));
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
    let (url, mut frames, _) = engine(false, false).await;
    let mut worker = worker(&url, &["--config", seed.to_str().unwrap()], None);
    let mut logs = lines(&mut worker);

    timeout(Duration::from_secs(20), async {
        let (mut ensured, mut resize, mut hook, mut observed, mut reloaded, mut ui_content) =
            (false, false, false, false, false, false);
        let mut ui_assets = std::collections::BTreeSet::new();
        let expected_assets = std::collections::BTreeSet::from([
            ("script".to_string(), "image-resize/page.js".to_string()),
            ("style".to_string(), "image-resize/styles.css".to_string()),
        ]);
        while !(ensured
            && resize
            && hook
            && observed
            && reloaded
            && ui_content
            && ui_assets == expected_assets)
        {
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
                "registerfunction" if value["id"] == "image-resize::ui-content" => {
                    assert_eq!(value["metadata"]["internal"], true);
                    ui_content = true;
                }
                "observe" if value["source"]["function_id"] == "ade::ui::asset" => {
                    assert_eq!(value["target"]["function_id"], "image-resize::ui-content");
                    let args = &value["source"]["args"];
                    let asset = (
                        args["kind"].as_str().unwrap().to_string(),
                        args["path"].as_str().unwrap().to_string(),
                    );
                    assert!(expected_assets.contains(&asset), "unexpected asset {args}");
                    assert!(ui_assets.insert(asset), "asset observed twice: {args}");
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
    .expect("the engine sees ensure, the functions, the observers and one reload");

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
    let (url, _frames, _) = engine(true, false).await;
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
async fn sigterm_unregisters_resize_and_exits_cleanly_with_blank_namespace() {
    let (url, mut frames, _) = engine(false, false).await;
    let mut worker = worker(&url, &[], Some(""));
    let mut logs = lines(&mut worker);
    wait_for(&mut logs, "waiting for invocations").await;

    sigterm(&worker);
    let status = exit_status(&mut worker, Duration::from_secs(10)).await;
    assert!(status.success(), "{status}");

    // The scripted engine ends, and closes `frames`, when the worker's socket closes.
    let unregistered = timeout(Duration::from_secs(10), async {
        while let Some(value) = frames.recv().await {
            if value["type"] == "unregisterfunction" && value["id"] == "image_resize::resize" {
                return true;
            }
        }
        false
    })
    .await
    .expect("the worker's socket closes");
    assert!(
        unregistered,
        "the worker stopped without unregistering image_resize::resize"
    );

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

/// A worker stopped by SIGTERM while a resize call it took waits on its
/// input, with the engine's end of that input and when the signal went.
struct Stopping {
    worker: Worker,
    frames: mpsc::UnboundedReceiver<Value>,
    logs: mpsc::UnboundedReceiver<String>,
    input: Socket,
    signalled: Instant,
}

async fn stop_during_a_call() -> Stopping {
    let (url, frames, mut inputs) = engine(false, true).await;
    let mut worker = worker(&url, &[], None);
    let mut logs = lines(&mut worker);
    let input = timeout(Duration::from_secs(20), inputs.recv())
        .await
        .expect("the worker reads the call's input")
        .unwrap();
    wait_for(&mut logs, "waiting for invocations").await;
    let signalled = Instant::now();
    sigterm(&worker);
    wait_for(&mut logs, "image-resize shutting down").await;
    Stopping {
        worker,
        frames,
        logs,
        input,
        signalled,
    }
}

async fn finish_input(input: &mut Socket, bytes: Vec<u8>) {
    input.send(Message::Binary(bytes.into())).await.unwrap();
    input.close(None).await.unwrap();
}

#[tokio::test]
async fn a_stop_sends_the_result_of_a_call_that_ends_during_the_drain() {
    let mut stop = stop_during_a_call().await;
    finish_input(&mut stop.input, b"not an image".to_vec()).await;

    let error = timeout(Duration::from_secs(20), async {
        while let Some(value) = stop.frames.recv().await {
            if value["type"] == "invocationresult" && value["invocation_id"] == RESIZE_CALL {
                return Some(value["error"].clone());
            }
        }
        None
    })
    .await
    .expect("the worker's socket closes")
    .expect("the worker closed its socket without sending the call's result");
    assert_eq!(error["code"], "INVALID_ARGS", "{error}");
    let status = exit_status(&mut stop.worker, Duration::from_secs(10)).await;
    assert!(status.success(), "{status}");
}

#[tokio::test]
async fn a_stop_waits_the_drain_deadline_but_not_for_a_resize_still_running() {
    let mut stop = stop_during_a_call().await;
    // About 12 s to decode and resize in a debug build; fed late, so it is still running at the 8 s deadline.
    let side = 7000;
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::NoFilter)
        .write_image(
            &vec![0; side * side],
            side as u32,
            side as u32,
            ExtendedColorType::L8,
        )
        .unwrap();
    tokio::time::sleep_until((stop.signalled + Duration::from_secs(7)).into()).await;
    assert!(
        stop.worker.0.try_wait().unwrap().is_none(),
        "the worker exited before the drain deadline"
    );
    finish_input(&mut stop.input, png).await;

    let limit = Duration::from_millis(9500);
    let status = exit_status(
        &mut stop.worker,
        limit.saturating_sub(stop.signalled.elapsed()),
    )
    .await;
    let stopped = stop.signalled.elapsed();
    assert!(status.success(), "{status}");
    assert!(
        stopped >= Duration::from_secs(8),
        "exited {stopped:?} after SIGTERM, before the drain deadline"
    );
    wait_for(
        &mut stop.logs,
        "calls still in flight at the drain deadline",
    )
    .await;
}
