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

pub struct E2eContext {
    client: IIIClient,
}

impl E2eContext {
    pub async fn connect(url: &str) -> Result<Self> {
        let client = register_worker(
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
        );
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

    pub async fn function_exists(&self, function_id: &str) -> Result<bool> {
        let listed = self
            .trigger_value(
                "engine::functions::list",
                json!({ "include_internal": true }),
            )
            .await?;
        let exists = function_ids(&listed).any(|id| id == function_id);
        Ok(exists)
    }

    pub async fn wait_for_turn(
        &self,
        session_id: &str,
        initial_turn_id: &str,
        timeout: Duration,
        progress_interval: Option<Duration>,
    ) -> Result<StatusReport> {
        let started = tokio::time::Instant::now();
        let deadline = started + timeout;
        let mut next_progress = progress_interval.map(|interval| started + interval);
        let mut active_turn_id = initial_turn_id.to_string();
        loop {
            let status: Option<StatusReport> = self
                .trigger("harness::status", json!({ "session_id": session_id }))
                .await?;
            if let Some(status) = status {
                if let Some(turn_id) = &status.turn_id {
                    active_turn_id.clone_from(turn_id);
                }
                if session_is_terminal(&status)? {
                    return Ok(status);
                }
                if progress_due(&mut next_progress, progress_interval) {
                    tracing::info!(
                        session_id,
                        elapsed_seconds = started.elapsed().as_secs(),
                        status = ?status.status,
                        step = status.step,
                        turns = status.turn_count,
                        max_turns = status.max_turns,
                        pending_functions = status.pending_function_calls.len(),
                        children = status.children.len(),
                        queued_messages = status.queued.len(),
                        expects_wake = status.expects_wake,
                        "E2E scenario progress"
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = self
                    .stop_session(session_id, Some(active_turn_id.as_str()))
                    .await;
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
        progress_interval: Option<Duration>,
    ) -> Result<SessionMetricsResponseV1> {
        let started = tokio::time::Instant::now();
        let deadline = started + timeout;
        let mut next_progress = progress_interval.map(|interval| started + interval);
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
            if progress_due(&mut next_progress, progress_interval) {
                let tree = self
                    .trigger::<_, harness::functions::session_tree::SessionTreeResponseV1>(
                        "harness::session-tree",
                        json!({ "root_session_id": session_id }),
                    )
                    .await;
                match tree {
                    Ok(tree) => tracing::info!(
                        session_id,
                        elapsed_seconds = started.elapsed().as_secs(),
                        sessions = tree.sessions.len(),
                        tree_complete = tree.complete,
                        "waiting for descendant sessions to finish"
                    ),
                    Err(error) => tracing::debug!(
                        session_id,
                        %error,
                        "could not collect progress for descendant sessions"
                    ),
                }
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

fn progress_due(
    next_progress: &mut Option<tokio::time::Instant>,
    interval: Option<Duration>,
) -> bool {
    let Some(next) = *next_progress else {
        return false;
    };
    let now = tokio::time::Instant::now();
    if now < next {
        return false;
    }
    *next_progress = interval.map(|interval| now + interval);
    true
}

fn function_ids(listed: &Value) -> impl Iterator<Item = &str> {
    listed
        .as_array()
        .or_else(|| listed.as_object()?.values().find_map(Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("function_id").and_then(Value::as_str))
                .or_else(|| item.get("id").and_then(Value::as_str))
        })
}

fn session_is_terminal(status: &StatusReport) -> Result<bool> {
    match status.status {
        TurnStatus::Completed => Ok(!status.expects_wake),
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
        TurnStatus::Running | TurnStatus::AwaitingFunctions => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(turn_id: &str, status: TurnStatus, expects_wake: bool) -> StatusReport {
        StatusReport {
            session_id: "session".to_string(),
            turn_id: Some(turn_id.to_string()),
            status,
            step: 0,
            turn_count: 1,
            max_turns: 100,
            validation_retries: 0,
            max_validation_retries: 0,
            transient_resumes: 0,
            max_transient_resumes: 0,
            partial_result_available: false,
            depth: 0,
            pending_function_calls: Vec::new(),
            children: Vec::new(),
            expects_wake,
            queued: Vec::new(),
            result: None,
            result_error: None,
        }
    }

    #[test]
    fn a_wake_can_advance_the_session_to_a_new_turn() {
        let parked = status("turn-initial", TurnStatus::Completed, true);
        let resumed = status("turn-after-wake", TurnStatus::Running, false);
        let completed = status("turn-after-wake", TurnStatus::Completed, false);

        assert!(!session_is_terminal(&parked).unwrap());
        assert!(!session_is_terminal(&resumed).unwrap());
        assert!(session_is_terminal(&completed).unwrap());
    }

    #[test]
    fn failed_and_cancelled_turns_are_errors() {
        for turn_status in [TurnStatus::Failed, TurnStatus::Cancelled] {
            let mut report = status("turn", turn_status, false);
            report.result_error = Some("provider stopped".to_string());

            let error = session_is_terminal(&report).unwrap_err();
            assert!(error.to_string().contains("provider stopped"));
        }
    }

    #[test]
    fn function_discovery_accepts_engine_response_shapes() {
        let listed = json!({
            "functions": [
                { "function_id": "provider::zai::stream" },
                { "id": "database::query" },
                "state::get"
            ]
        });
        assert_eq!(
            function_ids(&listed).collect::<Vec<_>>(),
            ["provider::zai::stream", "database::query", "state::get"]
        );
    }
}
