//! MOT-5569: pre_generate hook text inside the turn loop's budget. Drives the
//! production send and step handlers over an isolated mock SDK transport (no
//! engine, model or provider): static `inject_prompt` text must reach
//! `context::assemble`, and the dynamic hooks' measured text must size the
//! next request's reserve, across turns too.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use harness::config::WorkerConfig;
use harness::deps::Deps;
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::tungstenite::Message;

const STATIC: &str = "STATIC fp guidance";
const DYNAMIC: &str = "DYNAMIC code-runner guidance";
/// What `context::count-tokens` charges for the dynamic hook's text: more
/// than the flat 256 reserve, as code-runner's guidance is live.
const DYNAMIC_TOKENS: u64 = 900;
const BASE_TOKENS: u64 = 1_000;

#[derive(Default)]
struct Store {
    state: BTreeMap<(String, String), Value>,
    messages: Vec<Value>,
    calls: Vec<(String, Value)>,
}

impl Store {
    fn respond(&mut self, function: &str, data: &Value) -> Result<Value, String> {
        self.calls.push((function.into(), data.clone()));
        let scope = data["scope"].as_str().unwrap_or_default().to_string();
        let key = data["key"].as_str().unwrap_or_default().to_string();
        match function {
            "state::set" | "harness::state::set" => {
                self.state.insert((scope, key), data["value"].clone());
                Ok(json!({}))
            }
            "state::get" | "harness::state::get" => Ok(self
                .state
                .get(&(scope, key))
                .cloned()
                .unwrap_or(Value::Null)),
            "state::delete" | "harness::state::delete" => {
                self.state.remove(&(scope, key));
                Ok(json!({}))
            }
            "state::list" | "harness::state::list" => Ok(json!([])),
            "state::list_keys" => Ok(json!({"keys": []})),
            "harness::state::compare-and-set" => {
                let old = self
                    .state
                    .get(&(scope.clone(), key.clone()))
                    .cloned()
                    .unwrap_or(Value::Null);
                let swapped = old == data["expected"];
                if swapped {
                    self.state.insert((scope, key), data["value"].clone());
                }
                Ok(json!({"swapped": swapped, "current": old}))
            }
            "session::get" => Ok(json!({"meta": {"session_id": "s", "metadata": {}}})),
            "session::ensure" => {
                Ok(json!({"meta": {"session_id": "s", "metadata": {}}, "created": false}))
            }
            "session::messages" => Ok(json!({"messages": self.messages})),
            "session::append" => {
                let entry_id = match &data["entry_id"] {
                    Value::Null => json!(format!("e{}", self.messages.len())),
                    id => id.clone(),
                };
                self.messages.push(json!({
                    "entry_id": entry_id,
                    "message": data["message"],
                    "custom": data["custom"],
                }));
                Ok(json!({"entry_id": entry_id}))
            }
            "session::set-status" => Ok(json!({"ok": true})),
            // The step is driven by hand instead.
            "harness::turn" => Ok(json!({"message_receipt_id": "receipt"})),
            "context::assemble" => Ok(json!({
                "system_prompt": data["system_prompt"],
                "messages": data["messages"],
                "token_count": BASE_TOKENS + data["options"]["request_overhead_tokens"].as_u64().unwrap(),
                "usable": 100_000,
                "effective_max_output_tokens": 1_000,
            })),
            "context::count-tokens" => {
                let prompt = data["system_prompt"].as_str().unwrap_or_default();
                let hook = if prompt.contains(DYNAMIC) {
                    DYNAMIC_TOKENS
                } else {
                    0
                };
                Ok(json!({"tokens": BASE_TOKENS + hook}))
            }
            "test::dynamic-hook" => {
                let prompt = data["generate"]["system_prompt"].as_str().unwrap();
                Ok(json!({"mutations": {"system_prompt": format!("{prompt}\n\n{DYNAMIC}")}}))
            }
            // The step's boundary, the stream channel the provider call opens:
            // everything this test checks is decided and persisted before it.
            "engine::channels::create" => {
                Err("intentional boundary stop before the provider".into())
            }
            _ => Err(format!("unexpected mock RPC {function}")),
        }
    }
}

struct Stack {
    deps: Deps,
    store: Arc<Mutex<Store>>,
    push: mpsc::UnboundedSender<Value>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.deps.iii.shutdown();
        self.server.abort();
    }
}

