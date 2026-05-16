//! State-store and function-executor traits, plus their iii-backed
//! implementations.
//!
//! The traits exist as test seams — unit tests swap in
//! `InMemoryStateBus` / `FakeExecutor` while production code uses the
//! `Iii*` implementations that call iii directly.
//!
//! The `__from_approval` marker plumbing is gone (per the refactor's
//! threat-model decision: bus access ≡ shell access in the new model;
//! defense-in-depth via per-target marker verification is out of scope).

use async_trait::async_trait;
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

/// Abstraction over the iii state bus — the kv layer where pending and
/// resolved approval records live. Exists so unit tests can swap in a
/// `BTreeMap`-backed fake; production uses [`IiiStateBus`].
#[async_trait]
pub trait StateBus: Send + Sync {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), IIIError>;
    async fn get(&self, scope: &str, key: &str) -> Option<Value>;
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value>;
    /// Remove a key. Required by `approval::consume`, which returns Done
    /// rows and deletes them in the same call. Idempotent (deleting a
    /// missing key returns Ok).
    async fn delete(&self, scope: &str, key: &str) -> Result<(), IIIError>;
}

/// Invokes an iii function with arguments and returns its result or an
/// error string. Abstracted so tests can stub the underlying call.
#[async_trait]
pub trait FunctionExecutor: Send + Sync {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        function_call_id: &str,
        session_id: &str,
    ) -> Result<Value, String>;
}

/// Production [`FunctionExecutor`] backed by `iii.trigger`.
///
/// Forwards `function_id` + `args` directly to `iii.trigger`. No
/// `__from_approval` marker injection — the target trusts the bus.
pub struct IiiFunctionExecutor {
    pub iii: III,
}

#[async_trait]
impl FunctionExecutor for IiiFunctionExecutor {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        _function_call_id: &str,
        _session_id: &str,
    ) -> Result<Value, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: args,
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// Production [`StateBus`] backed by iii's `state::*` builtins.
pub struct IiiStateBus(pub III);

#[async_trait]
impl StateBus for IiiStateBus {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), IIIError> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": scope, "key": key, "value": value }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map(|_| ())
    }
    async fn get(&self, scope: &str, key: &str) -> Option<Value> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: None,
            })
            .await
            .ok()
            .filter(|v| !v.is_null())
    }
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
        let resp = self
            .0
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": scope, "prefix": prefix }),
                action: None,
                timeout_ms: None,
            })
            .await
            .unwrap_or_else(|_| json!({ "items": [] }));
        // Engine may return either {"items": [...]} or a plain Array.
        if let Some(arr) = resp.as_array() {
            return arr.clone();
        }
        resp.get("items")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.get("value").cloned().unwrap_or(entry))
            .collect()
    }
    async fn delete(&self, scope: &str, key: &str) -> Result<(), IIIError> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::delete".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map(|_| ())
    }
}

