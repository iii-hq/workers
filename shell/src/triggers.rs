//! Shared `iii.trigger` indirection. Both `fs::sandbox::SandboxFsBackend`
//! and `exec::sandbox::SandboxExecBackend` (landing in T6) consume
//! `Arc<dyn TriggerFwd>` so unit tests can inject a stub. The production
//! wiring is `IiiTriggerFwd` wrapping a real `iii_sdk::III`.

use async_trait::async_trait;
use iii_sdk::{IIIError, TriggerRequest};
use serde_json::Value;

#[async_trait]
pub trait TriggerFwd: Send + Sync {
    async fn trigger(&self, function_id: &str, payload: Value) -> Result<Value, IIIError>;
}

pub struct IiiTriggerFwd {
    iii: iii_sdk::III,
}

impl IiiTriggerFwd {
    pub fn new(iii: iii_sdk::III) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl TriggerFwd for IiiTriggerFwd {
    async fn trigger(&self, function_id: &str, payload: Value) -> Result<Value, IIIError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct CountingFwd {
        count: Mutex<usize>,
    }

    #[async_trait]
    impl TriggerFwd for CountingFwd {
        async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, IIIError> {
            *self.count.lock().unwrap() += 1;
            Ok(json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_via_arc() {
        let stub = Arc::new(CountingFwd {
            count: Mutex::new(0),
        });
        let fwd: Arc<dyn TriggerFwd> = stub.clone();
        fwd.trigger("x::y", json!({})).await.unwrap();
        fwd.trigger("x::y", json!({})).await.unwrap();
        // Asserting the side effect the stub records is what makes this
        // a runtime test and not just a compile-time object-safety check.
        assert_eq!(*stub.count.lock().unwrap(), 2);
    }
}
