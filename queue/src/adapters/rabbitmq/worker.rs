//! Per-subscriber consumer loop: pulls deliveries off a subscriber's queue,
//! hands each one to its `queue::on-message` observer, and passes the result
//! to the [`RetryHandler`] for ack/requeue/DLQ.
//!
//! Port of `engine/src/workers/queue/adapters/rabbitmq/worker.rs`. Seams
//! applied for the standalone worker:
//! - `Arc<Engine>` and `engine.call(...)` become `Arc<dyn
//!   crate::subscribers::Invoker>` and [`Subscriber::deliver`] (same seam as
//!   every other adapter in this crate, e.g. `adapters/redis.rs`).
//! - The engine's telemetry span (`tracing::info_span!` + OTel
//!   `SpanExt::with_parent_headers` + `Instrument`) is dropped; the worker
//!   emits plain `tracing` events instead of a parented span.

#![cfg(feature = "rabbitmq")]

use std::sync::Arc;

use futures::StreamExt;
use lapin::{message::Delivery, options::*, Channel};
use tokio::{
    sync::{oneshot, Semaphore},
    task::JoinSet,
};

use crate::subscribers::{Invoker, Subscriber};

use super::consumer::JobParser;
use super::retry::RetryHandler;
use super::types::{Job, QueueMode};

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

    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        self: Arc<Self>,
        topic: String,
        subscription_id: String,
        subscriber: Subscriber,
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
        let explicitly_cancelled = loop {
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
                    match self.queue_mode {
                        QueueMode::Fifo => {
                            // Processed inline and to completion: cancellation
                            // is honored at the next loop head, never
                            // mid-invocation — an aborted invocation's side
                            // effects would apply again on redelivery.
                            if let Err(e) = self
                                .process_delivery(delivery, &topic, &subscription_id, &subscriber)
                                .await
                            {
                                tracing::error!(
                                    topic = %topic,
                                    error = ?e,
                                    "Failed to process delivery"
                                );
                            }
                        }
                        QueueMode::Standard => {
                            let worker = Arc::clone(&self);
                            let topic_clone = topic.clone();
                            let subscription_id_clone = subscription_id.clone();
                            let subscriber_clone = subscriber.clone();
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
                                        &subscriber_clone,
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

        // Drain: every in-flight delivery completes its invocation and acks,
        // nacks, or republishes. Aborting here would kill invocations whose
        // side effects may already have applied server-side, so the broker's
        // redelivery would execute them a second time.
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
        subscriber: &Subscriber,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut job = JobParser::parse_from_delivery(&delivery)?;

        match self.process_job(&job, subscriber).await {
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
        subscriber: &Subscriber,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let invoker = Arc::clone(&self.invoker);
        let data = job.data.clone();

        match subscriber.deliver(invoker.as_ref(), data).await {
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
