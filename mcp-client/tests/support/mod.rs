#![allow(dead_code)]

pub mod mcp_mock;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mcp_client::config::RawConfig;
use mcp_client::host::{FunctionDef, Handler, Host, Registration, SecretError, ServerEvent};
use mcp_client::server::{Manager, ManagerOptions, Timings};
use serde_json::{Value, json};

pub const WAIT: Duration = Duration::from_secs(5);

pub const ECHO: &str = "mcp::srv::echo";

/// A tool of a `tools/list` reply with no parameters.
pub fn tool(name: &str) -> Value {
    json!({"name": name, "description": name, "inputSchema": {"type": "object", "properties": {}}})
}

/// The config with one http server `srv` that sends `secret://K` as a bearer token.
pub fn bearer(url: &str) -> Value {
    json!({"mcpServers": {"srv": {"type": "http", "url": url, "auth": {"type": "bearer", "token": "secret://K"}}}})
}

pub fn hi() -> Value {
    json!({"content": [{"type": "text", "text": "hi"}]})
}

/// The ids of the `RegOp::Unregister` ops, in order.
pub fn unregistered(host: &FakeHost) -> Vec<String> {
    host.ops()
        .into_iter()
        .filter_map(|op| match op {
            RegOp::Unregister(id) => Some(id),
            RegOp::Register(_) => None,
        })
        .collect()
}

/// Polls `check` every 10 ms for 5 s, then panics with `what`.
pub async fn eventually<F, Fut>(what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(5);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The production timings scaled down so a test waits milliseconds.
pub fn fast_timings() -> Timings {
    let ms = Duration::from_millis;
    Timings {
        http_handshake: ms(3_000),
        stdio_start: ms(6_000),
        list: ms(3_000),
        call: ms(1_000),
        backoff_min: ms(20),
        backoff_max: ms(200),
        budget_starts: 5,
        budget_window: ms(2_000),
        cooldown: ms(1_000),
        stable_after: ms(600),
        stop_grace: ms(500),
        relist_min_interval: ms(300),
    }
}

pub async fn connect(host: &Arc<FakeHost>, config: Value) -> Arc<Manager> {
    connect_with(host, config, fast_timings()).await
}

pub async fn connect_with(host: &Arc<FakeHost>, config: Value, timings: Timings) -> Arc<Manager> {
    // reqwest panics building a client when no rustls provider is installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let options = ManagerOptions {
        stdio_enabled: true,
        timings,
        worker_cwd: std::env::current_dir().unwrap(),
    };
    let manager = Manager::new(host.clone(), options);
    manager.apply_config(RawConfig::from_value(&config)).await;
    manager
}

/// That server's object of `servers::list`.
pub async fn server(manager: &Arc<Manager>, name: &str) -> Value {
    let list = serde_json::to_value(manager.list().await).unwrap();
    list["servers"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["name"] == name))
        .cloned()
        .unwrap_or_else(|| panic!("no server `{name}` in {list}"))
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegOp {
    Register(String),
    Unregister(String),
}

#[derive(Default)]
struct State {
    /// Keyed by reference text: a vault name `K` lives at `secret://K`.
    secrets: HashMap<String, String>,
    forbidden: HashSet<String>,
    resolves: HashMap<String, usize>,
    /// What the engine lists; `handlers` is what the worker holds.
    live: HashSet<String>,
    defs: HashMap<String, FunctionDef>,
    handlers: HashMap<String, Handler>,
    ops: Vec<RegOp>,
    events: Vec<ServerEvent>,
    port: Option<u16>,
    unreachable: bool,
}

/// An in-memory `Host`. Like the SDK, it panics when a live id is
/// registered again.
pub struct FakeHost {
    state: Arc<Mutex<State>>,
}

struct FakeRegistration {
    id: String,
    state: Arc<Mutex<State>>,
}

