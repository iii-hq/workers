//! Thin `iii-sdk` client wrapper. The runner, the scripted router, and the
//! recorder each open their own connection so they appear to the engine as
//! distinct workers (prior art: `worktree/tests/integration.rs`).

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::Value;

pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone)]
pub struct Client {
    iii: Arc<IIIClient>,
    name: &'static str,
}

impl Client {
    /// Connect with an explicit worker identity (`conformance-runner`,
    /// `conformance-scripted-router`, `conformance-recorder`).
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

    pub async fn call(&self, function_id: &str, payload: Value) -> Result<Value, String> {
        self.call_with_timeout(function_id, payload, DEFAULT_CALL_TIMEOUT_MS)
            .await
    }

    pub async fn call_with_timeout(
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

    pub async fn shutdown(&self) {
        self.iii.shutdown_async().await;
    }
}
