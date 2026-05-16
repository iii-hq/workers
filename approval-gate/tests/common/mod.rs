//! Shared fakes for the approval-gate test suite.
//!
//! Production code goes through `StateBus` and `FunctionExecutor` traits
//! exactly so unit tests can swap in these in-memory fakes. The trait
//! contracts are documented on the production types; the fakes here
//! mirror the wire shape closely enough that any handler behavior tied
//! to bus semantics surfaces in the tests.

#![allow(dead_code)] // Individual test binaries pull in subsets of these.

use std::collections::HashMap;
use std::sync::Mutex;

use approval_gate::{FunctionExecutor, IncomingCall, StateBus};
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
}

/// `StateBus` whose `set` always errors. Used to exercise the gate's
/// fail-closed behavior on transient kv outages.
pub struct FailingStateBus;

#[async_trait::async_trait]
impl StateBus for FailingStateBus {
    async fn set(
        &self,
        _scope: &str,
        _key: &str,
        _value: Value,
    ) -> Result<(), iii_sdk::IIIError> {
        Err(iii_sdk::IIIError::Runtime("kv unreachable".into()))
    }
    async fn get(&self, _scope: &str, _key: &str) -> Option<Value> {
        None
    }
    async fn list_prefix(&self, _scope: &str, _prefix: &str) -> Vec<Value> {
        Vec::new()
    }
}

/// A canonical `shell::fs::write` call gated by the run's
/// `approval_required` list. Most handler tests use this so the only
/// thing they need to vary is the session/call id + whether the run
/// opts in.
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

/// Empty runtime ruleset for handler tests that don't care about the
/// cascade-on-`always` path. Each call freshly constructs the lock so
/// tests stay independent — there's no shared mutable state.
pub fn empty_policy_rules() -> std::sync::RwLock<approval_gate::rules::Ruleset> {
    std::sync::RwLock::new(approval_gate::rules::Ruleset::new())
}
