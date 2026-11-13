//! Worker-owned queues. The queue worker refuses to enqueue onto an undefined
//! queue, so the workflow defines its tick and notify queues after registering
//! `workflow::tick` and before anything enqueues.

use std::time::Duration;

use iii_sdk::{CallRequest, IIIClient};
use serde_json::{json, Value};

pub const WORKFLOW_QUEUE: &str = "workflow";
/// Default queue for `notify` callbacks, apart from the ticks so a slow handler
/// cannot hold a tick slot.
pub const WORKFLOW_NOTIFY_QUEUE: &str = "workflow-notify";

const DEFINE_QUEUE_FUNCTION_ID: &str = "queue::define";
const DEFINE_TIMEOUT_MS: u64 = 5_000;
const DEFINE_ATTEMPTS: u32 = 20;
const DEFINE_RETRY_BACKOFF_MS: u64 = 250;

/// Define the `workflow` and `workflow-notify` queues, retrying while the queue
/// worker comes up.
pub async fn ensure_workflow_queue(iii: &IIIClient) -> Result<(), String> {
    for definition in [workflow_queue_definition(), notify_queue_definition()] {
        ensure_queue_with(
            definition,
            |payload| async move {
                iii.call(CallRequest {
                    function_id: DEFINE_QUEUE_FUNCTION_ID.into(),
                    payload,
                    action: None,
                    timeout_ms: Some(DEFINE_TIMEOUT_MS),
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
            },
            Duration::from_millis(DEFINE_RETRY_BACKOFF_MS),
        )
        .await?;
    }
    Ok(())
}

async fn ensure_queue_with<F, Fut>(
    payload: Value,
    mut define: F,
    backoff: Duration,
) -> Result<(), String>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let queue = payload["queue"].as_str().unwrap_or_default().to_string();
    let mut last_error = String::new();
    for attempt in 1..=DEFINE_ATTEMPTS {
        match define(payload.clone()).await {
            Ok(()) => {
                tracing::info!(%queue, "workflow queue ready");
                return Ok(());
            }
            Err(error) => {
                last_error = error;
                if attempt < DEFINE_ATTEMPTS {
                    tracing::warn!(%queue, attempt, error = %last_error, "queue definition failed; retrying readiness");
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(format!(
        "{DEFINE_QUEUE_FUNCTION_ID} {queue} failed after {DEFINE_ATTEMPTS} attempts: {last_error}"
    ))
}

fn workflow_queue_definition() -> Value {
    json!({
        "queue": WORKFLOW_QUEUE,
        "config": {
            "type": "fifo",
            "message_group_field": "run_id",
            "concurrency": 10,
            "max_retries": 3,
            "backoff_ms": 1_000,
            // A tick in flight across an engine restart would otherwise hold its run's
            // group (and a slot) for the full queue timeout; re-running a tick is safe.
            "redeliver_on_engine_restart": true
        }
    })
}

/// Standard, not fifo: the notify handler dedups on `run_id` and needs no order.
fn notify_queue_definition() -> Value {
    json!({
        "queue": WORKFLOW_NOTIFY_QUEUE,
        "config": {
            "type": "standard",
            "concurrency": 10,
            "max_retries": 3,
            "backoff_ms": 1_000
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_queue_is_fifo_grouped_by_run_id() {
        assert_eq!(
            workflow_queue_definition(),
            json!({
                "queue": "workflow",
                "config": {
                    "type": "fifo",
                    "message_group_field": "run_id",
                    "concurrency": 10,
                    "max_retries": 3,
                    "backoff_ms": 1_000,
                    "redeliver_on_engine_restart": true
                }
            })
        );
    }

    #[test]
    fn notify_queue_is_separate_and_standard() {
        assert_eq!(
            notify_queue_definition(),
            json!({
                "queue": "workflow-notify",
                "config": {
                    "type": "standard",
                    "concurrency": 10,
                    "max_retries": 3,
                    "backoff_ms": 1_000
                }
            })
        );
    }

    #[tokio::test]
    async fn define_retries_until_ready() {
        let mut calls = 0;
        ensure_queue_with(
            workflow_queue_definition(),
            |payload| {
                calls += 1;
                assert_eq!(payload["queue"], WORKFLOW_QUEUE);
                std::future::ready(if calls < 3 {
                    Err("function_not_found: queue::define".into())
                } else {
                    Ok(())
                })
            },
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(calls, 3);

        let mut calls = 0;
        let error = ensure_queue_with(
            notify_queue_definition(),
            |_| {
                calls += 1;
                std::future::ready(Err("not connected".into()))
            },
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert_eq!(calls, DEFINE_ATTEMPTS);
        assert!(
            error.contains("workflow-notify failed after 20 attempts"),
            "{error}"
        );
    }
}
