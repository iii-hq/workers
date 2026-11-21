//! The seam between the worker and the engine: function registration,
//! secret references, the vault and change events. Tests swap in a fake.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
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
