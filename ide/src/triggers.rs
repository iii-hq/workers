//! Shared `iii.trigger` indirection. Both `fs::sandbox::SandboxFsBackend`
//! and `exec::sandbox::SandboxExecBackend` (landing in T6) consume
//! `Arc<dyn TriggerFwd>` so unit tests can inject a stub. The production
//! wiring is `IiiTriggerFwd` wrapping a real `iii_sdk::IIIClient`.

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use serde_json::Value;

#[async_trait]
pub trait TriggerFwd: Send + Sync {
    async fn trigger(&self, function_id: &str, payload: Value) -> Result<Value, Error>;
}

pub struct IiiTriggerFwd {
    iii: iii_sdk::IIIClient,
}

impl IiiTriggerFwd {
    pub fn new(iii: iii_sdk::IIIClient) -> Self {
        Self { iii }
    }
}

/// Where iii-sandbox's `sandbox::*` functions resolve. iii-sandbox is
/// engine-owned, so they exist only in `default`, whatever namespace this
/// worker runs in (a Compose project's, say). A suite that stands in for
/// iii-sandbox with its own `sandbox::*` points this at its namespace through
/// `III_SANDBOX_NAMESPACE`.
static SANDBOX_NAMESPACE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("III_SANDBOX_NAMESPACE")
        .ok()
        .map(|namespace| namespace.trim().to_string())
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or_else(|| "default".to_string())
});

/// `sandbox::*` goes to `sandbox_namespace`; everything else (`compose::*`
/// included) keeps the SDK's routing.
fn forward_namespace<'a>(function_id: &str, sandbox_namespace: &'a str) -> Option<&'a str> {
    function_id
        .starts_with("sandbox::")
        .then_some(sandbox_namespace)
}

#[async_trait]
impl TriggerFwd for IiiTriggerFwd {
    async fn trigger(&self, function_id: &str, payload: Value) -> Result<Value, Error> {
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: None,
        };
        match forward_namespace(function_id, &SANDBOX_NAMESPACE) {
            Some(namespace) => self.iii.trigger(request.namespace(namespace)).await,
            None => self.iii.trigger(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn sandbox_calls_resolve_in_the_sandbox_namespace_and_compose_stays_in_the_project() {
        for function_id in ["sandbox::exec", "sandbox::fs::read", "sandbox::fs::write"] {
            assert_eq!(
                forward_namespace(function_id, "default"),
                Some("default"),
                "{function_id}"
            );
        }
        assert_eq!(
            forward_namespace("sandbox::fs::ls", "ide-e2e"),
            Some("ide-e2e")
        );
        for function_id in [
            "compose::add",
            "compose::status",
            "sandbox-code-runner::run",
        ] {
            assert_eq!(
                forward_namespace(function_id, "default"),
                None,
                "{function_id}"
            );
        }
    }

    struct CountingFwd {
        count: Mutex<usize>,
    }

    #[async_trait]
    impl TriggerFwd for CountingFwd {
        async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
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
