//! Per-scenario World.
//!
//! Carries two stacks side by side:
//!
//! - **@pure**: a fresh production stack per scenario — real handlers,
//!   real service, real emitter + filters — backed by a real `FsStore`
//!   over a per-scenario tempdir, deterministic `SeqIds` / `FakeClock`,
//!   and a recording deliverer. No engine involved; ids, timestamps and
//!   on-disk files are exact. `reopen_fs` swaps in a fresh `FsStore`
//!   over the same directory (same ids/clock) to simulate a worker
//!   restart.
//! - **@engine**: the shared engine connection (`None` when
//!   unreachable, so `@engine` steps soft-skip), alias bookkeeping for
//!   the real (random) ids, and named in-process bridged-instance
//!   stacks for the bridge propagation scenarios.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use cucumber::World;
use iii_sdk::III;
use serde_json::Value;

use session_manager::config::WorkerConfig;
use session_manager::error::SessionError;
use session_manager::events::{Emitter, TriggerSets};
use session_manager::functions::{
    append, append_many, create, delete, ensure, fork, get, get_message, list, messages,
    set_active_leaf, set_meta, set_status, update_message, Deps,
};
use session_manager::service::SessionService;
use session_manager::store::FsStore;

use super::bridge::BridgeStack;
use super::ids::{FakeClock, SeqIds};
use super::recorder::{Recorder, RecorderDeliverer};

#[derive(World)]
#[world(init = Self::new)]
pub struct SessionWorld {
    /// @pure stack, rebuilt before every scenario.
    pub deps: Arc<Deps>,
    pub emitter: Arc<Emitter>,
    pub recorder: Recorder,
    pub clock: Arc<FakeClock>,
    pub ids: Arc<SeqIds>,
    /// Data directory of the @pure FsStore (inside `_fs_guard`).
    pub fs_dir: PathBuf,
    _fs_guard: Option<tempfile::TempDir>,

    /// Outcome of the most recent call (pure, bridge, or engine).
    pub last_response: Option<Value>,
    pub last_error: Option<String>,
    /// Set when an engine-dependent step soft-skipped (no engine
    /// reachable); generic assertion steps then pass vacuously instead
    /// of panicking about a call that never happened.
    pub soft_skipped: bool,
    /// Outcome of the most recent binding registration attempt.
    pub last_binding_result: Option<Result<(), String>>,

    /// Alias -> value (e.g. "S1" -> a real session id) used by `${X}`
    /// substitution in payload docstrings.
    pub aliases: HashMap<String, String>,

    /// Shared engine connection; `None` soft-skips @engine steps.
    pub engine: Option<Arc<III>>,
    /// Alias -> recorder function id for engine subscribers.
    pub engine_subscribers: HashMap<String, String>,
    /// Named in-process bridged-instance stacks (@engine bridge tests).
    pub bridges: HashMap<String, BridgeStack>,
}

impl std::fmt::Debug for SessionWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionWorld")
            .field("engine_connected", &self.engine.is_some())
            .field("last_error", &self.last_error)
            .finish()
    }
}

impl SessionWorld {
    pub fn new() -> Self {
        let pure = build_pure_stack();
        Self {
            deps: pure.deps,
            emitter: pure.emitter,
            recorder: pure.recorder,
            clock: pure.clock,
            ids: pure.ids,
            fs_dir: pure.fs_dir,
            _fs_guard: Some(pure.guard),
            last_response: None,
            last_error: None,
            soft_skipped: false,
            last_binding_result: None,
            aliases: HashMap::new(),
            engine: None,
            engine_subscribers: HashMap::new(),
            bridges: HashMap::new(),
        }
    }

