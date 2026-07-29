//! Per-subscriber consumer loop: pulls deliveries off a function's queue,
//! invokes the target function (through an optional condition-function
//! gate), and hands the result to the [`RetryHandler`] for ack/requeue/DLQ.
//!
//! Port of `engine/src/workers/queue/adapters/rabbitmq/worker.rs`. Seams
//! applied for the standalone worker:
//! - `Arc<Engine>` and `engine.call(...)` become `Arc<dyn
//!   crate::trigger::Invoker>` and `invoker.call(function_id, payload)`
//!   (same seam as every other adapter in this crate, e.g.
//!   `adapters/redis.rs`).
//! - The engine's `condition::check_condition` helper and its telemetry span
//!   (`tracing::info_span!` + OTel `SpanExt::with_parent_headers` +
//!   `Instrument`) are dropped; `check_condition` is reimplemented locally
//!   against `Invoker` with identical semantics (same duplication pattern as
//!   `adapters/redis.rs::check_condition`) and the worker emits plain
//!   `tracing` events instead of a parented span.

#![cfg(feature = "rabbitmq")]

use std::sync::Arc;

use futures::StreamExt;
use lapin::{message::Delivery, options::*, Channel};
use serde_json::Value;
use tokio::{
    sync::{oneshot, Semaphore},
    task::JoinSet,
};

use crate::trigger::Invoker;

use super::consumer::JobParser;
use super::retry::RetryHandler;
use super::types::{Job, QueueMode};

