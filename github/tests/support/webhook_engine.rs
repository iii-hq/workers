//! Loopback test transport, based on http/tests/common/fake_engine.rs.
//! Only engine infrastructure is emulated. Registered function/trigger handlers
//! are dispatched through their real SDK connections, including namespaces.
use futures::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};
use tokio_tungstenite::tungstenite::{
    handshake::server::{Request, Response},
    Message,
};

type Sender = mpsc::UnboundedSender<Value>;
type Routes = HashMap<(String, String), Sender>;
struct Channel {
    tx: mpsc::UnboundedSender<Message>,
    rx: Option<mpsc::UnboundedReceiver<Message>>,
}
#[derive(Default)]
struct State {
    functions: Mutex<Routes>,
    providers: Mutex<Routes>,
    pending: Mutex<HashMap<String, Sender>>,
    registrations: Mutex<HashMap<String, Sender>>,
    channels: Mutex<HashMap<String, Channel>>,
    frames: Mutex<Vec<Value>>,
    bytes: Mutex<Vec<Vec<u8>>>,
}

pub struct Engine {
    pub url: String,
    state: Arc<State>,
    clients: Vec<Arc<IIIClient>>,
    task: JoinHandle<()>,
}
impl Engine {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let (stream, _) = incoming.unwrap();
                        connections.spawn(connection(stream, shared.clone()));
                    }
                    Some(done) = connections.join_next() => { done.unwrap(); }
                }
            }
        });
        Self {
            url,
            state,
            clients: Vec::new(),
            task,
        }
    }
    pub async fn client(&mut self, namespace: &str) -> Arc<IIIClient> {
        let iii = Arc::new(register_worker(
            &self.url,
            InitOptions {
                namespace: Some(namespace.into()),
                headers: Some(HashMap::from([(
                    "x-test-namespace".into(),
                    namespace.into(),
                )])),
                ..Default::default()
            },
        ));
        wait(|| iii.get_connection_state() == iii_sdk::runtime::IIIConnectionState::Connected)
            .await;
        self.clients.push(iii.clone());
        iii
    }
    pub fn frames(&self) -> Vec<Value> {
        self.state.frames.lock().unwrap().clone()
    }
    pub fn bytes(&self) -> Vec<Vec<u8>> {
        self.state.bytes.lock().unwrap().clone()
    }
    pub async fn registered(&self, trigger_type: &str, function: &str) {
        wait(|| {
            self.frames().iter().any(|f| {
                f["type"] == "triggerregistrationresult"
                    && f["trigger_type"] == trigger_type
                    && f["function_id"] == function
                    && f["error"].is_null()
            })
        })
        .await;
    }
    pub async fn function(&self, namespace: &str, function: &str) {
        wait(|| {
            self.state
                .functions
                .lock()
                .unwrap()
                .contains_key(&(namespace.into(), function.into()))
        })
        .await;
    }
    pub async fn shutdown(mut self) {
        for client in self.clients.drain(..) {
            tokio::task::spawn_blocking(move || client.shutdown())
                .await
                .unwrap();
        }
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        // shutdown() joins explicitly; this also handles a panicking assertion.
        for client in &self.clients {
            client.shutdown();
        }
        self.task.abort();
    }
}

pub async fn wait(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("local test condition did not become ready");
}
fn response(frame: &Value, result: Value) -> Value {
    json!({"type":"invocationresult", "invocation_id":frame["invocation_id"],
        "function_id":frame["function_id"], "result":result})
}
fn error(frame: &Value, message: &str) -> Value {
    json!({"type":"invocationresult", "invocation_id":frame["invocation_id"],
        "function_id":frame["function_id"], "error":{"code":"FUNCTION_NOT_FOUND", "message":message}})
}

