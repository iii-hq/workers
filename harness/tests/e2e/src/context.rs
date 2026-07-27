use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use harness::functions::metrics::SessionMetricsResponseV1;
use harness::functions::status::StatusReport;
use harness::types::turn::TurnStatus;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

pub const INVOCATION_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub struct E2eContext {
    client: Arc<IIIClient>,
}

impl E2eContext {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = Arc::new(register_worker(
            url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    runtime: "rust".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    name: "harness-e2e".to_string(),
                    os: std::env::consts::OS.to_string(),
                    pid: Some(std::process::id()),
                    ..WorkerMetadata::default()
                }),
                ..InitOptions::default()
            },
        ));
        let context = Self { client };
        context.wait_until_ready().await?;
        Ok(context)
    }

    pub async fn trigger<I, O>(&self, function_id: &str, payload: I) -> Result<O>
    where
        I: Serialize + Send,
        O: DeserializeOwned,
    {
        let payload = serde_json::to_value(payload)
            .with_context(|| format!("serialize request for {function_id}"))?;
        let value = self
            .trigger_value_with_timeout(function_id, payload, INVOCATION_TIMEOUT)
            .await?;
        serde_json::from_value(value).with_context(|| format!("decode response from {function_id}"))
    }

    pub async fn trigger_value(&self, function_id: &str, payload: Value) -> Result<Value> {
        self.trigger_value_with_timeout(function_id, payload, INVOCATION_TIMEOUT)
            .await
    }

    pub async fn wait_for_turn(
        &self,
        session_id: &str,
        turn_id: &str,
        timeout: Duration,
    ) -> Result<StatusReport> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let status: Option<StatusReport> = self
                .trigger("harness::status", json!({ "session_id": session_id }))
                .await?;
            if let Some(status) = status {
                if status.turn_id.as_deref().is_some_and(|id| id != turn_id) {
                    bail!(
                        "harness::status returned turn {:?} instead of {turn_id}",
                        status.turn_id
                    );
                }
                match status.status {
                    TurnStatus::Completed if !status.expects_wake => return Ok(status),
                    TurnStatus::Failed | TurnStatus::Cancelled => {
                        bail!(
                            "turn ended as {:?}: {}",
                            status.status,
                            status
                                .result_error
                                .as_deref()
                                .unwrap_or("no error was reported")
                        );
                    }
                    _ => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = self.stop_session(session_id, Some(turn_id)).await;
                bail!(
                    "scenario exceeded {}s while waiting for session {session_id}",
                    timeout.as_secs()
                );
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    pub async fn metrics(&self, session_id: &str) -> Result<SessionMetricsResponseV1> {
        self.trigger("harness::metrics", json!({ "root_session_id": session_id }))
            .await
    }

    pub async fn wait_for_complete_metrics(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> Result<SessionMetricsResponseV1> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                self.stop_session_tree(session_id).await;
                bail!(
                    "scenario exceeded {}s while waiting for the complete session tree {session_id}",
                    timeout.as_secs()
                );
            }
            let metrics = match tokio::time::timeout(remaining, self.metrics(session_id)).await {
                Ok(result) => result?,
                Err(_) => {
                    self.stop_session_tree(session_id).await;
                    bail!(
                        "scenario exceeded {}s while waiting for the complete session tree {session_id}",
                        timeout.as_secs()
                    );
                }
            };
            if metrics.complete {
                return Ok(metrics);
            }
            tokio::time::sleep(POLL_INTERVAL.min(remaining)).await;
        }
    }

    pub async fn transcript(&self, session_id: &str) -> Result<Value> {
        let mut cursor: Option<String> = None;
        let mut messages = Vec::new();
        loop {
            let response: Value = self
                .trigger(
                    "session::messages",
                    json!({
                        "session_id": session_id,
                        "limit": 500,
                        "cursor": cursor,
                        "include_custom": true,
                    }),
                )
                .await?;
            let page = response
                .get("messages")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("session::messages returned a malformed page"))?;
            messages.extend(page.iter().cloned());
            let next = response
                .get("next_cursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if next.is_none() {
                break;
            }
            if next == cursor {
                bail!("session::messages repeated transcript cursor for {session_id}");
            }
            cursor = next;
        }
        Ok(json!({ "messages": messages }))
    }

    pub async fn stop_session(&self, session_id: &str, turn_id: Option<&str>) -> Result<()> {
        let _: harness::functions::stop::StopResponse = self
            .trigger(
                "harness::stop",
                json!({ "session_id": session_id, "turn_id": turn_id }),
            )
            .await?;
        Ok(())
    }

    async fn stop_session_tree(&self, root_session_id: &str) {
        let tree = tokio::time::timeout(
            Duration::from_secs(5),
            self.trigger::<_, harness::functions::session_tree::SessionTreeResponseV1>(
                "harness::session-tree",
                json!({ "root_session_id": root_session_id }),
            ),
        )
        .await;
        if let Ok(Ok(tree)) = tree {
            for session in tree.sessions.iter().rev() {
                let _ = self.stop_session(&session.session_id, None).await;
            }
        } else {
            let _ = self.stop_session(root_session_id, None).await;
        }
    }

    pub async fn teardown(&self, root_session_id: &str) -> Result<u64> {
        let response: harness::functions::teardown::TeardownResponseV1 = self
            .trigger(
                "harness::teardown",
                json!({ "root_session_id": root_session_id }),
            )
            .await?;
        Ok(response.removed)
    }

    pub async fn shutdown(&self) {
        self.client.shutdown_async().await;
    }

    async fn wait_until_ready(&self) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if self
                .trigger_value_with_timeout(
                    "engine::functions::list",
                    json!({ "include_internal": true }),
                    Duration::from_secs(1),
                )
                .await
                .is_ok()
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("timed out connecting the E2E runner to the iii engine");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn trigger_value_with_timeout(
        &self,
        function_id: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let timeout_ms = timeout.as_millis().min(u64::MAX as u128) as u64;
        let outer = timeout + Duration::from_secs(5);
        match tokio::time::timeout(
            outer,
            self.client.trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            }),
        )
        .await
        {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(anyhow!("{function_id}: {error}")),
            Err(_) => bail!("{function_id}: no response within {}ms", outer.as_millis()),
        }
    }
}
