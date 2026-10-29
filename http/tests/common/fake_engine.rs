//! Deterministic loopback-only engine for HTTP integration tests. No external
//! service, real engine registration, telemetry endpoint, or skipped assertions.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex, Notify};
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
    /// Advertise `observe.v1` in `workerregistered`, as a 0.25 engine does.
    observe: AtomicBool,
    /// Frames pushed to the worker's (latest) control connection.
    push: StdMutex<Option<mpsc::UnboundedSender<Value>>>,
    /// Observer revision counter, as the engine keeps per function.
    revision: AtomicU64,
    /// `observerack` frames by observer id (the latest one).
    acks: StdMutex<HashMap<String, Value>>,
    acked: Notify,
    /// Function ids the worker registered (`registerfunction`).
    functions: StdMutex<HashSet<String>>,
    registered: Notify,
}

pub struct FakeEngine {
    pub url: String,
    pub iii: Arc<IIIClient>,
    state: Arc<State>,
    task: JoinHandle<()>,
}

impl FakeEngine {
    /// An engine without observe (0.24): no `workerregistered` features.
    pub async fn start() -> Self {
        Self::start_with(false).await
    }

    /// An engine with observe (0.25): advertises `observe.v1`, and the test
    /// plays its observer lifecycle with [`FakeEngine::add_observer`] /
    /// [`FakeEngine::remove_observer`]. An owner delivery (`obs.call`) is
    /// echoed back like any unknown invocation, so its target must be
    /// registered on [`FakeEngine::iii`].
    pub async fn start_with_observe() -> Self {
        Self::start_with(true).await
    }

    async fn start_with(observe: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        state.observe.store(observe, Ordering::SeqCst);
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

    /// Sends `observeradded` for `function_id` with `observer` (an observer
    /// view: `id`, `target`, `args`, `filters` in normal form) and waits for
    /// the owner's `observerack`. Returns the ack's `error`, `None` when the
    /// owner accepted the observer.
    pub async fn add_observer(&self, function_id: &str, observer: Value) -> Option<Value> {
        let id = observer["id"].as_str().expect("observer id").to_string();
        // Like the engine: observers of a function are announced once its
        // owner registered it.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let registered = self.state.registered.notified();
                if self.state.functions.lock().unwrap().contains(function_id) {
                    return;
                }
                registered.await;
            }
        })
        .await
        .expect("registerfunction");
        let revision = self.state.revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.push(json!({
            "type": "observeradded",
            "function_id": function_id,
            "revision": revision,
            "observer": observer,
        }));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let acked = self.state.acked.notified();
                if let Some(ack) = self.state.acks.lock().unwrap().get(&id) {
                    if ack["revision"].as_u64() == Some(revision) {
                        return ack.get("error").cloned();
                    }
                }
                acked.await;
            }
        })
        .await
        .expect("observerack")
    }

    /// Sends `observerremoved` for observer `id` of `function_id`.
    pub fn remove_observer(&self, function_id: &str, id: &str) {
        let revision = self.state.revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.push(json!({
            "type": "observerremoved",
            "function_id": function_id,
            "revision": revision,
            "id": id,
        }));
    }

    fn push(&self, frame: Value) {
        let push = self.state.push.lock().unwrap();
        push.as_ref()
            .expect("worker connected")
            .send(frame)
            .expect("worker connection open");
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

    let (push, mut pushed) = mpsc::unbounded_channel();
    // The SDK's telemetry connection (`/otel`) is not the worker's.
    if !path.starts_with("/otel") {
        *state.push.lock().unwrap() = Some(push);
    }
    if state.observe.load(Ordering::SeqCst) {
        let hello = json!({"type": "workerregistered", "worker_id": "fake-worker", "features": ["observe.v1"]});
        if sink
            .send(Message::Text(hello.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }

    loop {
        let reply = tokio::select! {
            message = source.next() => {
                let Some(Ok(message)) = message else { break };
                let Message::Text(text) = message else {
                    continue;
                };
                reply_to(serde_json::from_str(&text).unwrap(), &state).await
            }
            Some(frame) = pushed.recv() => Some(frame),
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

async fn reply_to(frame: Value, state: &State) -> Option<Value> {
    match frame["type"].as_str().unwrap_or_default() {
        "registertriggertype" => {
            if frame["id"] == "http" {
                state.providers.fetch_add(1, Ordering::SeqCst);
            }
            None
        }
        "registerfunction" => {
            let id = frame["id"].as_str().unwrap_or_default().to_string();
            state.functions.lock().unwrap().insert(id);
            state.registered.notify_waiters();
            None
        }
        "registertrigger" if frame["trigger_type"] == "http" => Some(frame),
        "unregistertrigger" | "invocationresult" => Some(frame),
        "observerack" => {
            let id = frame["id"].as_str().unwrap_or_default().to_string();
            state.acks.lock().unwrap().insert(id, frame);
            state.acked.notify_waiters();
            None
        }
        "invokefunction" => {
            let result = match frame["function_id"].as_str().unwrap() {
                "engine::workers::list" => Some(json!({"workers": []})),
                "configuration::get" => Some(json!({"value": state.config.lock().await.clone()})),
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
                // Echoed: the worker serves it (its own functions, and
                // owner deliveries to targets it registered).
                None => frame,
            })
        }
        _ => None,
    }
}
