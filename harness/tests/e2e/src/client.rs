//! Thin `iii-sdk` client wrapper. The runner, scripted router, and scenario
//! probe each open their own connection so they appear to the engine as
//! distinct workers (prior art: `worktree/tests/integration.rs`).

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::Value;

use crate::deadline::Deadline;

pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone)]
pub struct Client {
    iii: Arc<IIIClient>,
    name: &'static str,
}

impl Client {
    /// Connect with an explicit worker identity.
    pub fn connect(ws_url: &str, name: &'static str) -> Self {
        let metadata = iii_sdk::iii::WorkerMetadata {
            name: name.to_string(),
            pid: Some(std::process::id()),
            ..Default::default()
        };
        let iii = register_worker(
            ws_url,
            InitOptions {
                metadata: Some(metadata),
                ..Default::default()
            },
        );
        Self {
            iii: Arc::new(iii),
            name,
        }
    }

    pub fn inner(&self) -> &Arc<IIIClient> {
        &self.iii
    }

    async fn call_with_timeout(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Value, String> {
        // Outer timeout: the SDK's own timeout covers the engine round-trip,
        // but a connection that never establishes can park the future.
        let outer = std::time::Duration::from_millis(timeout_ms + 5_000);
        match tokio::time::timeout(
            outer,
            self.iii.trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            }),
        )
        .await
        {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(format!("{function_id}: {e}")),
            Err(_) => Err(format!(
                "{function_id}: no response within {}ms ({} connection)",
                outer.as_millis(),
                self.name
            )),
        }
    }

    /// Call an engine function without allowing either the SDK timeout or
    /// its connection wrapper to outlive the scenario deadline.
    pub async fn call_with_deadline(
        &self,
        function_id: &str,
        payload: Value,
        deadline: Deadline,
        maximum_ms: u64,
    ) -> Result<Value, String> {
        let timeout_ms = deadline
            .timeout_ms(maximum_ms, function_id)
            .map_err(|error| error.to_string())?;
        deadline
            .timeout(
                function_id,
                self.call_with_timeout(function_id, payload, timeout_ms),
            )
            .await
            .map_err(|error| error.to_string())?
    }

    pub async fn shutdown(&self) {
        self.iii.shutdown_async().await;
    }
}