impl Stack {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let store = Arc::new(Mutex::new(Store::default()));
        let (ready_tx, ready_rx) = oneshot::channel();
        let (push, mut pushed) = mpsc::unbounded_channel::<Value>();
        let state = store.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let _ = ready_tx.send(());
            loop {
                let frame = tokio::select! {
                    Some(message) = pushed.recv() => {
                        socket.send(Message::Text(message.to_string().into())).await.unwrap();
                        continue;
                    }
                    frame = socket.next() => match frame {
                        Some(Ok(frame)) => frame,
                        _ => break,
                    },
                };
                let Ok(message) = serde_json::from_str::<Value>(frame.to_text().unwrap_or(""))
                else {
                    continue;
                };
                if message["type"] != "invokefunction" {
                    continue;
                }
                let function = message["function_id"].as_str().unwrap();
                let result = state.lock().unwrap().respond(function, &message["data"]);
                if message["invocation_id"].is_null() {
                    continue;
                }
                let mut reply = json!({"type": "invocationresult",
                    "invocation_id": message["invocation_id"], "function_id": function});
                match result {
                    Ok(value) => reply["result"] = value,
                    Err(error) => reply["error"] = json!({"code": "test_error", "message": error}),
                }
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let iii = Arc::new(register_worker(&url, InitOptions::default()));
        ready_rx.await.unwrap();
        let cfg = WorkerConfig {
            session_timeout_ms: 2_000,
            ..WorkerConfig::default()
        };
        let deps = Deps::new(
            iii.clone(),
            Arc::new(RwLock::new(Arc::new(cfg))),
            harness::discovery::new_cell(),
            harness::skills::new_cell(),
            harness::events::TurnEvents::register(&iii),
            harness::hooks::HookRegistry::register(&iii),
        );
        Self {
            deps,
            store,
            push,
            server,
        }
    }

    /// Bind a pre_generate hook the way the engine does.
    async fn bind(&self, id: &str, function_id: &str, metadata: Value) {
        let before = self.deps.hooks.pre_generate.ordered().len();
        self.push
            .send(json!({"type": "registertrigger", "id": id,
                "trigger_type": harness::hooks::PRE_GENERATE, "function_id": function_id,
                "config": {}, "metadata": metadata}))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.deps.hooks.pre_generate.ordered().len() == before {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the hook binding registers");
    }

    fn turn(&self) -> Value {
        let store = self.store.lock().unwrap();
        store.state[&("harness_turn".to_string(), "s".to_string())].clone()
    }

    /// Run the turn's current step up to the provider boundary and return
    /// what it sent to `context::assemble` and the final request it counted.
    async fn step(&self) -> (Value, Value) {
        let turn = self.turn();
        let payload = serde_json::from_value(json!({"session_id": "s",
            "turn_id": turn["turn_id"], "step": turn["step"], "depth": 0}))
        .unwrap();
        let mark = self.store.lock().unwrap().calls.len();
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            harness::turn_loop::run_step(&self.deps, payload),
        )
        .await
        .expect("the step reaches the provider boundary");
        let store = self.store.lock().unwrap();
        let sent = |function: &str| {
            store.calls[mark..]
                .iter()
                .find(|(f, _)| f == function)
                .unwrap_or_else(|| panic!("the step calls {function}"))
                .1
                .clone()
        };
        (sent("context::assemble"), sent("context::count-tokens"))
    }

    async fn send(&self, text: &str) {
        let request = serde_json::from_value(json!({"session_id": "s",
            "message": text, "model": "fake", "options": {"system_prompt": "base"}}))
        .unwrap();
        harness::functions::send::start(&self.deps, request)
            .await
            .expect("send seeds a turn");
    }
}

fn overhead(assemble: &Value) -> u64 {
    assemble["options"]["request_overhead_tokens"]
        .as_u64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_hook_text_is_assembled_and_dynamic_hook_text_sizes_the_next_reserve() {
    let stack = Stack::new().await;
    stack
        .bind(
            "static",
            "fp::inject-guidance",
            json!({"inject_prompt": STATIC}),
        )
        .await;
    stack.bind("dynamic", "test::dynamic-hook", json!({})).await;

    stack.send("first").await;
    let (assemble, counted) = stack.step().await;
    let assembled = assemble["system_prompt"].as_str().unwrap();
    assert!(
        assembled.contains(STATIC),
        "assembly budgets the static text: {assembled}"
    );
    assert!(
        !assembled.contains(DYNAMIC),
        "dynamic text lands after assembly"
    );
    let sent = counted["system_prompt"].as_str().unwrap();
    assert_eq!(
        sent.matches(STATIC).count(),
        1,
        "static text is sent once: {sent}"
    );
    assert!(sent.contains(DYNAMIC));
    assert_eq!(
        stack.turn()["pre_generate_hook_tokens"],
        json!(DYNAMIC_TOKENS),
        "the step stores what the dynamic hook added past assembly"
    );
    let first_overhead = overhead(&assemble);

    // A finished turn: the next message seeds a new one from this record.
    {
        let mut store = stack.store.lock().unwrap();
        let key = ("harness_turn".to_string(), "s".to_string());
        let mut turn = store.state[&key].clone();
        turn["status"] = json!("completed");
        store.state.insert(key, turn);
    }
    stack.send("second").await;
    assert_eq!(stack.turn()["step"], 0);
    let (assemble, _) = stack.step().await;
    assert_eq!(
        overhead(&assemble) - first_overhead,
        DYNAMIC_TOKENS - 256,
        "a new turn's first step reserves the measured hook text, not the flat 256"
    );
}
