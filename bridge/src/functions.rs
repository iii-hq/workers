//! The bridging core. Port of the builtin's handler bodies
//! (engine/src/workers/bridge_client/mod.rs:104-237 and :250-266):
//! `Caller` abstracts "trigger a function on some IIIClient" so the four
//! handlers are unit-testable; `RemoteCaller` re-reads the swappable
//! [`RemoteCell`] per call so a `url` hot-reload takes effect without
//! touching registered handlers.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{Error, IIIClient, TriggerAction};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::config::{ExposeEntry, ForwardEntry};

pub const INVOKE_FN: &str = "bridge.invoke";
pub const INVOKE_ASYNC_FN: &str = "bridge.invoke_async";
/// Builtin default: `Duration::from_secs(30)` (mod.rs:122).
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// The remote engine connection, swappable on a `url` config change.
pub type RemoteCell = Arc<RwLock<Arc<IIIClient>>>;
/// Live `forward` entries keyed by `local_function`.
pub type ForwardTable = Arc<RwLock<HashMap<String, ForwardEntry>>>;
/// Live `expose` entries keyed by the remote function name.
pub type ExposeTable = Arc<RwLock<HashMap<String, ExposeEntry>>>;

/// ErrorBody parity with the builtin: code + message + stacktrace.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
    pub stacktrace: Option<String>,
}

impl BridgeError {
    pub fn bridge(message: impl Into<String>) -> Self {
        Self {
            code: "bridge_error".into(),
            message: message.into(),
            stacktrace: None,
        }
    }
}

impl From<BridgeError> for Error {
    fn from(e: BridgeError) -> Self {
        Error::Remote {
            code: e.code,
            message: e.message,
            stacktrace: e.stacktrace,
        }
    }
}

/// Exact parity with the builtin's `InvokeInput` (mod.rs:50-57).
#[derive(Debug, Clone, Deserialize)]
pub struct InvokeInput {
    pub function_id: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[async_trait]
pub trait Caller: Send + Sync + 'static {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BridgeError>;

    async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BridgeError>;
}

/// Calls the CURRENT remote client (re-read per call).
pub struct RemoteCaller {
    pub cell: RemoteCell,
}

#[async_trait]
impl Caller for RemoteCaller {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BridgeError> {
        let client = self.cell.read().await.clone();
        trigger(
            &client,
            function_id,
            payload,
            None,
            timeout_ms,
            map_remote_error,
        )
        .await
    }

    async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BridgeError> {
        let client = self.cell.read().await.clone();
        trigger(
            &client,
            function_id,
            payload,
            Some(TriggerAction::Void),
            None,
            map_remote_error,
        )
        .await
        .map(|_| ())
    }
}

/// Calls the LOCAL engine — the SDK-side replacement for the builtin's
/// `engine.call(&local_function, input)` (mod.rs:256).
pub struct LocalCaller {
    pub iii: Arc<IIIClient>,
}

#[async_trait]
impl Caller for LocalCaller {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BridgeError> {
        trigger(
            &self.iii,
            function_id,
            payload,
            None,
            timeout_ms,
            map_local_error,
        )
        .await
    }

    async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BridgeError> {
        trigger(
            &self.iii,
            function_id,
            payload,
            Some(TriggerAction::Void),
            None,
            map_local_error,
        )
        .await
        .map(|_| ())
    }
}

async fn trigger(
    client: &IIIClient,
    function_id: &str,
    payload: Value,
    action: Option<TriggerAction>,
    timeout_ms: Option<u64>,
    map_err: fn(Error) -> BridgeError,
) -> Result<Value, BridgeError> {
    client
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action,
            timeout_ms,
        })
        .await
        .map_err(map_err)
}

/// Collapse every SDK error to `bridge_error`, matching the builtin's
/// invoke/invoke_async/forward paths, which ALWAYS discard the real remote
/// error body — including `Error::Remote` — behind a generic `bridge_error`
/// (mod.rs:132-141, 179-186, 224-233).
fn map_remote_error(e: Error) -> BridgeError {
    BridgeError::bridge(e.to_string())
}

/// Preserve real remote error bodies (the builtin's expose path forwards the
/// local function's `ErrorBody` untouched, mod.rs:256-263); everything else
/// becomes `bridge_error`.
fn map_local_error(e: Error) -> BridgeError {
    match e {
        Error::Remote {
            code,
            message,
            stacktrace,
        } => BridgeError {
            code,
            message,
            stacktrace,
        },
        other => BridgeError::bridge(other.to_string()),
    }
}

fn parse_invoke(input: Value) -> Result<InvokeInput, BridgeError> {
    serde_json::from_value(input).map_err(|err| BridgeError {
        code: "deserialization_error".into(),
        message: format!("Failed to parse invoke input: {}", err),
        stacktrace: None,
    })
}

