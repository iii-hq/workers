//! Shared `iii.call` indirection. Both `fs::sandbox::SandboxFsBackend`
//! and `exec::sandbox::SandboxExecBackend` consume `Arc<dyn CallFwd>` so
//! unit tests can inject a stub. The production wiring is `IiiCallFwd`
//! wrapping a real `iii_sdk::IIIClient`.

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::CallRequest;
use serde_json::Value;

#[async_trait]
pub trait CallFwd: Send + Sync {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Value, Error>;
}

pub struct IiiCallFwd {
    iii: iii_sdk::IIIClient,
}

impl IiiCallFwd {
    pub fn new(iii: iii_sdk::IIIClient) -> Self {
        Self { iii }
    }
}

/// iii-sandbox is engine-owned: its `sandbox::*` functions exist only in
/// `default`, whatever namespace this worker runs in (a Compose project's,
/// say). A suite that stands in for its file operations (`sandbox::fs::*`)
/// names its own namespace through `III_SANDBOX_FS_NAMESPACE`; create, exec
/// and stop always reach the real daemon.
static SANDBOX_FS_NAMESPACE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("III_SANDBOX_FS_NAMESPACE")
        .ok()
        .map(|namespace| namespace.trim().to_string())
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or_else(|| "default".to_string())
});

/// `sandbox::fs::*` goes to `fs_namespace`, the rest of `sandbox::*` to
/// `default`; everything else (`compose::*` included) keeps the SDK's routing.
fn forward_namespace<'a>(function_id: &str, fs_namespace: &'a str) -> Option<&'a str> {
    if function_id.starts_with("sandbox::fs::") {
        Some(fs_namespace)
    } else if function_id.starts_with("sandbox::") {
        Some("default")
    } else {
        None
    }
}

/// Slack over a payload's own `timeout_ms` before the call gives up: the callee
/// (`sandbox::exec`) may use all of it, then the reply still has to come back.
const CALL_SLACK_MS: u64 = 30_000;

/// A payload's `timeout_ms` is the callee's own budget. Without a call timeout
/// the SDK waits only its 30 s default, so a longer sandbox exec failed with
/// `Timeout` while the VM was still running it.
fn call_request(function_id: &str, payload: Value) -> CallRequest {
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .map(|ms| ms.saturating_add(CALL_SLACK_MS));
    CallRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms,
    }
}

#[async_trait]
impl CallFwd for IiiCallFwd {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Value, Error> {
        let request = call_request(function_id, payload);
        match forward_namespace(function_id, &SANDBOX_FS_NAMESPACE) {
            Some(namespace) => self.iii.call(request.namespace(namespace)).await,
            None => self.iii.call(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn sandbox_calls_reach_default_and_only_fs_follows_the_override() {
        for function_id in ["sandbox::create", "sandbox::exec", "sandbox::stop"] {
            assert_eq!(
                forward_namespace(function_id, "ide-e2e"),
                Some("default"),
                "{function_id}"
            );
        }
        assert_eq!(
            forward_namespace("sandbox::fs::ls", "default"),
            Some("default")
        );
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

    #[test]
    fn a_long_exec_call_outlasts_the_sdk_default() {
        let exec = call_request(
            "sandbox::exec",
            json!({"cmd": "sleep", "args": ["40"], "timeout_ms": 120_000}),
        );
        assert_eq!(exec.timeout_ms, Some(150_000));
        // No budget in the payload keeps the SDK default.
        assert_eq!(call_request("compose::status", json!({})).timeout_ms, None);
    }

    struct CountingFwd {
        count: Mutex<usize>,
    }

    #[async_trait]
    impl CallFwd for CountingFwd {
        async fn call(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
            *self.count.lock().unwrap() += 1;
            Ok(json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_via_arc() {
        let stub = Arc::new(CountingFwd {
            count: Mutex::new(0),
        });
        let fwd: Arc<dyn CallFwd> = stub.clone();
        fwd.call("x::y", json!({})).await.unwrap();
        fwd.call("x::y", json!({})).await.unwrap();
        // Asserting the side effect the stub records is what makes this
        // a runtime test and not just a compile-time object-safety check.
        assert_eq!(*stub.count.lock().unwrap(), 2);
    }
}
