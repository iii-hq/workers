//! The seam between router logic and the iii SDK. No module other than
//! bus_sdk.rs / channels.rs may import iii_sdk.
//!
//! Each trait method maps 1:1 onto an iii primitive:
//! - `trigger` → `iii.trigger(TriggerRequest { function_id, payload, timeout_ms })`
//!   — invoke any iii function, including engine builtins (`state::get/set`,
//!   `configuration::*`).
//! - `register_function` → `iii.register_function(id, ...)` — expose an iii
//!   function (e.g. `router::chat`) on the bus.
//! - `register_trigger` → bind an existing trigger type (`configuration`,
//!   `subscribe`, ...) to one of our iii functions.
//! - `register_trigger_type` → declare a custom trigger type owned by this
//!   worker (e.g. `router::models::changed`).
use std::sync::Arc;

use crate::types::errors::RouterError;
use futures::future::BoxFuture;
use serde_json::Value;

#[derive(Debug, Clone, thiserror::Error)]
pub enum BusError {
    /// The `{ code, message }` convention (maps to IIIError::Remote on the wire).
    #[error("{code}: {message}")]
    Coded { code: String, message: String },
    #[error("invocation timed out")]
    Timeout,
    #[error("function not found: {0}")]
    FunctionNotFound(String),
    #[error("transport: {0}")]
    Transport(String),
}

impl From<RouterError> for BusError {
    fn from(e: RouterError) -> Self {
        BusError::Coded {
            code: e.code.as_str().to_string(),
            message: e.message,
        }
    }
}

impl BusError {
    pub fn code(&self) -> Option<&str> {
        match self {
            BusError::Coded { code, .. } => Some(code),
            _ => None,
        }
    }
}

pub type Handler = Arc<dyn Fn(Value) -> BoxFuture<'static, Result<Value, BusError>> + Send + Sync>;

/// Build a `Handler` from an async closure.
pub fn handler<F, Fut>(f: F) -> Handler
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Value, BusError>> + Send + 'static,
{
    Arc::new(move |v| Box::pin(f(v)))
}

#[derive(Debug, Clone)]
pub struct TriggerBinding {
    pub id: String,
    pub function_id: String,
    pub config: Value,
}

pub struct TriggerTypeCallbacks {
    pub on_register: Arc<dyn Fn(&TriggerBinding) + Send + Sync>,
    pub on_unregister: Arc<dyn Fn(&TriggerBinding) + Send + Sync>,
}

#[async_trait::async_trait]
pub trait Bus: Send + Sync {
    async fn trigger(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError>;
    fn register_function(&self, id: &str, handler: Handler);
    fn register_trigger(&self, trigger_type: &str, function_id: &str, config: Value);
    fn register_trigger_type(&self, id: &str, description: &str, callbacks: TriggerTypeCallbacks);
}
