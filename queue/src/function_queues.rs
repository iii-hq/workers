//! Runtime for named function queues used by engine `TriggerAction::Enqueue`.
//!
//! Unlike durable topics, a function queue carries its target function id with
//! each job. The queue worker owns its consumers; callers only provision a
//! named queue and enqueue a normal function invocation through the engine.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::Context;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;

use crate::adapter::{FunctionQueueConfig, QueueAdapter, QueueMessage, SwappableAdapter};
use crate::trigger::Invoker;

struct ActiveConsumer {
    config: FunctionQueueConfig,
    task: JoinHandle<()>,
}

/// Owns one delivery loop per configured named function queue.
pub struct FunctionQueueRuntime {
    adapter: Arc<SwappableAdapter>,
    invoker: Arc<dyn Invoker>,
    active: Mutex<HashMap<String, ActiveConsumer>>,
    operation_lock: Mutex<()>,
}

impl FunctionQueueRuntime {
    pub fn new(adapter: Arc<SwappableAdapter>, invoker: Arc<dyn Invoker>) -> Self {
        Self {
            adapter,
            invoker,
            active: Mutex::new(HashMap::new()),
            operation_lock: Mutex::new(()),
        }
    }

    /// Start added queues, restart changed queues, and stop removed queues.
    pub async fn reconcile(
        &self,
        queue_configs: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<()> {
        let _operation = self.operation_lock.lock().await;
        for (name, config) in queue_configs {
            config
                .validate()
                .with_context(|| format!("invalid function queue '{name}'"))?;
        }

        let removed = {
            let active = self.active.lock().await;
            active
                .keys()
                .filter(|name| !queue_configs.contains_key(*name))
                .cloned()
                .collect::<Vec<_>>()
        };
        for name in removed {
            if let Some(previous) = self.active.lock().await.remove(&name) {
                previous.task.abort();
                tracing::info!(queue = %name, "stopped removed function queue consumer");
            }
        }

        for (name, config) in queue_configs {
            let unchanged = self
                .active
                .lock()
                .await
                .get(name)
                .is_some_and(|active| active.config == *config);
            if !unchanged {
                self.start_consumer(name, config).await?;
            }
        }
        Ok(())
    }

    /// Recreate every consumer after an adapter replacement.
    pub async fn restart_all(
        &self,
        queue_configs: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<()> {
        let _operation = self.operation_lock.lock().await;
        let active = std::mem::take(&mut *self.active.lock().await);
        for (_, consumer) in active {
            consumer.task.abort();
        }
        for (name, config) in queue_configs {
            config
                .validate()
                .with_context(|| format!("invalid function queue '{name}'"))?;
            self.start_consumer(name, config).await?;
        }
        Ok(())
    }

    async fn start_consumer(
        &self,
        queue_name: &str,
        config: &FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        self.adapter
            .setup_function_queue(queue_name, config)
            .await
            .with_context(|| format!("setting up function queue '{queue_name}'"))?;
        let receiver = self
            .adapter
            .consume_function_queue(queue_name, config.concurrency)
            .await
            .with_context(|| format!("starting consumer for function queue '{queue_name}'"))?;
        let task = spawn_consumer(
            self.adapter.clone(),
            self.invoker.clone(),
            queue_name.to_string(),
            config.clone(),
            receiver,
        );
        let previous = self.active.lock().await.insert(
            queue_name.to_string(),
            ActiveConsumer {
                config: config.clone(),
                task,
            },
        );
        if let Some(previous) = previous {
            previous.task.abort();
        }
        tracing::info!(
            queue = %queue_name,
            concurrency = config.concurrency,
            max_retries = config.max_retries,
            "started named function queue consumer"
        );
        Ok(())
    }

    pub async fn enqueue(
        &self,
        queue_name: &str,
        function_id: &str,
        data: Value,
        message_id: &str,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) -> anyhow::Result<()> {
        let config = self
            .active
            .lock()
            .await
            .get(queue_name)
            .map(|active| active.config.clone())
            .ok_or_else(|| anyhow::anyhow!("function queue '{queue_name}' is not provisioned"))?;
        self.adapter
            .publish_to_function_queue(
                queue_name,
                function_id,
                data,
                message_id,
                config.max_retries,
                config.backoff_ms,
                traceparent,
                baggage,
                None,
            )
            .await
            .with_context(|| format!("persisting message in function queue '{queue_name}'"))
    }

    pub async fn shutdown(&self) {
        let active = std::mem::take(&mut *self.active.lock().await);
        for (_, consumer) in active {
            consumer.task.abort();
        }
    }
}

fn spawn_consumer(
    adapter: Arc<SwappableAdapter>,
    invoker: Arc<dyn Invoker>,
    queue_name: String,
    config: FunctionQueueConfig,
    mut receiver: mpsc::Receiver<QueueMessage>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let semaphore = Arc::new(Semaphore::new(config.concurrency as usize));
        while let Some(message) = receiver.recv().await {
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };
            let adapter = adapter.clone();
            let invoker = invoker.clone();
            let queue_name = queue_name.clone();
            let max_retries = config.max_retries;
            tokio::spawn(async move {
                let delivery_id = message.delivery_id;
                let result = if message.function_id.is_empty() {
                    Err("function queue message has no function id".to_string())
                } else {
                    invoker
                        .call(
                            &message.function_id,
                            message.data,
                            message.traceparent,
                            message.baggage,
                        )
                        .await
                        .map(|_| ())
                };
                match result {
                    Ok(()) => {
                        if let Err(error) =
                            adapter.ack_function_queue(&queue_name, delivery_id).await
                        {
                            tracing::error!(queue = %queue_name, delivery_id, error = %error, "failed to acknowledge function queue message");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            queue = %queue_name,
                            function_id = %message.function_id,
                            attempt = message.attempt,
                            max_retries,
                            error = %error,
                            "function queue message failed"
                        );
                        if let Err(nack_error) = adapter
                            .nack_function_queue(
                                &queue_name,
                                delivery_id,
                                message.attempt,
                                max_retries,
                            )
                            .await
                        {
                            tracing::error!(queue = %queue_name, delivery_id, error = %nack_error, "failed to nack function queue message");
                        }
                    }
                }
                drop(permit);
            });
        }
        tracing::warn!(queue = %queue_name, "function queue consumer ended");
    })
}
