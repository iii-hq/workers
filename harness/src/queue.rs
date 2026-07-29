//! Harness-owned durable queue provisioning.
//!
//! The `queue` worker does not infer named function queues from enqueue calls.
//! The harness therefore defines its turn queue after registering
//! `harness::turn`, but before binding lifecycle triggers or announcing ready.

use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub const TURN_QUEUE: &str = "harness-turn";

const DEFINE_QUEUE_FUNCTION_ID: &str = "queue::define";
const DEFINE_TIMEOUT_MS: u64 = 5_000;
const DEFINE_ATTEMPTS: u32 = 20;
const DEFINE_RETRY_BACKOFF_MS: u64 = 250;

/// Ensure the queue required by every `harness::turn` enqueue exists.
///
/// Retries cover the short worker-registration race during local stack boot.
/// Exhaustion is fatal to harness startup: accepting sends without this queue
/// would acknowledge work that cannot run durably.
pub async fn ensure_turn_queue(iii: &IIIClient) -> Result<(), String> {
    let payload = turn_queue_definition();
    let mut last_error = String::new();

    for attempt in 1..=DEFINE_ATTEMPTS {
        match iii
            .trigger(TriggerRequest {
                function_id: DEFINE_QUEUE_FUNCTION_ID.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(DEFINE_TIMEOUT_MS),
            })
            .await
        {
            Ok(_) => {
                tracing::info!(queue = TURN_QUEUE, "harness turn queue ready");
                return Ok(());
            }
            Err(error) => {
                last_error = error.to_string();
                if attempt < DEFINE_ATTEMPTS {
                    tracing::warn!(
                        queue = TURN_QUEUE,
                        attempt,
                        error = %last_error,
                        "queue definition failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(DEFINE_RETRY_BACKOFF_MS)).await;
                }
            }
        }
    }

    Err(format!(
        "{DEFINE_QUEUE_FUNCTION_ID} failed after {DEFINE_ATTEMPTS} attempts: {last_error}"
    ))
}

fn turn_queue_definition() -> Value {
    json!({
        "queue": TURN_QUEUE,
        "config": {
            "type": "fifo",
            "message_group_field": "session_id",
            "concurrency": 10,
            "max_retries": 3,
            "backoff_ms": 1_000,
            "poll_interval_ms": 100,
            "redeliver_on_engine_restart": true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_queue_is_grouped_fifo_and_uses_worker_timeout_default() {
        assert_eq!(
            turn_queue_definition(),
            json!({
                "queue": "harness-turn",
                "config": {
                    "type": "fifo",
                    "message_group_field": "session_id",
                    "concurrency": 10,
                    "max_retries": 3,
                    "backoff_ms": 1_000,
                    "poll_interval_ms": 100,
                    "redeliver_on_engine_restart": true
                }
            })
        );
        assert!(turn_queue_definition()["config"]
            .get("timeout_ms")
            .is_none());
    }
}