/// `bridge.invoke` — call a remote function and wait (default 30s).
pub async fn handle_invoke(remote: &dyn Caller, input: Value) -> Result<Value, BridgeError> {
    let invoke = parse_invoke(input)?;
    let timeout = invoke.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    remote
        .call(&invoke.function_id, invoke.data, Some(timeout))
        .await
}

/// `bridge.invoke_async` — fire-and-forget. The builtin returns
/// `FunctionResult::NoResult`; SDK function handlers must return a value, so
/// `null` is the closest parity.
pub async fn handle_invoke_async(remote: &dyn Caller, input: Value) -> Result<Value, BridgeError> {
    let invoke = parse_invoke(input)?;
    remote.call_void(&invoke.function_id, invoke.data).await?;
    Ok(Value::Null)
}

/// A `forward` local function — proxy to its remote function.
pub async fn handle_forward(
    remote: &dyn Caller,
    forwards: &ForwardTable,
    local_function: &str,
    input: Value,
) -> Result<Value, BridgeError> {
    let entry = forwards
        .read()
        .await
        .get(local_function)
        .cloned()
        .ok_or_else(|| {
            BridgeError::bridge(format!(
                "forward '{local_function}' was removed from the bridge config"
            ))
        })?;
    let timeout = entry.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    remote
        .call(&entry.remote_function, input, Some(timeout))
        .await
}

