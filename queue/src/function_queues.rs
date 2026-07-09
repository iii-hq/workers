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
use crate::function_queue_id::physical_function_queue_name;
use crate::trigger::Invoker;

struct ActiveConsumer {
    function_id: String,
    physical_name: String,
    config: FunctionQueueConfig,
    task: JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionQueueStatus {
    pub function_id: String,
    pub consumer_count: u32,
    pub healthy: bool,
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
                tracing::info!(function_id = %name, "stopped removed function queue consumer");
            }
        }

        for (name, config) in queue_configs {
            let unchanged = self
                .active
                .lock()
                .await
                .get(name)
                .is_some_and(|active| active.config == *config && !active.task.is_finished());
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
        function_id: &str,
        config: &FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        let physical_name = physical_function_queue_name(function_id);
        self.adapter
            .setup_function_queue(&physical_name, config)
            .await
            .with_context(|| format!("setting up function queue '{function_id}'"))?;
        let prefetch = if config.r#type == "fifo" {
            1
        } else {
            config.concurrency
        };
        let receiver = self
            .adapter
            .consume_function_queue(&physical_name, prefetch)
            .await
            .with_context(|| format!("starting consumer for function queue '{function_id}'"))?;
        let task = spawn_consumer(
            self.adapter.clone(),
            self.invoker.clone(),
            function_id.to_string(),
            physical_name.clone(),
            config.clone(),
            receiver,
        );
        let previous = self.active.lock().await.insert(
            function_id.to_string(),
            ActiveConsumer {
                function_id: function_id.to_string(),
                physical_name,
                config: config.clone(),
                task,
            },
        );
        if let Some(previous) = previous {
            previous.task.abort();
        }
        tracing::info!(
            function_id = %function_id,
            concurrency = config.concurrency,
            max_retries = config.max_retries,
            "started function-bound queue consumer"
        );
        Ok(())
    }

    pub async fn enqueue(
        &self,
        function_id: &str,
        data: Value,
        message_id: &str,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) -> anyhow::Result<()> {
        let (config, physical_name) = self
            .active
            .lock()
            .await
            .get(function_id)
            .map(|active| (active.config.clone(), active.physical_name.clone()))
            .ok_or_else(|| anyhow::anyhow!("function queue '{function_id}' is not provisioned"))?;
        let priority = config.validate_payload(&data)?;
        self.adapter
            .publish_to_function_queue(
                &physical_name,
                function_id,
                data,
                message_id,
                config.max_retries,
                config.backoff_ms,
                traceparent,
                baggage,
                priority,
            )
            .await
            .with_context(|| format!("persisting message in function queue '{function_id}'"))
    }

    pub async fn statuses(&self) -> Vec<FunctionQueueStatus> {
        let mut statuses = self
            .active
            .lock()
            .await
            .values()
            .map(|active| FunctionQueueStatus {
                function_id: active.function_id.clone(),
                consumer_count: active.config.concurrency,
                healthy: !active.task.is_finished(),
            })
            .collect::<Vec<_>>();
        statuses.sort_by(|left, right| left.function_id.cmp(&right.function_id));
        statuses
    }

    pub async fn status(&self, function_id: &str) -> Option<FunctionQueueStatus> {
        self.active
            .lock()
            .await
            .get(function_id)
            .map(|active| FunctionQueueStatus {
                function_id: active.function_id.clone(),
                consumer_count: active.config.concurrency,
                healthy: !active.task.is_finished(),
            })
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
    function_id: String,
    physical_name: String,
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
            let function_id = function_id.clone();
            let physical_name = physical_name.clone();
            let max_retries = config.max_retries;
            tokio::spawn(async move {
                let delivery_id = message.delivery_id;
                let result = if message.function_id != function_id {
                    Err(format!(
                        "function queue envelope target '{}' does not match bound function '{}'",
                        message.function_id, function_id
                    ))
                } else {
                    invoker
                        .call(
                            &function_id,
                            message.data,
                            message.traceparent,
                            message.baggage,
                        )
                        .await
                        .map(|_| ())
                };
                match result {
                    Ok(()) => {
                        if let Err(error) = adapter
                            .ack_function_queue(&physical_name, delivery_id)
                            .await
                        {
                            tracing::error!(queue = %physical_name, delivery_id, error = %error, "failed to acknowledge function queue message");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            function_id = %function_id,
                            queue = %physical_name,
                            envelope_function_id = %message.function_id,
                            attempt = message.attempt,
                            max_retries,
                            error = %error,
                            "function queue message failed"
                        );
                        if let Err(nack_error) = adapter
                            .nack_function_queue(
                                &physical_name,
                                delivery_id,
                                message.attempt,
                                if message.function_id == function_id {
                                    max_retries
                                } else {
                                    message.attempt
                                },
                            )
                            .await
                        {
                            tracing::error!(queue = %physical_name, delivery_id, error = %nack_error, "failed to nack function queue message");
                        }
                    }
                }
                drop(permit);
            });
        }
        tracing::warn!(function_id = %function_id, queue = %physical_name, "function queue consumer ended");
    })
}
