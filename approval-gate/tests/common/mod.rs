//! Shared fakes for the approval-gate test suite.
//!
//! Production code goes through `StateBus` and `FunctionExecutor` traits
//! exactly so unit tests can swap in these in-memory fakes. The trait
//! contracts are documented on the production types; the fakes here
//! mirror the wire shape closely enough that any handler behavior tied
//! to bus semantics surfaces in the tests.

#![allow(dead_code)] // Individual test binaries pull in subsets of these.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use approval_gate::{FunctionExecutor, IncomingCall, OrchestratorWaker, StateBus};
use serde_json::{json, Value};

/// Records every invocation and replays a canned response. By default
/// the fake returns `Ok({"ok": true})`; set [`Self::response`] to
/// override.
pub struct FakeExecutor {
    pub calls: Mutex<Vec<(String, Value, String, String)>>,
    pub response: Mutex<Option<Result<Value, String>>>,
}

impl Default for FakeExecutor {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            response: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl FunctionExecutor for FakeExecutor {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        function_call_id: &str,
        session_id: &str,
    ) -> Result<Value, String> {
        self.calls.lock().unwrap().push((
            function_id.to_string(),
            args,
            function_call_id.to_string(),
            session_id.to_string(),
        ));
        self.response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(json!({ "ok": true })))
    }
}

/// In-memory implementation of [`approval_gate::StateBus`]. Keys are
/// `"<scope>/<key>"`; `list_prefix` filters by that flattened prefix
/// (same shape the production iii bus exposes).
pub struct InMemoryStateBus {
    store: Mutex<HashMap<String, Value>>,
}

impl InMemoryStateBus {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl StateBus for InMemoryStateBus {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
        self.store
            .lock()
            .unwrap()
            .insert(format!("{scope}/{key}"), value);
        Ok(())
    }
    async fn get(&self, scope: &str, key: &str) -> Option<Value> {
        self.store
            .lock()
            .unwrap()
            .get(&format!("{scope}/{key}"))
            .cloned()
    }
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
        let map = self.store.lock().unwrap();
        map.iter()
            .filter(|(k, _)| k.starts_with(&format!("{scope}/{prefix}")))
            .map(|(_, v)| v.clone())
            .collect()
    }
    async fn delete(&self, scope: &str, key: &str) -> Result<bool, iii_sdk::IIIError> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .remove(&format!("{scope}/{key}"))
            .is_some())
    }
}

/// `StateBus` wrapper that observes the peak number of concurrent
/// `set()` calls in flight at any moment AND inserts yield points
/// inside `set()` to deterministically widen any check-and-set race
/// window in the handler under test.
///
/// Used by the `concurrent_resolves_*` race test (finding #3): if the
/// per-key mutex around the read→check→write critical section is in
/// place, `peak_concurrent_sets` stays at 1 because the lock
/// serializes the entire window. If the mutex is removed, the yields
/// inside `set()` deterministically overlap the two `InFlight` writes
/// and `peak_concurrent_sets` reaches 2. The test asserts `<=1` so
/// removing the lock makes it fail every run, not just on a lucky
/// scheduler.
pub struct MaxConcurrentSetBus {
    inner: InMemoryStateBus,
    in_flight_sets: AtomicUsize,
    pub peak_concurrent_sets: AtomicUsize,
    pub set_yield_count: usize,
}

impl MaxConcurrentSetBus {
    pub fn new(set_yield_count: usize) -> Self {
        Self {
            inner: InMemoryStateBus::new(),
            in_flight_sets: AtomicUsize::new(0),
            peak_concurrent_sets: AtomicUsize::new(0),
            set_yield_count,
        }
    }
    pub fn peak(&self) -> usize {
        self.peak_concurrent_sets.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl StateBus for MaxConcurrentSetBus {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
        let n = self.in_flight_sets.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_concurrent_sets.fetch_max(n, Ordering::SeqCst);
        for _ in 0..self.set_yield_count {
            tokio::task::yield_now().await;
        }
        let out = self.inner.set(scope, key, value).await;
        self.in_flight_sets.fetch_sub(1, Ordering::SeqCst);
        out
    }
    async fn get(&self, scope: &str, key: &str) -> Option<Value> {
        self.inner.get(scope, key).await
    }
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
        self.inner.list_prefix(scope, prefix).await
    }
    async fn delete(&self, scope: &str, key: &str) -> Result<bool, iii_sdk::IIIError> {
        self.inner.delete(scope, key).await
    }
}

/// `StateBus` whose `set` always errors. Used to exercise the gate's
/// fail-closed behavior on transient kv outages.
pub struct FailingStateBus;

#[async_trait::async_trait]
impl StateBus for FailingStateBus {
    async fn set(&self, _scope: &str, _key: &str, _value: Value) -> Result<(), iii_sdk::IIIError> {
        Err(iii_sdk::IIIError::Runtime("kv unreachable".into()))
    }
    async fn get(&self, _scope: &str, _key: &str) -> Option<Value> {
        None
    }
    async fn list_prefix(&self, _scope: &str, _prefix: &str) -> Vec<Value> {
        Vec::new()
    }
    async fn delete(&self, _scope: &str, _key: &str) -> Result<bool, iii_sdk::IIIError> {
        Err(iii_sdk::IIIError::Runtime("kv unreachable".into()))
    }
}

/// A canonical `shell::fs::write` call. The legacy `approval_required`
/// payload is present only to prove the gate ignores it for policy.
pub fn sample_call() -> IncomingCall {
    IncomingCall {
        session_id: "s1".into(),
        function_call_id: "tc-1".into(),
        function_id: "shell::fs::write".into(),
        args: json!({ "path": "/tmp/x" }),
        approval_required: vec!["shell::fs::write".into()],
        event_id: "evt-1".into(),
        reply_stream: "rs-1".into(),
    }
}

/// Explicit empty global ruleset: policy enabled with ask-all fallback.
pub fn empty_policy_rules() -> std::sync::RwLock<approval_gate::LayeredRules> {
    std::sync::RwLock::new(approval_gate::LayeredRules::from_global(Some(Vec::new())))
}

/// Test-only [`OrchestratorWaker`] that records every `resume` and
/// `write_wake_failed` call. Used by the run_tick behavioral tests to
/// assert the watchdog actually wakes the orchestrator for each
/// reclaimed session (instead of just flipping state and silently
/// returning).
pub struct FakeWaker {
    pub resume_calls: Mutex<Vec<String>>,
    pub wake_failed_calls: Mutex<Vec<(String, String)>>,
    /// When `Some`, every `resume` returns this canned outcome instead
    /// of the default `Ok(json!({}))`. Lets a test drive the
    /// wake-failure branch deterministically.
    pub resume_response: Mutex<Option<Result<Value, String>>>,
}

impl Default for FakeWaker {
    fn default() -> Self {
        Self {
            resume_calls: Mutex::new(Vec::new()),
            wake_failed_calls: Mutex::new(Vec::new()),
            resume_response: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl OrchestratorWaker for FakeWaker {
    async fn resume(&self, session_id: &str) -> Result<Value, String> {
        self.resume_calls
            .lock()
            .unwrap()
            .push(session_id.to_string());
        self.resume_response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(json!({})))
    }
    async fn write_wake_failed(&self, session_id: &str, error: &str) {
        self.wake_failed_calls
            .lock()
            .unwrap()
            .push((session_id.to_string(), error.to_string()));
    }
}
