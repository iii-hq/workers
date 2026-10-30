//! Loopback test transport, based on http/tests/common/fake_engine.rs.
//! Only engine infrastructure is emulated: registration and routing by
//! namespace, the observer lifecycle (`observe` → `observeradded` →
//! `observerack` → `observerstatechanged`, and `unobserve`), calls through
//! an observer, which reach its stored target with its metadata, and a
//! departed connection: the observers of the functions it hosted go
//! `pending` until a new host registers them, and the observers it made are
//! `suspended` and disposed, at once or, after [`Engine::hold_departed`], when
//! the test ends the grace period with [`Engine::release_departed`]. An
//! `observe` naming the id of a suspended observer replaces it, as the
//! engine does for a worker back on a new connection: the function's host
//! hears `observerremoved` for the suspended one and `observeradded` for the
//! new one. An id another live connection holds is refused with
//! `OBSERVER_ID_CONFLICT`, and keeps nothing for the refused connection: an
//! `unobserve` from any connection but the observing one is ignored.
//! Registered function handlers and observer hooks run on their real SDK
//! connections.
use futures::{SinkExt, StreamExt};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
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
/// `(namespace, function_id)`.
type Key = (String, String);
struct Channel {
    tx: mpsc::UnboundedSender<Message>,
    rx: Option<mpsc::UnboundedReceiver<Message>>,
}
/// A registered function: the connection that hosts it and its
/// `registerfunction` frame.
struct Host {
    tx: Sender,
    registration: Value,
}
/// One observer, as the engine keeps it.
struct Observer {
    function: Key,
    target: Key,
    /// What the function's host sees (`observeradded.observer`).
    view: Value,
    metadata: Option<Value>,
    observing: Sender,
    state: &'static str,
    /// Whether `observeradded` went to the function's host.
    announced: bool,
}
#[derive(Default)]
struct State {
    functions: Mutex<HashMap<Key, Host>>,
    observers: Mutex<HashMap<String, Observer>>,
    revision: AtomicU64,
    pending: Mutex<HashMap<String, Sender>>,
    channels: Mutex<HashMap<String, Channel>>,
    /// Every frame a worker sent.
    frames: Mutex<Vec<Value>>,
    /// Every invocation the engine forwarded to a host.
    dispatched: Mutex<Vec<Value>>,
    bytes: Mutex<Vec<Vec<u8>>>,
    /// Keep a departed connection's observers until `release_departed`.
    hold_departed: AtomicBool,
    /// Observers of departed connections, kept for their grace period.
    held: Mutex<Vec<String>>,
    /// Connections that ended.
    departures: AtomicUsize,
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
    pub fn dispatched(&self) -> Vec<Value> {
        self.state.dispatched.lock().unwrap().clone()
    }
    pub fn bytes(&self) -> Vec<Vec<u8>> {
        self.state.bytes.lock().unwrap().clone()
    }
    /// From now on, the observers a departed connection made stay
    /// `suspended` (the engine's grace period) until [`Self::release_departed`].
    pub fn hold_departed(&self) {
        self.state.hold_departed.store(true, Ordering::SeqCst);
    }
    /// Ends the grace period: disposes every held observer, and disposes the
    /// observers of later departures at once.
    pub fn release_departed(&self) {
        self.state.hold_departed.store(false, Ordering::SeqCst);
        let held = std::mem::take(&mut *self.state.held.lock().unwrap());
        for id in held {
            dispose(&self.state, &id);
        }
    }
    /// How many connections ended so far.
    pub fn departures(&self) -> usize {
        self.state.departures.load(Ordering::SeqCst)
    }
    /// Waits until an observer of `function` with target `target` is active:
    /// its host acked it.
    pub async fn observed(&self, function: &str, target: &str) {
        wait(|| {
            self.state
                .observers
                .lock()
                .unwrap()
                .values()
                .any(|o| o.function.1 == function && o.target.1 == target && o.state == "active")
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
/// Answers `frame` with an error; a fire-and-forget call gets no answer.
fn fail(tx: &Sender, frame: &Value, code: &str, message: &str) {
    if frame["invocation_id"].is_string() {
        let _ = tx.send(
            json!({"type":"invocationresult", "invocation_id":frame["invocation_id"],
            "function_id":frame["function_id"], "error":{"code":code, "message":message}}),
        );
    }
}
fn state_changed(id: &str, state: &str, reason: Option<Value>) -> Value {
    json!({"type":"observerstatechanged", "id":id, "state":state, "reason":reason, "fires":0})
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
    // An engine with observe v2: the SDK registers private functions, args
    // formats and deliver hooks only after this hello.
    let hello = json!({"type":"workerregistered", "worker_id":"test-worker",
        "features":["observe.v1", "observe.v2"]});
    if sink
        .send(Message::Text(hello.to_string().into()))
        .await
        .is_err()
    {
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
    departed(&state, &tx);
}

/// The connection `tx` ended. Its functions are unregistered and their
/// observers go `pending` until a new host registers them; the observers it
/// made are `suspended`, and disposed at once unless departures are held.
fn departed(state: &State, tx: &Sender) {
    let gone: Vec<Key> = {
        let mut functions = state.functions.lock().unwrap();
        let gone: Vec<Key> = functions
            .iter()
            .filter(|(_, host)| host.tx.same_channel(tx))
            .map(|(key, _)| key.clone())
            .collect();
        for key in &gone {
            functions.remove(key);
        }
        gone
    };
    let made: Vec<String> = {
        let mut observers = state.observers.lock().unwrap();
        for (id, observer) in observers.iter_mut() {
            if gone.contains(&observer.function) && observer.state != "error" {
                observer.state = "pending";
                observer.announced = false;
                let _ = observer.observing.send(state_changed(id, "pending", None));
            }
        }
        observers
            .iter_mut()
            .filter(|(_, observer)| observer.observing.same_channel(tx))
            .map(|(id, observer)| {
                observer.state = "suspended";
                id.clone()
            })
            .collect()
    };
    if state.hold_departed.load(Ordering::SeqCst) {
        state.held.lock().unwrap().extend(made);
    } else {
        for id in made {
            dispose(state, &id);
        }
    }
    state.departures.fetch_add(1, Ordering::SeqCst);
}

/// A new connection observes under the id of the suspended observer `id`:
/// the suspended one leaves the grace period and is disposed, so the new one
/// takes its place.
fn replace_suspended(state: &State, id: &str) {
    state.held.lock().unwrap().retain(|held| held != id);
    dispose(state, id);
}

/// Removes the observer `id`. Its function's host hears `observerremoved` if
/// it accepted the observer; its observing connection hears `disposed`.
fn dispose(state: &State, id: &str) {
    let Some(observer) = state.observers.lock().unwrap().remove(id) else {
        return;
    };
    let host = state
        .functions
        .lock()
        .unwrap()
        .get(&observer.function)
        .map(|h| h.tx.clone());
    if let (true, Some(host)) = (observer.announced && observer.state != "error", host) {
        let revision = state.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = host.send(json!({"type":"observerremoved",
            "function_id":observer.function.1, "namespace":observer.function.0,
            "revision":revision, "id":id}));
    }
    let reason = json!({"code":"UNOBSERVED", "message":"disposed"});
    let _ = observer
        .observing
        .send(state_changed(id, "disposed", Some(reason)));
}

/// Sends `observeradded` to the host of `function` for each of its observers
/// not announced yet. Like the engine, it refuses args to a function that
/// declares no `args_format` (`INVALID_ARGS`) and a private target
/// (`FORBIDDEN`).
fn announce(state: &State, function: &Key) {
    let (host, private_targets) = {
        let functions = state.functions.lock().unwrap();
        let host = functions
            .get(function)
            .map(|h| (h.tx.clone(), !h.registration["args_format"].is_null()));
        let private: Vec<Key> = functions
            .iter()
            .filter(|(_, h)| h.registration["private"] == true)
            .map(|(k, _)| k.clone())
            .collect();
        (host, private)
    };
    let Some((host, takes_args)) = host else {
        return;
    };
    let mut observers = state.observers.lock().unwrap();
    for (id, observer) in observers.iter_mut() {
        if observer.function != *function || observer.announced || observer.state != "pending" {
            continue;
        }
        let has_args = observer.view["args"]
            .as_object()
            .is_some_and(|args| !args.is_empty());
        let refusal = if has_args && !takes_args {
            Some(("INVALID_ARGS", "the function takes no args"))
        } else if private_targets.contains(&observer.target) {
            Some(("FORBIDDEN", "a private function cannot be a target"))
        } else {
            None
        };
        if let Some((code, message)) = refusal {
            observer.state = "error";
            let reason = json!({"code":code, "message":message});
            let _ = observer
                .observing
                .send(state_changed(id, "error", Some(reason)));
            continue;
        }
        observer.announced = true;
        let revision = state.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = host.send(json!({"type":"observeradded", "function_id":function.1,
            "namespace":function.0, "revision":revision, "observer":observer.view}));
    }
}

/// Forwards an invocation to `host`, as the engine does: answers return to
/// `caller`, and the engine's transport metadata is injected.
fn forward(state: &State, mut frame: Value, caller: &Sender, host: &Sender) {
    if let Some(invocation_id) = frame["invocation_id"].as_str() {
        state
            .pending
            .lock()
            .unwrap()
            .insert(invocation_id.to_owned(), caller.clone());
    }
    if let Some(data) = frame["data"].as_object_mut() {
        data.insert("_caller_worker_id".into(), json!("test-worker"));
    }
    state.dispatched.lock().unwrap().push(frame.clone());
    let _ = host.send(frame);
}

fn route(frame: Value, namespace: &str, tx: &Sender, state: &State) {
    let text = |field: &str| frame[field].as_str().unwrap_or_default().to_owned();
    match text("type").as_str() {
        "registerfunction" => {
            let key = (namespace.to_owned(), text("id"));
            state.functions.lock().unwrap().insert(
                key.clone(),
                Host {
                    tx: tx.clone(),
                    registration: frame.clone(),
                },
            );
            // Observers of a function are announced once its host registers it.
            announce(state, &key);
        }
        "observe" => {
            let source = &frame["source"];
            let target = &frame["target"];
            let function = (
                source["namespace"].as_str().unwrap_or(namespace).to_owned(),
                source["function_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
            let target = (
                target["namespace"].as_str().unwrap_or(namespace).to_owned(),
                target["function_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
            let id = text("id");
            let existing = state
                .observers
                .lock()
                .unwrap()
                .get(&id)
                .map(|o| (o.observing.same_channel(tx), o.state));
            match existing {
                Some((false, "suspended")) => replace_suspended(state, &id),
                Some((false, _)) => {
                    let reason = json!({"code":"OBSERVER_ID_CONFLICT",
                        "message":format!("observer `{id}` is owned by another connection")});
                    let _ = tx.send(json!({"type":"observeresult", "id":id, "state":"error",
                        "error":reason}));
                    return;
                }
                _ => {}
            }
            let mut view = json!({"id":id, "args":source["args"], "filters":source["filters"],
                "target":target.1});
            for field in ["capture", "on"] {
                if !source[field].is_null() {
                    view[field] = source[field].clone();
                }
            }
            state.observers.lock().unwrap().insert(
                id.clone(),
                Observer {
                    function: function.clone(),
                    target,
                    view,
                    metadata: frame.get("metadata").filter(|m| !m.is_null()).cloned(),
                    observing: tx.clone(),
                    state: "pending",
                    announced: false,
                },
            );
            let _ = tx.send(json!({"type":"observeresult", "id":id, "state":"pending"}));
            announce(state, &function);
        }
        "observerack" => {
            let id = text("id");
            let mut observers = state.observers.lock().unwrap();
            if let Some(observer) = observers.get_mut(&id) {
                let refused = frame.get("error").filter(|e| !e.is_null()).cloned();
                observer.state = if refused.is_some() { "error" } else { "active" };
                let _ = observer
                    .observing
                    .send(state_changed(&id, observer.state, refused));
            }
        }
        "unobserve" => {
            // Only the observing connection disposes its observer this way;
            // the engine ignores, and does not answer, anyone else's.
            let id = text("id");
            let owned = state
                .observers
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|o| o.observing.same_channel(tx));
            if owned {
                dispose(state, &id);
            }
        }
        "invocationresult" => {
            if let Some(target) = state.pending.lock().unwrap().remove(&text("invocation_id")) {
                // The engine activates `observed` itself and strips it.
                let mut frame = frame;
                if let Some(result) = frame.as_object_mut() {
                    result.remove("observed");
                }
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
            if let Some(observer_id) = frame["activation"]["observer_id"].as_str() {
                // A call through an observer: only its function's host may make
                // it, and it reaches the observer's stored target with the
                // observer's metadata.
                let observer = state.observers.lock().unwrap().get(observer_id).map(|o| {
                    (
                        o.function.clone(),
                        o.target.clone(),
                        o.metadata.clone(),
                        o.state,
                    )
                });
                let Some((function, target, metadata, observer_state)) = observer else {
                    fail(tx, &frame, "OBSERVER_NOT_FOUND", "no such observer");
                    return;
                };
                if observer_state != "active" {
                    fail(tx, &frame, "OBSERVER_NOT_ACTIVE", "observer not active");
                    return;
                }
                let (owner, host) = {
                    let functions = state.functions.lock().unwrap();
                    (
                        functions
                            .get(&function)
                            .is_some_and(|h| h.tx.same_channel(tx)),
                        functions.get(&target).map(|h| h.tx.clone()),
                    )
                };
                if !owner {
                    fail(tx, &frame, "NOT_FUNCTION_OWNER", "not the host");
                    return;
                }
                let Some(host) = host else {
                    fail(tx, &frame, "TARGET_UNAVAILABLE", "target not registered");
                    return;
                };
                let mut frame = frame.clone();
                frame["function_id"] = json!(target.1);
                frame["namespace"] = json!(target.0);
                frame["metadata"] = metadata.unwrap_or(Value::Null);
                frame["activation"] = json!({"observer_id":observer_id,
                    "activation_id":uuid::Uuid::new_v4().to_string()});
                forward(state, frame, tx, &host);
                return;
            }
            let ns = frame["namespace"].as_str().unwrap_or(namespace).to_owned();
            let host = state
                .functions
                .lock()
                .unwrap()
                .get(&(ns, text("function_id")))
                .map(|h| (h.tx.clone(), h.registration["private"] == true));
            match host {
                Some((host, true)) if !host.same_channel(tx) => {
                    fail(tx, &frame, "FORBIDDEN", "private function");
                }
                Some((host, _)) => forward(state, frame.clone(), tx, &host),
                None => {
                    fail(
                        tx,
                        &frame,
                        "FUNCTION_NOT_FOUND",
                        "unregistered test function/namespace",
                    );
                }
            }
        }
        "ping" => {
            let _ = tx.send(json!({"type":"pong"}));
        }
        _ => {}
    }
}
