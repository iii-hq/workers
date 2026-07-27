use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, TriggerAction};
use serde_json::{json, Value};

use crate::contract::StepRequestV1;
use crate::error::EvalError;

pub const RUN_QUEUE: &str = "eval-run";
const DEFINE_TIMEOUT_MS: u64 = 5_000;
const DEFINE_ATTEMPTS: u32 = 20;
const DEFINE_RETRY_BACKOFF_MS: u64 = 250;

pub async fn ensure_run_queue(iii: &IIIClient) -> Result<(), EvalError> {
    let payload = run_queue_definition();
    let mut last_error = String::new();
    for attempt in 1..=DEFINE_ATTEMPTS {
        match iii
            .trigger(TriggerRequest {
                function_id: "queue::define".into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(DEFINE_TIMEOUT_MS),
            })
            .await
        {
            Ok(_) => return Ok(()),
            Err(error) => {
                last_error = error.to_string();
                if attempt < DEFINE_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(DEFINE_RETRY_BACKOFF_MS)).await;
                }
            }
        }
    }
    Err(EvalError::Dependency(format!(
        "queue::define failed after {DEFINE_ATTEMPTS} attempts: {last_error}"
    )))
}

pub async fn enqueue_step(
    iii: &IIIClient,
    evaluation_id: &str,
    step: u64,
) -> Result<(), EvalError> {
    let payload = serde_json::to_value(StepRequestV1 {
        evaluation_id: evaluation_id.into(),
        step,
    })?;
    iii.trigger(TriggerRequest {
        function_id: "eval::step".into(),
        payload,
        action: Some(TriggerAction::Enqueue {
            queue: RUN_QUEUE.into(),
        }),
        timeout_ms: None,
    })
    .await
    .map(|_| ())
    .map_err(|error| EvalError::Dependency(format!("enqueue eval::step: {error}")))
}

fn run_queue_definition() -> Value {
    json!({
        "queue": RUN_QUEUE,
        "config": {
            "type": "fifo",
            "message_group_field": "evaluation_id",
            "concurrency": 4,
            "max_retries": 3,
            "backoff_ms": 1_000,
            "poll_interval_ms": 100
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_queue_is_fifo_and_grouped_by_evaluation() {
        let definition = run_queue_definition();
        assert_eq!(definition["queue"], "eval-run");
        assert_eq!(definition["config"]["message_group_field"], "evaluation_id");
        assert_eq!(definition["config"]["type"], "fifo");
    }
}
