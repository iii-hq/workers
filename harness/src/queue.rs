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

/// Provision both queues before recovery or readiness. Deletion REQUIRES
/// restart redelivery; only the pre-existing turn queue allows a legacy schema.
/// Both use the same bounded boot-readiness retry policy.
pub async fn ensure_turn_queue(iii: &IIIClient) -> Result<(), String> {
    ensure_queues_with(
        |payload| async move {
            iii.trigger(TriggerRequest {
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
    .await
}

async fn ensure_queues_with<F, Fut>(mut define: F, backoff: Duration) -> Result<(), String>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    for (payload, allow_legacy) in [
        (turn_queue_definition(), true),
        (deletion_queue_definition(), false),
    ] {
        ensure_queue_with(payload, allow_legacy, &mut define, backoff).await?;
    }
    Ok(())
}

async fn ensure_queue_with<F, Fut>(
    mut payload: Value,
    allow_legacy: bool,
    define: &mut F,
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
                tracing::info!(%queue, "harness queue ready");
                return Ok(());
            }
            Err(error) => {
                last_error = error;
                if is_legacy_queue_schema_error(&last_error)
                    && payload["config"]
                        .get("redeliver_on_engine_restart")
                        .is_some()
                {
                    if !allow_legacy {
                        return Err(format!("{queue} requires a queue worker supporting redeliver_on_engine_restart; cannot safely start deletion recovery: {last_error}"));
                    }
                    tracing::warn!(%queue, error = %last_error, "retrying turn queue with legacy schema");
                    if let Some(config) = payload["config"].as_object_mut() {
                        config.remove("redeliver_on_engine_restart");
                    }
                    continue;
                }
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

fn deletion_queue_definition() -> Value {
    json!({"queue": crate::functions::delete_session_tree::QUEUE, "config": {
        "type": "fifo", "message_group_field": "operation_id", "concurrency": 2,
        "max_retries": 3, "backoff_ms": 1_000, "poll_interval_ms": 100, "timeout_ms": 600_000,
        "redeliver_on_engine_restart": true
    }})
}

fn is_legacy_queue_schema_error(error: &str) -> bool {
    error.contains("unknown field `redeliver_on_engine_restart`")
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

    #[tokio::test]
    async fn both_queues_retry_readiness_before_success() {
        let mut attempts = std::collections::BTreeMap::new();
        ensure_queues_with(
            |payload| {
                let count = attempts
                    .entry(payload["queue"].as_str().unwrap().to_string())
                    .or_insert(0);
                *count += 1;
                std::future::ready(if *count < 3 {
                    Err("function_not_found: queue::define".into())
                } else {
                    Ok(())
                })
            },
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(attempts[TURN_QUEUE], 3);
        assert_eq!(attempts[crate::functions::delete_session_tree::QUEUE], 3);
    }

    #[tokio::test]
    async fn legacy_turn_fallback_does_not_silently_downgrade_deletion() {
        let mut seen = Vec::new();
        let error = ensure_queues_with(
            |payload| {
                let legacy = payload["config"]
                    .get("redeliver_on_engine_restart")
                    .is_none();
                seen.push(payload);
                std::future::ready(if legacy {
                    Ok(())
                } else {
                    Err("serialization error: unknown field `redeliver_on_engine_restart`".into())
                })
            },
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[1]["queue"], TURN_QUEUE);
        assert!(seen[1]["config"]
            .get("redeliver_on_engine_restart")
            .is_none());
        assert_eq!(seen[2]["config"]["redeliver_on_engine_restart"], true);
        assert!(error.contains("requires a queue worker supporting redeliver_on_engine_restart"));
    }

    #[tokio::test]
    async fn deletion_readiness_exhaustion_blocks_boot_with_named_error() {
        let mut calls = 0;
        let error = ensure_queues_with(
            |payload| {
                calls += 1;
                std::future::ready(if payload["queue"] == TURN_QUEUE {
                    Ok(())
                } else {
                    Err("not connected".into())
                })
            },
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert_eq!(calls, DEFINE_ATTEMPTS + 1);
        assert!(error.contains(crate::functions::delete_session_tree::QUEUE));
        assert!(error.contains("20 attempts"));
    }

    #[test]
    fn deletion_queue_matches_the_function_queue_config_surface() {
        let definition = deletion_queue_definition();
        assert_eq!(definition["config"]["type"], "fifo");
        assert_eq!(definition["config"]["message_group_field"], "operation_id");
        assert_eq!(definition["config"]["timeout_ms"], 600_000);
        assert_eq!(definition["config"]["redeliver_on_engine_restart"], true);
    }

    #[test]
    fn only_the_unsupported_restart_redelivery_field_triggers_fallback() {
        assert!(is_legacy_queue_schema_error(
            "serialization error: unknown field `redeliver_on_engine_restart`"
        ));
        assert!(!is_legacy_queue_schema_error(
            "serialization error: unknown field `message_group_field`"
        ));
    }
}
