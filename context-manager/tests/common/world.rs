//! Per-scenario World.
//!
//! Carries two stacks side by side:
//!
//! - **@pure**: a fresh production stack per scenario — the real
//!   `handle` functions over real core logic, with deterministic fakes
//!   behind the ports (scripted router catalog, scripted summariser,
//!   in-memory atomic lease store, settable clock). No engine; counts,
//!   placeholders, and lease timestamps are exact.
//! - **@engine**: the shared engine connection (`None` when
//!   unreachable, so `@engine` steps soft-skip) driving the same
//!   handlers registered in-process with the *production* adapters.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use iii_sdk::{TriggerRequest, III};
use serde_json::Value;

use context_manager::config::WorkerConfig;
use context_manager::error::ContextError;
use context_manager::functions::{assemble, compact, count_tokens, prune};
use context_manager::ports::Deps;

use super::fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore};

#[derive(World)]
#[world(init = Self::new)]
pub struct ContextWorld {
    /// @pure stack, rebuilt before every scenario.
    pub config: WorkerConfig,
    pub resolver: Arc<FakeModelResolver>,
    pub summarizer: Arc<FakeSummarizer>,
    pub leases: Arc<InMemoryLeaseStore>,
    pub clock: Arc<FakeClock>,

    /// The history the Given steps build, as raw JSON messages.
    pub messages: Vec<Value>,
    /// Named model inputs (`ModelInput` JSON) the call steps reference.
    pub model_inputs: HashMap<String, Value>,
    /// Monotonic timestamp for built messages.
    pub next_ts: i64,

    /// Outcome of the most recent call (pure or engine).
    pub last_response: Option<Value>,
    pub last_error: Option<String>,
    /// Set when an engine-dependent step soft-skipped (no engine
    /// reachable); generic assertion steps then pass vacuously.
    pub soft_skipped: bool,

    /// Alias -> value used by `${X}` substitution in docstrings.
    pub aliases: HashMap<String, String>,

    /// Shared engine connection; `None` soft-skips @engine steps.
    pub engine: Option<Arc<III>>,
}

impl std::fmt::Debug for ContextWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextWorld")
            .field("engine_connected", &self.engine.is_some())
            .field("messages", &self.messages.len())
            .field("last_error", &self.last_error)
            .finish()
    }
}

impl ContextWorld {
    pub fn new() -> Self {
        Self {
            config: WorkerConfig::default(),
            resolver: Arc::new(FakeModelResolver::new()),
            summarizer: Arc::new(FakeSummarizer::new()),
            leases: Arc::new(InMemoryLeaseStore::new()),
            clock: Arc::new(FakeClock::new()),
            messages: Vec::new(),
            model_inputs: HashMap::new(),
            next_ts: 1_000,
            last_response: None,
            last_error: None,
            soft_skipped: false,
            aliases: HashMap::new(),
            engine: None,
        }
    }

    /// Fresh pure stack + cleared bookkeeping. Runs before every
    /// scenario.
    pub fn reset_pure(&mut self) {
        *self = Self::new();
    }

    /// Build the production `Deps` over the current fakes + config.
    pub fn deps(&self) -> Deps {
        Deps {
            config: self.config.clone(),
            resolver: self.resolver.clone(),
            summarizer: self.summarizer.clone(),
            leases: self.leases.clone(),
            clock: self.clock.clone(),
        }
    }

    /// Mark the scenario as engine-dependent-but-skipped. Returns true
    /// (and records the skip) when no engine is reachable.
    pub fn skip_without_engine(&mut self) -> bool {
        if self.engine.is_none() {
            self.soft_skipped = true;
            return true;
        }
        false
    }

    /// Next message timestamp (monotonic).
    pub fn tick(&mut self) -> i64 {
        self.next_ts += 1;
        self.next_ts
    }

    /// The `ModelInput` JSON registered under `name`, defaulting to a
    /// bare `{ "id": name }`.
    pub fn model_input(&self, name: &str) -> Value {
        self.model_inputs
            .get(name)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "id": name }))
    }

    /// Replace `${alias}` placeholders; `${messages}` expands to the
    /// built history as a JSON array.
    pub fn substitute(&self, text: &str) -> String {
        let mut out = text.to_string();
        if out.contains("${messages}") {
            let rendered =
                serde_json::to_string(&self.messages).expect("history serializes to JSON");
            out = out.replace("${messages}", &rendered);
        }
        for (alias, value) in &self.aliases {
            out = out.replace(&format!("${{{alias}}}"), value);
        }
        out
    }

    /// Drive a `context::*` function through the production handler
    /// (@pure). Mirrors the bus contract: serde rejects malformed
    /// requests, handler errors carry `code: message`.
    pub async fn call_pure(&mut self, function: &str, payload: Value) {
        let deps = self.deps();
        let result = dispatch(&deps, function, payload).await;
        self.record_outcome(result);
    }

    /// Drive a function over the live engine (@engine).
    pub async fn call_engine(&mut self, function: &str, payload: Value) {
        let Some(engine) = self.engine.clone() else {
            self.soft_skipped = true;
            return;
        };
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            engine.trigger(TriggerRequest {
                function_id: function.to_string(),
                payload,
                action: None,
                timeout_ms: Some(12_000),
            }),
        )
        .await;
        let outcome = match result {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("engine trigger timed out".to_string()),
        };
        self.record_outcome(outcome);
    }

    fn record_outcome(&mut self, result: Result<Value, String>) {
        match result {
            Ok(v) => {
                self.last_response = Some(v);
                self.last_error = None;
            }
            Err(e) => {
                self.last_response = None;
                self.last_error = Some(e);
            }
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(payload: Value) -> Result<T, String> {
    serde_json::from_value(payload).map_err(|e| format!("invalid request: {e}"))
}

fn out<T: serde::Serialize>(result: Result<T, ContextError>) -> Result<Value, String> {
    result
        .map(|v| serde_json::to_value(v).expect("responses always serialize"))
        .map_err(|e| e.to_string())
}

/// Dispatch by function id to the same `handle` functions the binary
/// registers — the full production path minus the WebSocket.
pub async fn dispatch(deps: &Deps, function: &str, payload: Value) -> Result<Value, String> {
    match function {
        "context::assemble" => out(assemble::handle(deps, parse(payload)?).await),
        "context::compact" => out(compact::handle(deps, parse(payload)?).await),
        "context::prune" => out(prune::handle(deps, parse(payload)?).await),
        "context::count_tokens" => out(count_tokens::handle(deps, parse(payload)?).await),
        other => Err(format!("unknown function {other}")),
    }
}
