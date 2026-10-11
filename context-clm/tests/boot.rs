//! Boot the real binary against a one-connection fake engine: it must
//! register itself, report readiness and exit cleanly on SIGTERM.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn boots_registers_and_exits_cleanly_on_sigterm() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, mut frames) = mpsc::unbounded_channel::<Value>();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let hello = json!({"type": "workerregistered", "worker_id": "boot-test"});
        socket
            .send(Message::Text(hello.to_string().into()))
            .await
            .unwrap();
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let _ = tx.send(serde_json::from_str(&text).unwrap());
        }
    });

    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_context-clm"))
            .env("III_URL", &url)
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

    timeout(Duration::from_secs(10), async {
        loop {
            let frame = frames.recv().await.expect("worker stays connected");
            if frame["type"] == "invokefunction"
                && frame["function_id"] == "engine::workers::register"
            {
                assert_eq!(frame["data"]["name"], "context-clm");
                break;
            }
        }
    })
    .await
    .expect("worker registers with the engine");

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