    /// Fresh pure stack + cleared bookkeeping. Runs before every scenario.
    pub fn reset_pure(&mut self) {
        let pure = build_pure_stack();
        self.deps = pure.deps;
        self.emitter = pure.emitter;
        self.recorder = pure.recorder;
        self.clock = pure.clock;
        self.ids = pure.ids;
        self.fs_dir = pure.fs_dir;
        self._fs_guard = Some(pure.guard);
        self.last_response = None;
        self.last_error = None;
        self.soft_skipped = false;
        self.last_binding_result = None;
        self.aliases.clear();
        self.engine_subscribers.clear();
        self.bridges.clear();
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

    /// Simulate a worker restart: a brand-new `FsStore` (empty cache)
    /// over the same directory, same ids/clock so the sequence
    /// continues, same emitter/bindings/recorder.
    pub fn reopen_fs(&mut self) {
        let store = Arc::new(FsStore::new(&self.fs_dir).expect("reopen FsStore"));
        let service = Arc::new(SessionService::with_parts(
            store,
            self.ids.clone(),
            self.clock.clone(),
            &WorkerConfig::default(),
        ));
        self.deps = Arc::new(Deps {
            service,
            sink: self.emitter.clone(),
        });
    }

    /// Replace `${alias}` placeholders with stashed values.
    pub fn substitute(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (alias, value) in &self.aliases {
            out = out.replace(&format!("${{{alias}}}"), value);
        }
        out
    }

    /// Drive a `session::*` function through the production handler
    /// (@pure). Mirrors the bus contract: serde rejects malformed
    /// requests, handler errors carry `code: message`.
    pub async fn call_pure(&mut self, function: &str, payload: Value) {
        let result = dispatch(&self.deps, function, payload).await;
        self.record_outcome(result);
    }

    /// Drive a function through a named bridged-instance stack
    /// (@engine bridge scenarios).
    pub async fn call_via_bridge(&mut self, instance: &str, function: &str, payload: Value) {
        let deps = self
            .bridges
            .get(instance)
            .unwrap_or_else(|| panic!("no bridged instance named `{instance}`"))
            .deps
            .clone();
        let result = dispatch(&deps, function, payload).await;
        self.record_outcome(result);
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

struct PureStack {
    deps: Arc<Deps>,
    emitter: Arc<Emitter>,
    recorder: Recorder,
    clock: Arc<FakeClock>,
    ids: Arc<SeqIds>,
    fs_dir: PathBuf,
    guard: tempfile::TempDir,
}

fn build_pure_stack() -> PureStack {
    let cfg = WorkerConfig::default();
    let guard = tempfile::tempdir().expect("create scenario tempdir");
    let fs_dir = guard.path().to_path_buf();
    let clock = Arc::new(FakeClock::new());
    let ids = Arc::new(SeqIds::default());
    let store = Arc::new(FsStore::new(&fs_dir).expect("open scenario FsStore"));
    let service = Arc::new(SessionService::with_parts(
        store,
        ids.clone(),
        clock.clone(),
        &cfg,
    ));
    let recorder = Recorder::new();
    let emitter = Arc::new(Emitter::new(
        TriggerSets::new(),
        Arc::new(RecorderDeliverer(recorder.clone())),
    ));
    let deps = Arc::new(Deps {
        service,
        sink: emitter.clone(),
    });
    PureStack {
        deps,
        emitter,
        recorder,
        clock,
        ids,
        fs_dir,
        guard,
    }
}

fn parse<T: serde::de::DeserializeOwned>(payload: Value) -> Result<T, String> {
    serde_json::from_value(payload).map_err(|e| format!("invalid request: {e}"))
}

fn out<T: serde::Serialize>(result: Result<T, SessionError>) -> Result<Value, String> {
    result
        .map(|v| serde_json::to_value(v).expect("responses always serialize"))
        .map_err(|e| e.to_string())
}

/// Dispatch by function id to the same `handle` functions the binary
/// registers — the full production path minus the WebSocket.
pub async fn dispatch(deps: &Deps, function: &str, payload: Value) -> Result<Value, String> {
    match function {
        "session::create" => out(create::handle(deps, parse(payload)?).await),
        "session::ensure" => out(ensure::handle(deps, parse(payload)?).await),
        "session::get" => out(get::handle(deps, parse(payload)?).await),
        "session::list" => out(list::handle(deps, parse(payload)?).await),
        "session::set-meta" => out(set_meta::handle(deps, parse(payload)?).await),
        "session::set-status" => out(set_status::handle(deps, parse(payload)?).await),
        "session::delete" => out(delete::handle(deps, parse(payload)?).await),
        "session::append" => out(append::handle(deps, parse(payload)?).await),
        "session::append-many" => out(append_many::handle(deps, parse(payload)?).await),
        "session::update-message" => out(update_message::handle(deps, parse(payload)?).await),
        "session::messages" => out(messages::handle(deps, parse(payload)?).await),
        "session::get-message" => out(get_message::handle(deps, parse(payload)?).await),
        "session::fork" => out(fork::handle(deps, parse(payload)?).await),
        "session::set-active-leaf" => out(set_active_leaf::handle(deps, parse(payload)?).await),
        other => Err(format!("unknown function {other}")),
    }
}