impl Registration for FakeRegistration {
    fn unregister(&self) {
        let mut s = self.state.lock().unwrap();
        s.ops.push(RegOp::Unregister(self.id.clone()));
        s.handlers.remove(&self.id);
        s.defs.remove(&self.id);
        s.live.remove(&self.id);
    }
}

fn vault_key(name: &str) -> String {
    format!("secret://{name}")
}

impl FakeHost {
    pub fn new() -> Arc<FakeHost> {
        Arc::new(FakeHost {
            state: Arc::default(),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    pub fn set_secret(&self, reference: &str, value: &str) {
        self.state().secrets.insert(reference.into(), value.into());
    }

    pub fn forbid(&self, reference: &str) {
        self.state().forbidden.insert(reference.into());
    }

    pub fn set_unreachable(&self, on: bool) {
        self.state().unreachable = on;
    }

    /// The vault secret `name` (bare, as `MCP_CLIENT_APPROVALS`).
    pub fn vault(&self, name: &str) -> Option<String> {
        self.state().secrets.get(&vault_key(name)).cloned()
    }

    pub fn resolve_count(&self, reference: &str) -> usize {
        self.state().resolves.get(reference).copied().unwrap_or(0)
    }

    pub fn registered_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.state().live.iter().cloned().collect();
        ids.sort();
        ids
    }

    pub fn def(&self, id: &str) -> Option<FunctionDef> {
        self.state().defs.get(id).cloned()
    }

    pub async fn call(&self, id: &str, payload: Value) -> Result<Value, String> {
        let handler = self.state().handlers.get(id).cloned();
        match handler {
            Some(handler) => handler(payload).await,
            None => Err(format!("function not found: {id}")),
        }
    }

    pub fn ops(&self) -> Vec<RegOp> {
        self.state().ops.clone()
    }

    /// The engine forgets `id` while the worker still holds it: no op is
    /// recorded and the handler stays.
    pub fn drop_registration(&self, id: &str) {
        self.state().live.remove(id);
    }

    pub fn events(&self) -> Vec<ServerEvent> {
        self.state().events.clone()
    }

    pub fn set_http_port(&self, port: Option<u16>) {
        self.state().port = port;
    }
}

#[async_trait::async_trait]
impl Host for FakeHost {
    fn register(&self, def: &FunctionDef, handler: Handler) -> Box<dyn Registration> {
        let mut s = self.state();
        if s.handlers.contains_key(&def.id) {
            // Unlock first so the panic does not poison the state.
            drop(s);
            panic!("already registered: {}", def.id);
        }
        s.ops.push(RegOp::Register(def.id.clone()));
        s.handlers.insert(def.id.clone(), handler);
        s.defs.insert(def.id.clone(), def.clone());
        s.live.insert(def.id.clone());
        Box::new(FakeRegistration {
            id: def.id.clone(),
            state: self.state.clone(),
        })
    }

    async fn registered(&self) -> HashSet<String> {
        self.state().live.clone()
    }

    async fn resolve(&self, reference: &str) -> Result<String, SecretError> {
        let mut s = self.state();
        *s.resolves.entry(reference.into()).or_default() += 1;
        if s.unreachable {
            return Err(SecretError::Unreachable("engine unreachable".into()));
        }
        if s.forbidden.contains(reference) {
            return Err(SecretError::Forbidden);
        }
        s.secrets
            .get(reference)
            .cloned()
            .ok_or(SecretError::NotFound)
    }

    async fn compare_and_set(
        &self,
        name: &str,
        expected: Option<&str>,
        value: &str,
    ) -> Result<bool, SecretError> {
        let mut s = self.state();
        if s.unreachable {
            return Err(SecretError::Unreachable("engine unreachable".into()));
        }
        let key = vault_key(name);
        if s.secrets.get(&key).map(String::as_str) != expected {
            return Ok(false);
        }
        s.secrets.insert(key, value.into());
        Ok(true)
    }

    fn emit(&self, event: ServerEvent) {
        self.state().events.push(event);
    }

    async fn http_port(&self) -> Option<u16> {
        self.state().port
    }
}
