//! A scripted [`Bus`]: records every call and answers from per-function
//! handlers. [`MemoryState`] mirrors the engine's state worker semantics
//! exactly — including the null-tombstone behavior of `state::set` with
//! `value: null` — so the delete-with-gate logic is tested against the
//! real contract.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::bus::{Bus, BusError};

type Handler = Arc<dyn Fn(&Value) -> Result<Value, BusError> + Send + Sync>;

#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub function_id: String,
    pub payload: Value,
    pub void: bool,
    pub timeout_ms: Option<u64>,
}

#[derive(Default)]
pub struct FakeBus {
    handlers: Mutex<HashMap<String, Handler>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script a handler for one function id.
    pub fn on<F>(&self, function_id: &str, handler: F)
    where
        F: Fn(&Value) -> Result<Value, BusError> + Send + Sync + 'static,
    {
        self.lock_handlers()
            .insert(function_id.to_string(), Arc::new(handler));
    }

    /// Script a constant reply.
    pub fn on_value(&self, function_id: &str, value: Value) {
        self.on(function_id, move |_| Ok(value.clone()));
    }

    /// Script a constant transport error.
    pub fn on_error(&self, function_id: &str, message: &str) {
        let message = message.to_string();
        self.on(function_id, move |_| Err(BusError(message.clone())));
    }

    /// Install `state::get/set/delete/list` handlers backed by a shared
    /// in-memory store; returns the store for direct assertions.
    pub fn with_memory_state(&self) -> MemoryState {
        let state = MemoryState::default();

        let s = state.clone();
        self.on("state::get", move |payload| Ok(s.get(payload)));
        let s = state.clone();
        self.on("state::set", move |payload| Ok(s.set(payload)));
        let s = state.clone();
        self.on("state::delete", move |payload| Ok(s.delete(payload)));
        let s = state.clone();
        self.on("state::list", move |payload| Ok(s.list(payload)));

        state
    }

    pub fn calls(&self) -> Vec<RecordedCall> {
        self.lock_calls().clone()
    }

    pub fn calls_to(&self, function_id: &str) -> Vec<RecordedCall> {
        self.lock_calls()
            .iter()
            .filter(|c| c.function_id == function_id)
            .cloned()
            .collect()
    }

    fn lock_handlers(&self) -> std::sync::MutexGuard<'_, HashMap<String, Handler>> {
        self.handlers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_calls(&self) -> std::sync::MutexGuard<'_, Vec<RecordedCall>> {
        self.calls
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn handler_for(&self, function_id: &str) -> Option<Handler> {
        self.lock_handlers().get(function_id).cloned()
    }
}

#[async_trait]
impl Bus for FakeBus {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError> {
        self.lock_calls().push(RecordedCall {
            function_id: function_id.to_string(),
            payload: payload.clone(),
            void: false,
            timeout_ms,
        });
        match self.handler_for(function_id) {
            Some(handler) => handler(&payload),
            // An unscripted function behaves like an absent sibling —
            // the right default for fail-closed tests.
            None => Err(BusError(format!("no handler for {function_id}"))),
        }
    }

    async fn call_void(&self, function_id: &str, payload: Value) {
        self.lock_calls().push(RecordedCall {
            function_id: function_id.to_string(),
            payload: payload.clone(),
            void: true,
            timeout_ms: None,
        });
        if let Some(handler) = self.handler_for(function_id) {
            let _ = handler(&payload);
        }
    }
}

/// In-memory mirror of the engine's state worker (builtins/kv.rs):
/// `set` stores the value verbatim (a JSON null IS stored — the tombstone
/// the production delete path must clean up), returning `{old_value,
/// new_value}` atomically; `delete` removes and returns the old value;
/// `list` returns values only.
#[derive(Clone, Default)]
pub struct MemoryState {
    inner: Arc<Mutex<HashMap<(String, String), Value>>>,
}

impl MemoryState {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Value>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn scope_key(payload: &Value) -> (String, String) {
        (
            payload
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            payload
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    }

    fn get(&self, payload: &Value) -> Value {
        let key = Self::scope_key(payload);
        self.lock().get(&key).cloned().unwrap_or(Value::Null)
    }

    fn set(&self, payload: &Value) -> Value {
        let key = Self::scope_key(payload);
        let value = payload.get("value").cloned().unwrap_or(Value::Null);
        let old = self.lock().insert(key, value.clone());
        json!({ "old_value": old.unwrap_or(Value::Null), "new_value": value })
    }

    fn delete(&self, payload: &Value) -> Value {
        let key = Self::scope_key(payload);
        self.lock().remove(&key).unwrap_or(Value::Null)
    }

    fn list(&self, payload: &Value) -> Value {
        let scope = payload
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let values: Vec<Value> = self
            .lock()
            .iter()
            .filter(|((s, _), _)| *s == scope)
            .map(|(_, v)| v.clone())
            .collect();
        Value::Array(values)
    }

    /// Direct read for assertions.
    pub fn peek(&self, scope: &str, key: &str) -> Option<Value> {
        self.lock()
            .get(&(scope.to_string(), key.to_string()))
            .cloned()
    }

    /// Direct write for seeding.
    pub fn seed(&self, scope: &str, key: &str, value: Value) {
        self.lock()
            .insert((scope.to_string(), key.to_string()), value);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}