/// An `expose` function (registered on the REMOTE engine) — call the local
/// function and forward its result/error untouched.
pub async fn handle_expose(
    local: &dyn Caller,
    exposes: &ExposeTable,
    remote_name: &str,
    input: Value,
) -> Result<Value, BridgeError> {
    let entry = exposes
        .read()
        .await
        .get(remote_name)
        .cloned()
        .ok_or_else(|| {
            BridgeError::bridge(format!(
                "exposed function '{remote_name}' was removed from the bridge config"
            ))
        })?;
    local.call(&entry.local_function, input, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Records the last call; returns a canned result or error.
    #[derive(Default)]
    struct FakeCaller {
        last: tokio::sync::Mutex<Option<(String, Value, Option<u64>, bool)>>, // (fn, payload, timeout, void)
        fail_with: Option<BridgeError>,
    }

    #[async_trait]
    impl Caller for FakeCaller {
        async fn call(
            &self,
            function_id: &str,
            payload: Value,
            timeout_ms: Option<u64>,
        ) -> Result<Value, BridgeError> {
            *self.last.lock().await =
                Some((function_id.into(), payload.clone(), timeout_ms, false));
            match &self.fail_with {
                Some(e) => Err(e.clone()),
                None => Ok(json!({"echo": payload})),
            }
        }

        async fn call_void(&self, function_id: &str, payload: Value) -> Result<(), BridgeError> {
            *self.last.lock().await = Some((function_id.into(), payload, None, true));
            match &self.fail_with {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
    }

    fn forwards(entries: &[(&str, &str, Option<u64>)]) -> ForwardTable {
        Arc::new(tokio::sync::RwLock::new(
            entries
                .iter()
                .map(|(l, r, t)| {
                    (
                        l.to_string(),
                        crate::config::ForwardEntry {
                            local_function: l.to_string(),
                            remote_function: r.to_string(),
                            timeout_ms: *t,
                        },
                    )
                })
                .collect(),
        ))
    }

    fn exposes(entries: &[(&str, &str)]) -> ExposeTable {
        Arc::new(tokio::sync::RwLock::new(
            entries
                .iter()
                .map(|(remote, local)| {
                    (
                        remote.to_string(),
                        crate::config::ExposeEntry {
                            local_function: local.to_string(),
                            remote_function: Some(remote.to_string()),
                        },
                    )
                })
                .collect(),
        ))
    }

    #[tokio::test]
    async fn invoke_calls_remote_with_default_timeout() {
        let c = FakeCaller::default();
        let out = handle_invoke(&c, json!({"function_id": "r.fn", "data": {"n": 1}}))
            .await
            .unwrap();
        assert_eq!(out["echo"]["n"], 1);
        let (id, payload, timeout, void) = c.last.lock().await.clone().unwrap();
        assert_eq!(id, "r.fn");
        assert_eq!(payload["n"], 1);
        assert_eq!(timeout, Some(30_000), "builtin default timeout is 30s");
        assert!(!void);
    }

    #[tokio::test]
    async fn invoke_honors_explicit_timeout_and_defaults_data_to_null() {
        let c = FakeCaller::default();
        handle_invoke(&c, json!({"function_id": "r.fn", "timeout_ms": 1500}))
            .await
            .unwrap();
        let (_, payload, timeout, _) = c.last.lock().await.clone().unwrap();
        assert_eq!(timeout, Some(1500));
        assert!(
            payload.is_null(),
            "data defaults like the builtin's #[serde(default)]"
        );
    }

    #[tokio::test]
    async fn invoke_bad_input_is_deserialization_error() {
        let c = FakeCaller::default();
        let err = handle_invoke(&c, json!({"bad": true})).await.unwrap_err();
        assert_eq!(err.code, "deserialization_error");
        assert!(err.message.starts_with("Failed to parse invoke input:"));
    }

    #[tokio::test]
    async fn invoke_async_fires_void_and_returns_null() {
        let c = FakeCaller::default();
        let out = handle_invoke_async(&c, json!({"function_id": "r.fn", "data": 1}))
            .await
            .unwrap();
        assert!(out.is_null(), "builtin NoResult maps to null");
        let (_, _, timeout, void) = c.last.lock().await.clone().unwrap();
        assert!(void, "must use TriggerAction::Void");
        assert_eq!(timeout, None);
    }

    #[tokio::test]
    async fn forward_routes_to_remote_function_with_entry_timeout() {
        let c = FakeCaller::default();
        let t = forwards(&[("f.local", "f.remote", Some(5000))]);
        handle_forward(&c, &t, "f.local", json!({"v": 1}))
            .await
            .unwrap();
        let (id, _, timeout, _) = c.last.lock().await.clone().unwrap();
        assert_eq!(id, "f.remote");
        assert_eq!(timeout, Some(5000));
    }

    #[tokio::test]
    async fn forward_removed_entry_is_bridge_error() {
        let c = FakeCaller::default();
        let t = forwards(&[]);
        let err = handle_forward(&c, &t, "f.local", json!(1))
            .await
            .unwrap_err();
        assert_eq!(err.code, "bridge_error");
        assert!(err.message.contains("removed"));
    }

    #[tokio::test]
    async fn expose_calls_local_function_and_preserves_remote_error() {
        let ok = FakeCaller::default();
        let t = exposes(&[("remote.echo", "local.echo")]);
        handle_expose(&ok, &t, "remote.echo", json!({"x": 1}))
            .await
            .unwrap();
        let (id, ..) = ok.last.lock().await.clone().unwrap();
        assert_eq!(id, "local.echo");

        let failing = FakeCaller {
            fail_with: Some(BridgeError {
                code: "custom_code".into(),
                message: "boom".into(),
                stacktrace: Some("st".into()),
            }),
            ..Default::default()
        };
        let err = handle_expose(&failing, &t, "remote.echo", json!(1))
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "custom_code",
            "builtin forwards the real error code"
        );
        assert_eq!(err.stacktrace.as_deref(), Some("st"));
    }

    #[test]
    fn bridge_error_maps_to_sdk_remote_error() {
        let e: iii_sdk::Error = BridgeError::bridge("x").into();
        match e {
            iii_sdk::Error::Remote { code, .. } => assert_eq!(code, "bridge_error"),
            other => panic!("expected Error::Remote, got {other:?}"),
        }
    }

    #[test]
    fn map_remote_error_collapses_remote_error_to_bridge_error() {
        let e = Error::Remote {
            code: "validation_error".into(),
            message: "bad input".into(),
            stacktrace: Some("trace".into()),
        };
        let mapped = map_remote_error(e);
        assert_eq!(
            mapped.code, "bridge_error",
            "builtin invoke/invoke_async/forward always collapse to bridge_error (mod.rs:132-141, 179-186, 224-233)"
        );
        assert_eq!(mapped.stacktrace, None);
    }

    #[test]
    fn map_remote_error_collapses_non_remote_error_to_bridge_error() {
        let mapped = map_remote_error(Error::Timeout);
        assert_eq!(mapped.code, "bridge_error");
        assert_eq!(mapped.stacktrace, None);
    }

    #[test]
    fn map_local_error_preserves_remote_error_fields() {
        let e = Error::Remote {
            code: "validation_error".into(),
            message: "bad input".into(),
            stacktrace: Some("trace".into()),
        };
        let mapped = map_local_error(e);
        assert_eq!(
            mapped.code, "validation_error",
            "builtin expose path forwards the local function's real error untouched (mod.rs:256-263)"
        );
        assert_eq!(mapped.message, "bad input");
        assert_eq!(mapped.stacktrace.as_deref(), Some("trace"));
    }

    #[test]
    fn map_local_error_collapses_non_remote_error_to_bridge_error() {
        let mapped = map_local_error(Error::Timeout);
        assert_eq!(mapped.code, "bridge_error");
        assert_eq!(mapped.stacktrace, None);
    }
}
