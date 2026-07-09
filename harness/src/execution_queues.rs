//! Harness-owned named function queues.
//!
//! The standalone `queue` worker owns their durable configuration and
//! consumers. Harness only ensures its three workload lanes at boot, then
//! uses `TriggerAction::Enqueue` for every durable turn step.

use std::time::Duration;

use anyhow::{anyhow, Result};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::time::Instant;

use crate::types::turn::TurnLane;

const ENSURE_QUEUE_FUNCTION_ID: &str = "engine::queue::ensure";
const LIST_TOPICS_FUNCTION_ID: &str = "engine::queue::list_topics";
const BOOT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

fn ensure_requests() -> Vec<TriggerRequest> {
    TurnLane::ALL
        .into_iter()
        .map(|lane| TriggerRequest {
            function_id: ENSURE_QUEUE_FUNCTION_ID.to_string(),
            payload: json!({
                "queue": lane.queue_name(),
                "config": {
                    "type": "standard",
                    "concurrency": 10,
                    "max_retries": 3,
                    "backoff_ms": 1000
                }
            }),
            action: None,
            timeout_ms: Some(REQUEST_TIMEOUT.as_millis() as u64),
        })
        .collect()
}

fn topics_ready(value: &Value) -> bool {
    let topics = value
        .as_array()
        .or_else(|| value.get("topics").and_then(Value::as_array));
    let Some(topics) = topics else {
        return false;
    };

    TurnLane::ALL.iter().all(|lane| {
        let queue = lane.queue_name();
        topics.iter().any(|topic| {
            topic.as_str() == Some(queue)
                || topic.get("name").and_then(Value::as_str) == Some(queue)
                || topic.get("queue").and_then(Value::as_str) == Some(queue)
        })
    })
}

/// Ensure every harness execution queue exists and is visible through the
/// queue worker before this process accepts sends.
///
/// Ensures are deliberately retried until the shared boot deadline: worker
/// manifest dependencies may still be connecting when harness starts. The
/// queues are durable worker-owned resources, so a partial success is left in
/// place and retried idempotently on the next pass or restart.
pub async fn provision(iii: &IIIClient) -> Result<()> {
    let deadline = Instant::now() + BOOT_TIMEOUT;
    let mut last_error = "queue worker has not reported all named queues".to_string();

    loop {
        let mut all_ensured = true;

        for request in ensure_requests() {
            let queue = request.payload["queue"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow!(
                    "harness execution-queue provisioning timed out after 10 seconds: {last_error}"
                ));
            }
            let remaining = deadline.saturating_duration_since(now);
            match tokio::time::timeout(remaining.min(REQUEST_TIMEOUT), iii.trigger(request)).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    all_ensured = false;
                    last_error = format!("ensuring `{queue}`: {error}");
                }
                Err(_) => {
                    all_ensured = false;
                    last_error = format!("ensuring `{queue}` timed out");
                }
            }
        }

        if all_ensured {
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow!(
                    "harness execution-queue provisioning timed out after 10 seconds: {last_error}"
                ));
            }
            let remaining = deadline.saturating_duration_since(now);
            let request = iii.trigger(TriggerRequest {
                function_id: LIST_TOPICS_FUNCTION_ID.to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(REQUEST_TIMEOUT.as_millis() as u64),
            });
            match tokio::time::timeout(remaining.min(REQUEST_TIMEOUT), request).await {
                Ok(Ok(value)) if topics_ready(&value) => return Ok(()),
                Ok(Ok(_)) => last_error = "not all harness queues are ready".to_string(),
                Ok(Err(error)) => last_error = error.to_string(),
                Err(_) => last_error = "queue readiness request timed out".to_string(),
            }
        }

        let now = Instant::now();
        if now >= deadline {
            return Err(anyhow!(
                "harness execution-queue provisioning timed out after 10 seconds: {last_error}"
            ));
        }
        tokio::time::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_builds_three_independent_standard_queue_ensures() {
        let requests = ensure_requests();
        assert_eq!(requests.len(), 3);
        for (request, lane) in requests.iter().zip(TurnLane::ALL) {
            assert_eq!(request.function_id, ENSURE_QUEUE_FUNCTION_ID);
            assert!(request.action.is_none());
            assert_eq!(request.payload["queue"], lane.queue_name());
            assert_eq!(request.payload["config"]["type"], "standard");
            assert_eq!(request.payload["config"]["concurrency"], 10);
            assert_eq!(request.payload["config"]["max_retries"], 3);
            assert_eq!(request.payload["config"]["backoff_ms"], 1000);
            assert_eq!(request.timeout_ms, Some(1000));
        }
    }

    #[test]
    fn readiness_requires_every_harness_queue() {
        assert!(!topics_ready(&serde_json::json!([
            {"name": "harness-turn"},
            {"name": "harness-subagent"}
        ])));
        assert!(topics_ready(&serde_json::json!({
            "topics": [
                {"name": "harness-reactive"},
                {"name": "harness-turn"},
                {"name": "harness-subagent"}
            ]
        })));
    }
}