#[expect(clippy::result_large_err)]
async fn connection(stream: TcpStream, state: Arc<State>) {
    let handshake = Arc::new(Mutex::new((String::new(), String::new())));
    let capture = handshake.clone();
    let ws = tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, res: Response| {
        *capture.lock().unwrap() = (
            req.uri().to_string(),
            req.headers()
                .get("x-test-namespace")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("default")
                .to_owned(),
        );
        Ok(res)
    })
    .await
    .unwrap();
    let (path, namespace) = handshake.lock().unwrap().clone();
    let (mut sink, mut source) = ws.split();
    if let Some(rest) = path.strip_prefix("/ws/channels/") {
        let id = rest.split('?').next().unwrap();
        if path.contains("dir=write") {
            let tx = state.channels.lock().unwrap().get(id).unwrap().tx.clone();
            let mut bytes = Vec::new();
            while let Some(Ok(message)) = source.next().await {
                if let Message::Binary(data) = &message {
                    bytes.extend_from_slice(data);
                }
                let done = message.is_close();
                if tx.send(message).is_err() || done {
                    break;
                }
            }
            state.bytes.lock().unwrap().push(bytes);
        } else {
            let mut rx = state
                .channels
                .lock()
                .unwrap()
                .get_mut(id)
                .unwrap()
                .rx
                .take()
                .unwrap();
            while let Some(message) = rx.recv().await {
                let done = message.is_close();
                if sink.send(message).await.is_err() || done {
                    break;
                }
            }
            state.channels.lock().unwrap().remove(id);
        }
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    loop {
        tokio::select! {
            outbound = rx.recv() => {
                let Some(frame) = outbound else { break; };
                if sink.send(Message::Text(frame.to_string().into())).await.is_err() { break; }
            }
            incoming = source.next() => {
                let frame: Value = match incoming {
                    Some(Ok(Message::Text(text))) => serde_json::from_str(&text).unwrap(),
                    Some(Ok(Message::Ping(data))) => {
                        if sink.send(Message::Pong(data)).await.is_err() { break; }
                        continue;
                    }
                    Some(Ok(Message::Pong(_))) => continue,
                    _ => break,
                };
                state.frames.lock().unwrap().push(frame.clone());
                route(frame, &namespace, &tx, &state);
            }
        }
    }
}
fn route(frame: Value, namespace: &str, tx: &Sender, state: &State) {
    let text = |field: &str| frame[field].as_str().unwrap_or_default().to_owned();
    match text("type").as_str() {
        "registerfunction" => {
            state
                .functions
                .lock()
                .unwrap()
                .insert((namespace.into(), text("id")), tx.clone());
        }
        "registertriggertype" => {
            state
                .providers
                .lock()
                .unwrap()
                .insert((namespace.into(), text("id")), tx.clone());
        }
        "registertrigger" => {
            let provider_ns = frame["trigger_namespace"].as_str().unwrap_or(namespace);
            let provider = state
                .providers
                .lock()
                .unwrap()
                .get(&(provider_ns.into(), text("trigger_type")))
                .cloned();
            let provider = provider.expect("trigger provider must register before binding");
            state
                .registrations
                .lock()
                .unwrap()
                .insert(text("id"), tx.clone());
            provider.send(frame).unwrap();
        }
        "triggerregistrationresult" => {
            assert!(frame["error"].is_null(), "registration rejected: {frame}");
            if let Some(target) = state.registrations.lock().unwrap().remove(&text("id")) {
                let _ = target.send(frame);
            }
        }
        "invocationresult" => {
            if let Some(target) = state.pending.lock().unwrap().remove(&text("invocation_id")) {
                let _ = target.send(frame);
            }
        }
        "invokefunction" => {
            let builtin = match text("function_id").as_str() {
                "engine::workers::list" => Some(json!({"workers":[]})),
                "engine::workers::register" => Some(json!({})),
                "engine::telemetry::config" => Some(json!({"enabled":false})),
                "engine::channels::create" => {
                    let id = uuid::Uuid::new_v4().to_string();
                    let (sender, receiver) = mpsc::unbounded_channel();
                    state.channels.lock().unwrap().insert(
                        id.clone(),
                        Channel {
                            tx: sender,
                            rx: Some(receiver),
                        },
                    );
                    Some(
                        json!({"writer":{"channel_id":id,"access_key":"local","direction":"write"},
                        "reader":{"channel_id":id,"access_key":"local","direction":"read"}}),
                    )
                }
                _ => None,
            };
            if let Some(value) = builtin {
                let _ = tx.send(response(&frame, value));
                return;
            }
            let ns = frame["namespace"].as_str().unwrap_or(namespace);
            let target = state
                .functions
                .lock()
                .unwrap()
                .get(&(ns.into(), text("function_id")))
                .cloned();
            if let Some(target) = target {
                if frame["invocation_id"].is_string() {
                    state
                        .pending
                        .lock()
                        .unwrap()
                        .insert(text("invocation_id"), tx.clone());
                }
                let _ = target.send(frame);
            } else {
                let _ = tx.send(error(&frame, "unregistered test function/namespace"));
            }
        }
        "ping" => {
            let _ = tx.send(json!({"type":"pong"}));
        }
        _ => {}
    }
}
