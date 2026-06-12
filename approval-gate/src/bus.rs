//! The testability seam over `iii.trigger`: every sibling call
//! (`state::*`, `policy::check_permissions`, `harness::function::resolve`,
//! `session::get`, `configuration::*`, event fan-out) goes through [`Bus`],
//! so handlers are fully exercisable without an engine (see
//! testkit/fake_bus.rs).

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{TriggerAction, TriggerRequest, III};
use serde_json::Value;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct BusError(pub String);

#[async_trait]
pub trait Bus: Send + Sync {
    /// Request/response call to another function on the bus.
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError>;

    /// Fire-and-forget delivery (`TriggerAction::Void`) — failures are
    /// logged and swallowed so event fan-out never blocks or breaks the
    /// mutation that produced it.
    async fn call_void(&self, function_id: &str, payload: Value);
}

pub struct IiiBus {
    iii: Arc<III>,
}

impl IiiBus {
    pub fn new(iii: Arc<III>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl Bus for IiiBus {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms,
            })
            .await
            .map_err(|e| BusError(e.to_string()))
    }

    async fn call_void(&self, function_id: &str, payload: Value) {
        let res = self
            .iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(function_id, error = %e, "void fan-out failed");
        }
    }
}