/// Port of the engine's `check_condition` (`engine/src/condition.rs`) against
/// `Invoker` instead of `EngineTrait` — same semantics as
/// `adapters/redis.rs::check_condition`: `Ok(Some(v))` proceeds unless `v` is
/// the JSON boolean `false`, `Ok(None)` proceeds (with a warn log), `Err`
/// propagates the invocation failure.
async fn check_condition(
    invoker: &dyn Invoker,
    condition_function_id: &str,
    data: Value,
) -> Result<bool, String> {
    match invoker.call(condition_function_id, data).await {
        Ok(Some(result)) => Ok(result.as_bool() != Some(false)),
        Ok(None) => {
            tracing::warn!(
                condition_function_id = %condition_function_id,
                "Condition function returned no result"
            );
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

pub struct Worker {
    channel: Arc<Channel>,
    retry_handler: Arc<RetryHandler>,
    invoker: Arc<dyn Invoker>,
    semaphore: Option<Arc<Semaphore>>,
    queue_mode: QueueMode,
    prefetch_count: u16,
}

impl Worker {
    pub fn new(
        channel: Arc<Channel>,
        retry_handler: Arc<RetryHandler>,
        invoker: Arc<dyn Invoker>,
        queue_mode: QueueMode,
        prefetch_count: u16,
    ) -> Self {
        let semaphore = match queue_mode {
            QueueMode::Fifo => None,
            QueueMode::Standard => Some(Arc::new(Semaphore::new(prefetch_count as usize))),
        };

        Self {
            channel,
            retry_handler,
            invoker,
            semaphore,
            queue_mode,
            prefetch_count,
        }
    }

    pub async fn run(
        self: Arc<Self>,
        topic: String,
        subscription_id: String,
        function_id: String,
        metadata: Option<Value>,
        condition_function_id: Option<String>,
        consumer_tag: String,
        queue_name: String,
        mut cancelled: oneshot::Receiver<()>,
    ) {
        // Apply per-consumer prefetch (QoS) so the broker keeps a backlog in the
        // queue instead of shipping everything to this consumer at once. Without
        // it a priority subscriber queue can't reorder (the broker only orders
        // messages it has yet to deliver), and the queue loses backpressure. The
        // function-queue consumer sets QoS the same way.
        if let Err(e) = self
            .channel
            .basic_qos(self.prefetch_count, BasicQosOptions::default())
            .await
        {
            tracing::error!(topic = %topic, error = ?e, "Failed to set consumer QoS");
            return;
        }

        let mut consumer = match self
            .channel
            .basic_consume(
                &queue_name,
                &consumer_tag,
                BasicConsumeOptions::default(),
                lapin::types::FieldTable::default(),
            )
            .await
        {
            Ok(consumer) => consumer,
            Err(e) => {
                tracing::error!(
                    topic = %topic,
                    error = ?e,
                    "Failed to create consumer"
                );
                return;
            }
        };

        let mut tasks = JoinSet::new();
        let mut explicitly_cancelled = loop {
            while tasks.try_join_next().is_some() {}
            let delivery_result = tokio::select! {
                biased;
                _ = &mut cancelled => break true,
                delivery = consumer.next() => match delivery {
                    Some(delivery) => delivery,
                    None => break false,
                },
            };

            match delivery_result {
                Ok(delivery) => {
                    let worker = Arc::clone(&self);
                    let topic_clone = topic.clone();
                    let subscription_id_clone = subscription_id.clone();
                    let function_id_clone = function_id.clone();
                    let metadata_clone = metadata.clone();
                    let condition_function_id_clone = condition_function_id.clone();

                    match self.queue_mode {
                        QueueMode::Fifo => {
                            let result = tokio::select! {
                                biased;
                                _ = &mut cancelled => break true,
                                result = worker.process_delivery(
                                    delivery,
                                    &topic_clone,
                                    &subscription_id_clone,
                                    &function_id_clone,
                                    metadata_clone.as_ref(),
                                    condition_function_id_clone.as_deref(),
                                ) => result,
                            };
                            if let Err(e) = result {
                                tracing::error!(
                                    topic = %topic_clone,
                                    error = ?e,
                                    "Failed to process delivery"
                                );
                            }
                        }
                        QueueMode::Standard => {
                            let semaphore = self.semaphore.as_ref().map(Arc::clone);
                            tasks.spawn(async move {
                                let _permit = if let Some(ref sem) = semaphore {
                                    Some(sem.acquire().await.unwrap())
                                } else {
                                    None
                                };

                                if let Err(e) = worker
                                    .process_delivery(
                                        delivery,
                                        &topic_clone,
                                        &subscription_id_clone,
                                        &function_id_clone,
                                        metadata_clone.as_ref(),
                                        condition_function_id_clone.as_deref(),
                                    )
                                    .await
                                {
                                    tracing::error!(
                                        topic = %topic_clone,
                                        error = ?e,
                                        "Failed to process delivery"
                                    );
                                }
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        topic = %topic,
                        error = ?e,
                        "Error receiving delivery"
                    );
                }
            }
        };

        if explicitly_cancelled {
            tasks.abort_all();
        } else {
            while !tasks.is_empty() {
                tokio::select! {
                    biased;
                    _ = &mut cancelled => {
                        explicitly_cancelled = true;
                        tasks.abort_all();
                        break;
                    }
                    _ = tasks.join_next() => {}
                }
            }
        }
        while tasks.join_next().await.is_some() {}

        if explicitly_cancelled {
            tracing::debug!(topic = %topic, "Consumer cancelled");
        } else {
            tracing::warn!(topic = %topic, "Consumer stream ended");
        }
    }

    async fn process_delivery(
        &self,
        delivery: Delivery,
        topic: &str,
        subscription_id: &str,
        function_id: &str,
        metadata: Option<&Value>,
        condition_function_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut job = JobParser::parse_from_delivery(&delivery)?;

        match self
            .process_job(&job, function_id, metadata, condition_function_id)
            .await
        {
            Ok(_) => {
                delivery
                    .ack(BasicAckOptions::default())
                    .await
                    .map_err(|e| format!("Failed to ack message: {}", e))?;

                self.retry_handler
                    .handle_success(topic, &job)
                    .await
                    .map_err(|e| format!("Failed to handle success: {}", e))?;
            }
            Err(e) => {
                delivery
                    .nack(BasicNackOptions {
                        requeue: false,
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| format!("Failed to nack message: {}", e))?;

                self.retry_handler
                    .handle_failure(topic, &mut job, &format!("{:?}", e), Some(subscription_id))
                    .await
                    .map_err(|e| format!("Failed to handle failure: {}", e))?;
            }
        }

        Ok(())
    }

    async fn process_job(
        &self,
        job: &Job,
        function_id: &str,
        metadata: Option<&Value>,
        condition_function_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let invoker = Arc::clone(&self.invoker);
        let data = job.data.clone();

        if let Some(condition_path) = condition_function_id {
            tracing::debug!(
                condition_function_id = %condition_path,
                "Checking trigger conditions"
            );
            match check_condition(invoker.as_ref(), condition_path, data.clone()).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::debug!(
                        function_id = %function_id,
                        "Condition check failed, skipping handler"
                    );
                    return Ok(());
                }
                Err(err) => {
                    tracing::error!(
                        condition_function_id = %condition_path,
                        error = %err,
                        "Error invoking condition function"
                    );
                    return Err(format!("Condition function error: {:?}", err).into());
                }
            }
        }

        match invoker
            .call_delivery(function_id, data, metadata.cloned())
            .await
        {
            Ok(_) => {
                tracing::debug!(job_id = %job.id, "Job processed successfully");
                Ok(())
            }
            Err(e) => {
                tracing::error!(job_id = %job.id, error = ?e, "Job processing failed");
                Err(format!("Job processing error: {:?}", e).into())
            }
        }
    }
}
