//! The seam between the worker and the engine: function registration,
//! secret references, the vault and change events. Tests swap in a fake.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::functions::{Coalescer, ON_SERVERS_CHANGED};

pub type CallFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
pub type Handler = Arc<dyn Fn(Value) -> CallFuture + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDef {
    pub id: String,
    pub description: String,
    pub request_format: Value,
    pub response_format: Value,
    pub metadata: Value,
}

/// A live registration. Dropping it does nothing; only `unregister` frees
/// the id.
pub trait Registration: Send + Sync {
    fn unregister(&self);
}

#[derive(Debug, Clone, PartialEq)]
pub enum SecretError {
    NotFound,
    Forbidden,
    Unreachable(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerEvent {
    pub server: String,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    State,
    Tools,
    Process,
    Login,
    Stderr,
}

#[async_trait::async_trait]
pub trait Host: Send + Sync + 'static {
    /// Panics when `def.id` is already registered, as the SDK does:
    /// unregister the old one first.
    fn register(&self, def: &FunctionDef, handler: Handler) -> Box<dyn Registration>;
    /// The function ids the engine lists now.
    async fn registered(&self) -> HashSet<String>;
    /// The value behind a `secret://` or `env://` reference.
    async fn resolve(&self, reference: &str) -> Result<String, SecretError>;
    /// Writes the vault secret `name` when its current value equals
    /// `expected` (`None`: absent). `Ok(false)` when it did not.
    async fn compare_and_set(
        &self,
        name: &str,
        expected: Option<&str>,
        value: &str,
    ) -> Result<bool, SecretError>;
    fn emit(&self, event: ServerEvent);
    /// The engine's http port, when it runs the http worker.
    async fn http_port(&self) -> Option<u16>;
}

/// The `Host` of a running worker: every method is one engine call.
pub struct EngineHost {
    iii: Arc<IIIClient>,
    events: Arc<Coalescer>,
}

struct EngineRegistration(FunctionRef);

impl Registration for EngineRegistration {
    fn unregister(&self) {
        self.0.unregister();
    }
}

fn secret_error(error: &Error) -> SecretError {
    match error {
        Error::Remote { code, .. } if code == "SECRET_NOT_FOUND" => SecretError::NotFound,
        Error::Remote { code, .. } if code == "SECRET_FORBIDDEN" => SecretError::Forbidden,
        other => SecretError::Unreachable(other.to_string()),
    }
}

/// The `mcp::` ids of an `engine::functions::list` reply.
fn mcp_ids(list: &Value) -> HashSet<String> {
    list["functions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["function_id"].as_str())
        .filter(|id| id.starts_with("mcp::"))
        .map(String::from)
        .collect()
}

impl EngineHost {
    pub fn new(iii: Arc<IIIClient>) -> Arc<EngineHost> {
        let engine = iii.clone();
        // Unobserved also before `WorkerRegistered` and on an engine without
        // observe v2, where calling the private function would fail.
        let events = Coalescer::new(Arc::new(move |event: ServerEvent| {
            if !engine.is_observed(ON_SERVERS_CHANGED) {
                return;
            }
            let iii = engine.clone();
            tokio::spawn(async move {
                let request = CallRequest {
                    action: Some(CallAction::Void),
                    ..CallRequest::new(ON_SERVERS_CHANGED, json!(event))
                };
                if let Err(error) = iii.call(request).await {
                    tracing::debug!(%error, "{ON_SERVERS_CHANGED} call failed");
                }
            });
        }));
        Arc::new(EngineHost { iii, events })
    }

    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, Error> {
        self.iii
            .call(CallRequest {
                function_id: function_id.into(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
    }
}

#[async_trait::async_trait]
impl Host for EngineHost {
    fn register(&self, def: &FunctionDef, handler: Handler) -> Box<dyn Registration> {
        let function = RegisterFunction::new_async(move |input: Value| {
            let call = handler(input);
            async move { call.await.map_err(Error::Handler) }
        })
        .description(def.description.clone())
        .request_format(def.request_format.clone())
        .response_format(def.response_format.clone())
        .metadata(def.metadata.clone());
        Box::new(EngineRegistration(
            self.iii.register_function(def.id.clone(), function),
        ))
    }

    async fn registered(&self) -> HashSet<String> {
        match self.call("engine::functions::list", json!({}), 5_000).await {
            Ok(list) => mcp_ids(&list),
            Err(error) => {
                tracing::warn!(%error, "engine::functions::list failed; no function shows as registered");
                HashSet::new()
            }
        }
    }

    async fn resolve(&self, reference: &str) -> Result<String, SecretError> {
        let reply = self
            .call("secrets::resolve", json!({ "ref": reference }), 10_000)
            .await
            .map_err(|e| secret_error(&e))?;
        reply["value"].as_str().map(String::from).ok_or_else(|| {
            SecretError::Unreachable("secrets::resolve answered without a value".into())
        })
    }

    async fn compare_and_set(
        &self,
        name: &str,
        expected: Option<&str>,
        value: &str,
    ) -> Result<bool, SecretError> {
        let payload = json!({
            "name": name,
            "expected": expected,
            "value": value,
            "consumers": ["mcp-client"],
        });
        let reply = self
            .call("secrets::compare-and-set", payload, 10_000)
            .await
            .map_err(|e| secret_error(&e))?;
        reply["swapped"].as_bool().ok_or_else(|| {
            SecretError::Unreachable("secrets::compare-and-set answered without `swapped`".into())
        })
    }

    fn emit(&self, event: ServerEvent) {
        self.events.push(event);
    }

    async fn http_port(&self) -> Option<u16> {
        let reply = self.call("http::status", json!({}), 2_000).await.ok()?;
        reply["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(code: &str) -> Error {
        Error::Remote {
            code: code.into(),
            message: "m".into(),
            stacktrace: None,
        }
    }

    #[test]
    fn secret_errors_follow_the_secrets_worker_codes() {
        assert_eq!(
            secret_error(&remote("SECRET_NOT_FOUND")),
            SecretError::NotFound
        );
        assert_eq!(
            secret_error(&remote("SECRET_FORBIDDEN")),
            SecretError::Forbidden
        );
        assert_eq!(
            secret_error(&remote("function_not_found")),
            SecretError::Unreachable("remote error (function_not_found): m".into())
        );
        assert_eq!(
            secret_error(&Error::Timeout),
            SecretError::Unreachable("invocation timed out".into())
        );
    }

    #[test]
    fn registered_ids_are_the_mcp_rows_of_the_engine_list() {
        let list = json!({ "functions": [
            { "function_id": "mcp::linear::whoami", "description": "x" },
            { "function_id": "mcp-client::servers::list" },
            { "function_id": "state::get" }
        ] });
        assert_eq!(
            mcp_ids(&list),
            HashSet::from(["mcp::linear::whoami".to_string()])
        );
        assert!(mcp_ids(&json!({})).is_empty());
    }
}
