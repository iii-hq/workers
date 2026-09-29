//! Deterministic loopback-only engine for HTTP integration tests. No external
//! service, real engine registration, telemetry endpoint, or skipped assertions.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::{
    handshake::server::{Request, Response},
    Message,
};

struct Channel {
    sender: mpsc::UnboundedSender<Message>,
    receiver: Option<mpsc::UnboundedReceiver<Message>>,
}

#[derive(Default)]
struct State {
    channels: Mutex<HashMap<String, Channel>>,
    config: Mutex<Value>,
    providers: AtomicUsize,
}

pub struct FakeEngine {
    pub url: String,
    pub iii: Arc<IIIClient>,
    state: Arc<State>,
    task: JoinHandle<()>,
}

impl FakeEngine {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        connections.spawn(connection(stream, shared.clone()));
                    }
                    Some(result) = connections.join_next() => { result.unwrap(); }
                }
            }
        });
        let iii = Arc::new(register_worker(&url, InitOptions::default()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("local fake engine handshake");
        Self {
            url,
            iii,
            state,
            task,
        }
    }

    pub async fn set_config(&self, value: Value) {
        *self.state.config.lock().await = value;
    }

    pub fn http_provider_count(&self) -> usize {
        self.state.providers.load(Ordering::SeqCst)
    }

    pub async fn shutdown(self) {
        self.iii.shutdown_async().await;
        self.task.abort();
    }
}

impl Drop for FakeEngine {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// Tungstenite's handshake callback fixes this error type; boxing would violate its API.
#[expect(clippy::result_large_err)]
async fn connection(stream: TcpStream, state: Arc<State>) {
    let path = Arc::new(StdMutex::new(String::new()));
    let captured = path.clone();
    let socket =
        tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, response: Response| {
            *captured.lock().unwrap() = req.uri().to_string();
            Ok(response)
        })
        .await
        .unwrap();
    let path = path.lock().unwrap().clone();
    let (mut sink, mut source) = socket.split();
    if let Some(channel_path) = path.strip_prefix("/ws/channels/") {
        let id = channel_path.split('?').next().unwrap();
        if path.contains("dir=write") {
            let sender = state.channels.lock().await.get(id).unwrap().sender.clone();
            while let Some(Ok(message)) = source.next().await {
                let closed = message.is_close();
                if sender.send(message).is_err() || closed {
                    break;
                }
            }
        } else {
            let receiver = state
                .channels
                .lock()
                .await
                .get_mut(id)
                .unwrap()
                .receiver
                .take();
            let mut receiver = receiver.expect("one reader per channel");
            while let Some(message) = receiver.recv().await {
                let closed = message.is_close();
                if sink.send(message).await.is_err() || closed {
                    break;
                }
            }
        }
        return;
    }

    while let Some(Ok(message)) = source.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let frame: Value = serde_json::from_str(&text).unwrap();
        let reply = match frame["type"].as_str().unwrap_or_default() {
            "registertriggertype" => {
                if frame["id"] == "http" {
                    state.providers.fetch_add(1, Ordering::SeqCst);
                }
                None
            }
            "registertrigger" if frame["trigger_type"] == "http" => Some(frame),
            "unregistertrigger" | "invocationresult" => Some(frame),
            "invokefunction" => {
                let result = match frame["function_id"].as_str().unwrap() {
                    "engine::workers::list" => Some(json!({"workers": []})),
                    "configuration::get" => {
                        Some(json!({"value": state.config.lock().await.clone()}))
                    }
                    "engine::channels::create" => {
                        let id = uuid::Uuid::new_v4().to_string();
                        let (sender, receiver) = mpsc::unbounded_channel();
                        state.channels.lock().await.insert(
                            id.clone(),
                            Channel {
                                sender,
                                receiver: Some(receiver),
                            },
                        );
                        Some(json!({
                            "writer": {"channel_id": id, "access_key": "local", "direction": "write"},
                            "reader": {"channel_id": id, "access_key": "local", "direction": "read"}
                        }))
                    }
                    // Keep SDK telemetry local and disabled in this fixture.
                    "engine::telemetry::config" => Some(json!({"enabled": false})),
                    _ => None,
                };
                Some(match result {
                    Some(result) => {
                        json!({"type": "invocationresult", "invocation_id": frame["invocation_id"], "function_id": frame["function_id"], "result": result})
                    }
                    None => frame,
                })
            }
            _ => None,
        };
        if let Some(reply) = reply {
            if sink
                .send(Message::Text(reply.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}
