//! `tmux::terminal::*` against a scripted engine: every Console terminal
//! function registers with `observable: false`, since `open` returns the
//! terminal's access key, every other call carries it, and `write` carries
//! the keystrokes.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::terminal::{self, TerminalManager};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// A loopback engine that advertises observe.v2 and hands every frame the
/// worker sends to the returned receiver.
async fn engine() -> (String, mpsc::UnboundedReceiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("ws://{}", listener.local_addr().unwrap());
    let (captured_tx, captured) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let hello = json!({
            "type": "workerregistered",
            "worker_id": "tmux-test",
            "features": ["observe.v1", "observe.v2"],
        });
        if ws
            .send(WsMessage::Text(hello.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
        while let Some(Ok(message)) = ws.next().await {
            match message {
                WsMessage::Text(text) => {
                    let _ = captured_tx.send(serde_json::from_str(&text).unwrap());
                }
                WsMessage::Ping(data) => {
                    if ws.send(WsMessage::Pong(data)).await.is_err() {
                        break;
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
    });
    (address, captured)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn console_terminal_functions_cannot_be_observed() {
    let (address, mut captured) = engine().await;
    let iii = register_worker(&address, InitOptions::default());
    let config = WorkerConfig::default().into_shared();
    let console = ConsoleClients::default();
    let collector = Arc::new(Collector::new(config, console.clone()));
    let manager = TerminalManager::new(
        Arc::new(iii.clone()),
        collector,
        console,
        Arc::new(|| {}),
        Arc::new(|| Some(true)),
    );
    terminal::register(&iii, manager);

    let expected: Vec<&str> = terminal::FUNCTIONS.iter().map(|(id, _)| *id).collect();
    let mut registrations = BTreeMap::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while registrations.len() < expected.len() {
            let frame = captured.recv().await.unwrap();
            if frame["type"] == "registerfunction" {
                registrations.insert(frame["id"].as_str().unwrap().to_string(), frame);
            }
        }
    })
    .await
    .expect("every tmux::terminal::* function registers");

    for id in expected {
        let registration = &registrations[id];
        assert_eq!(registration["observable"], false, "{registration}");
        assert!(registration.get("private").is_none(), "{registration}");
        assert_eq!(registration["metadata"]["internal"], true, "{registration}");
    }

    tokio::task::spawn_blocking(move || iii.shutdown())
        .await
        .unwrap();
}
