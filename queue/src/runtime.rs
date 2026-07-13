//! Durable named function-queue lifecycle and provider functions.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;
use tracing::Instrument;

use crate::adapter::{FunctionQueueConfig, QueueAdapter, QueueMessage, SwappableAdapter};
use crate::boot::{ApplyLock, ConfigCell};
use crate::config::QueueConfig;
use crate::trigger::{Invoker, QueueTriggerHandler};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefineQueueInput {
    pub queue: String,
    #[serde(default)]
    pub config: FunctionQueueConfig,
    /// Internal metadata injected by the engine when dispatching a worker
    /// function. It is accepted on the wire but intentionally omitted from
    /// the published function schema and never affects queue behavior.
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DefineQueueOutput {
    pub queue: String,
    pub changed: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnqueueInput {
    pub queue: String,
    pub function_id: String,
    pub data: Value,
    #[serde(rename = "messageReceiptId")]
    pub message_receipt_id: String,
    /// Trace context captured by the engine at the enqueue boundary and
    /// restored when the queued function is invoked.
    #[serde(default)]
    pub traceparent: Option<String>,
    #[serde(default)]
    pub baggage: Option<String>,
    /// Internal metadata injected by the engine; see [`DefineQueueInput`].
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct EnqueueOutput {
    #[serde(rename = "messageReceiptId")]
    pub message_receipt_id: String,
}

struct ConsumerHandle {
    task: JoinHandle<()>,
}

impl ConsumerHandle {
    fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    async fn abort(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

type PendingConsumers = BTreeMap<String, ConsumerHandle>;

#[derive(Clone)]
pub struct FunctionQueueRuntime {
    iii: Arc<iii_sdk::IIIClient>,
    adapter: Arc<SwappableAdapter>,
    invoker: Arc<dyn Invoker>,
    config: ConfigCell,
    apply_lock: ApplyLock,
    consumers: Arc<Mutex<BTreeMap<String, ConsumerHandle>>>,
}

impl FunctionQueueRuntime {
    pub fn new(
        iii: Arc<iii_sdk::IIIClient>,
        adapter: Arc<SwappableAdapter>,
        invoker: Arc<dyn Invoker>,
        config: ConfigCell,
        apply_lock: ApplyLock,
    ) -> Self {
        Self {
            iii,
            adapter,
            invoker,
            config,
            apply_lock,
            consumers: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let _guard = self.apply_lock.write().await;
        let config = self.config.read().await.clone();
        let adapter = self.adapter.current().await;
        let pending = self
            .setup_and_start_consumers(adapter, &config.queue_configs)
            .await?;
        self.replace_all_consumers(pending).await?;
        Ok(())
    }

    pub async fn config_snapshot(&self) -> Arc<QueueConfig> {
        self.config.read().await.clone()
    }

    pub async fn shutdown(&self) {
        let adapter = self.adapter.current().await;
        if let Err(error) = self.stop_all_consumers(adapter).await {
            tracing::error!(error = %error, "failed to stop named queue consumers cleanly");
        }
    }

    pub async fn define(&self, input: DefineQueueInput) -> Result<DefineQueueOutput, Error> {
        let queue = input.queue.trim().to_string();
        input.config.validate(&queue).map_err(Error::Handler)?;

        let _guard = self.apply_lock.write().await;
        let current = self.config.read().await.clone();
        let authoritative = crate::configuration::fetch_config(&self.iii)
            .await
            .map_err(|err| Error::Handler(format!("cannot read queue configuration: {err}")))?;
        if current.as_ref() != &authoritative {
            return Err(Error::Handler(
                "queue configuration is still being applied; retry queue::define".to_string(),
            ));
        }
        let changed = authoritative.queue_configs.get(&queue) != Some(&input.config);
        let finished = {
            let consumers = self.consumers.lock().await;
            consumers
                .get(&queue)
                .is_some_and(ConsumerHandle::is_finished)
        };
        let adapter = self.adapter.current().await;
        if finished {
            if let Some(handle) = self.consumers.lock().await.remove(&queue) {
                self.stop_consumer(adapter.clone(), &queue, handle)
                    .await
                    .map_err(|error| {
                        Error::Handler(format!(
                            "cannot restart finished queue '{queue}' consumer: {error}"
                        ))
                    })?;
            }
        }
        let already_running = self.consumers.lock().await.contains_key(&queue);
        if !changed && already_running {
            return Ok(DefineQueueOutput {
                queue,
                changed: false,
            });
        }

        let previous_definition = authoritative.queue_configs.get(&queue).cloned();
        validate_queue_update(
            &self.adapter.current_name().await,
            &queue,
            previous_definition.as_ref(),
            &input.config,
        )
        .map_err(Error::Handler)?;

        self.setup_consumer(adapter.clone(), &queue, &input.config)
            .await
            .map_err(|err| Error::Handler(format!("cannot define queue '{queue}': {err}")))?;

        if !changed {
            let consumer = self
                .start_consumer(adapter, &queue, &input.config)
                .await
                .map_err(|err| Error::Handler(format!("cannot start queue '{queue}': {err}")))?;
            self.replace_consumer(queue.clone(), consumer)
                .await
                .map_err(|err| Error::Handler(format!("cannot install queue '{queue}': {err}")))?;
            return Ok(DefineQueueOutput {
                queue,
                changed: false,
            });
        }

        let latest = match crate::configuration::fetch_config(&self.iii).await {
            Ok(latest) => latest,
            Err(error) => {
                let restore_error = self
                    .restore_definition(adapter.clone(), &queue, previous_definition.as_ref())
                    .await
                    .err();
                let mut message = format!(
                    "cannot recheck queue configuration before defining '{queue}': {error}"
                );
                if let Some(error) = restore_error {
                    message.push_str(&format!("; adapter definition rollback failed: {error}"));
                }
                return Err(Error::Handler(message));
            }
        };
        if latest != authoritative {
            let restore_error = self
                .restore_definition(adapter.clone(), &queue, previous_definition.as_ref())
                .await
                .err();
            let mut message = format!(
                "queue configuration changed while defining '{queue}'; retry queue::define"
            );
            if let Some(error) = restore_error {
                message.push_str(&format!("; adapter definition rollback failed: {error}"));
            }
            return Err(Error::Handler(message));
        }
        let mut next = authoritative.clone();
        next.queue_configs
            .insert(queue.clone(), input.config.clone());
        if let Err(error) = crate::configuration::persist_config(&self.iii, &next).await {
            let restore_error = self
                .restore_definition(adapter.clone(), &queue, previous_definition.as_ref())
                .await
                .err();
            let mut message = format!("cannot persist queue '{queue}' definition: {error}");
            if let Some(error) = restore_error {
                message.push_str(&format!("; adapter definition rollback failed: {error}"));
            }
            return Err(Error::Handler(message));
        }

        let previous_consumer = self.consumers.lock().await.remove(&queue);
        if let Some(handle) = previous_consumer {
            if let Err(stop_error) = self.stop_consumer(adapter.clone(), &queue, handle).await {
                let rollback_error =
                    crate::configuration::persist_config(&self.iii, &authoritative)
                        .await
                        .err();
                let restore_result = self
                    .restore_definition(adapter.clone(), &queue, previous_definition.as_ref())
                    .await;
                let restart_error = if restore_result.is_ok() {
                    if let Some(previous) = previous_definition.as_ref() {
                        match self.start_consumer(adapter.clone(), &queue, previous).await {
                            Ok(consumer) => {
                                self.replace_consumer(queue.clone(), consumer).await.err()
                            }
                            Err(error) => Some(error),
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mut message = format!("cannot stop queue '{queue}' consumer: {stop_error}");
                if let Some(error) = rollback_error {
                    message.push_str(&format!("; configuration rollback failed: {error}"));
                }
                if let Err(error) = restore_result {
                    message.push_str(&format!("; adapter definition rollback failed: {error}"));
                }
                if let Some(error) = restart_error {
                    message.push_str(&format!("; previous consumer restart failed: {error}"));
                }
                return Err(Error::Handler(message));
            }
        }

        match self
            .start_consumer(adapter.clone(), &queue, &input.config)
            .await
        {
            Ok(consumer) => {
                *self.config.write().await = Arc::new(next);
                self.replace_consumer(queue.clone(), consumer)
                    .await
                    .map_err(|err| {
                        Error::Handler(format!("cannot install queue '{queue}': {err}"))
                    })?;
                Ok(DefineQueueOutput {
                    queue,
                    changed: true,
                })
            }
            Err(start_error) => {
                let rollback_error =
                    crate::configuration::persist_config(&self.iii, &authoritative)
                        .await
                        .err();
                let restart_error = if let Some(previous) = previous_definition.as_ref() {
                    match self.setup_consumer(adapter.clone(), &queue, previous).await {
                        Ok(()) => match self.start_consumer(adapter, &queue, previous).await {
                            Ok(consumer) => {
                                self.replace_consumer(queue.clone(), consumer).await.err()
                            }
                            Err(error) => Some(error),
                        },
                        Err(error) => Some(error),
                    }
                } else {
                    self.restore_definition(adapter, &queue, None).await.err()
                };

                let mut message = format!("cannot start queue '{queue}': {start_error}");
                if let Some(error) = rollback_error {
                    message.push_str(&format!("; configuration rollback failed: {error}"));
                }
                if let Some(error) = restart_error {
                    message.push_str(&format!("; previous consumer restart failed: {error}"));
                }
                Err(Error::Handler(message))
            }
        }
    }

    pub async fn enqueue(&self, input: EnqueueInput) -> Result<EnqueueOutput, Error> {
        let queue = input.queue.trim();
        if queue.is_empty() {
            return Err(Error::Handler("queue is required".to_string()));
        }
        if input.function_id.trim().is_empty() {
            return Err(Error::Handler("function_id is required".to_string()));
        }
        if input.message_receipt_id.trim().is_empty() {
            return Err(Error::Handler("messageReceiptId is required".to_string()));
        }

        // Keep the config snapshot and transport stable through publication,
        // while still allowing unrelated enqueue calls to run concurrently.
        let _guard = self.apply_lock.read().await;
        let snapshot = self.config.read().await.clone();
        let config = snapshot.queue_configs.get(queue).ok_or_else(|| {
            Error::Handler(format!(
                "queue '{queue}' is not defined; call queue::define before enqueueing"
            ))
        })?;
        let priority = priority_from_data(&input.data, config.priority_field.as_deref());
        self.adapter
            .publish_to_function_queue(
                queue,
                &input.function_id,
                input.data,
                &input.message_receipt_id,
                config.max_retries,
                config.backoff_ms,
                input.traceparent,
                input.baggage,
                priority,
            )
            .await
            .map_err(|err| Error::Handler(format!("enqueue to '{queue}' failed: {err}")))?;

        Ok(EnqueueOutput {
            message_receipt_id: input.message_receipt_id,
        })
    }

    /// Apply an authoritative configuration snapshot. When `replacement` is
    /// present, all consumers are prepared on it before the live transport is
    /// swapped, preserving the previous working adapter on setup failure.
    pub async fn apply_config(
        &self,
        next: QueueConfig,
        replacement: Option<(Arc<dyn QueueAdapter>, String)>,
        trigger_handler: &QueueTriggerHandler,
    ) -> anyhow::Result<()> {
        let _guard = self.apply_lock.write().await;
        self.apply_config_locked(next, replacement, trigger_handler)
            .await
    }

    /// Re-read and apply the newest stored configuration while holding the
    /// same lock used by `queue::define`. Fetching under the lock prevents an
    /// older configuration event from applying after a newer definition.
    pub async fn refresh_config(
        &self,
        trigger_handler: &QueueTriggerHandler,
    ) -> anyhow::Result<()> {
        let _guard = self.apply_lock.write().await;
        let next = crate::configuration::fetch_config(&self.iii)
            .await
            .map_err(anyhow::Error::msg)?;
        let current = self.config.read().await.clone();
        let replacement = if crate::configuration::swap_needed(&current, &next) {
            let adapter = crate::boot::build_adapter(&next, self.invoker.clone()).await?;
            Some((
                adapter,
                crate::boot::adapter_identity_name(&next).to_string(),
            ))
        } else {
            None
        };
        self.apply_config_locked(next, replacement, trigger_handler)
            .await
    }

    async fn apply_config_locked(
        &self,
        next: QueueConfig,
        replacement: Option<(Arc<dyn QueueAdapter>, String)>,
        trigger_handler: &QueueTriggerHandler,
    ) -> anyhow::Result<()> {
        next.validate().map_err(anyhow::Error::msg)?;
        let old = self.config.read().await.clone();

        if replacement.is_none() && old.as_ref() == &next {
            return Ok(());
        }

        if let Some((new_adapter, name)) = replacement {
            let old_adapter = self.adapter.current().await;
            self.setup_consumers(new_adapter.clone(), &next.queue_configs)
                .await?;
            if let Err(error) = self.stop_all_consumers(old_adapter.clone()).await {
                let restart = self
                    .setup_and_start_consumers(old_adapter, &old.queue_configs)
                    .await;
                let restart_summary = result_summary(&restart);
                if let Ok(consumers) = restart {
                    self.replace_all_consumers(consumers).await?;
                }
                anyhow::bail!(
                    "cannot stop previous queue consumers before adapter replacement: {error}; consumer restart: {restart_summary}"
                );
            }
            self.adapter.replace(new_adapter.clone(), name).await;

            match self
                .start_consumers(new_adapter.clone(), &next.queue_configs)
                .await
            {
                Ok(pending) => {
                    *self.config.write().await = Arc::new(next);
                    self.replace_all_consumers(pending).await?;
                    trigger_handler.resubscribe_all().await;
                    old_adapter.shutdown().await;
                    return Ok(());
                }
                Err(error) => {
                    self.adapter
                        .replace(
                            old_adapter.clone(),
                            crate::boot::adapter_identity_name(&old),
                        )
                        .await;
                    new_adapter.shutdown().await;
                    let rollback = self
                        .setup_and_start_consumers(old_adapter, &old.queue_configs)
                        .await;
                    match rollback {
                        Ok(consumers) => {
                            self.replace_all_consumers(consumers).await?;
                            return Err(error);
                        }
                        Err(rollback_error) => {
                            anyhow::bail!(
                                "queue adapter replacement failed: {error}; previous consumers could not be restored: {rollback_error}"
                            );
                        }
                    }
                }
            }
        }

        let current_adapter = self.adapter.current().await;
        validate_queue_updates(
            &self.adapter.current_name().await,
            &old.queue_configs,
            &next.queue_configs,
        )
        .map_err(anyhow::Error::msg)?;
        if let Err(error) = self
            .setup_consumers(current_adapter.clone(), &next.queue_configs)
            .await
        {
            let rollback = self
                .restore_definitions(current_adapter, &old.queue_configs, &next.queue_configs)
                .await;
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(anyhow::anyhow!(
                    "queue configuration setup failed: {error}; adapter definition rollback failed: {rollback_error}"
                )),
            };
        }
        if let Err(error) = self.stop_all_consumers(current_adapter.clone()).await {
            let restore = self
                .restore_definitions(
                    current_adapter.clone(),
                    &old.queue_configs,
                    &next.queue_configs,
                )
                .await;
            let restart = self
                .start_consumers(current_adapter, &old.queue_configs)
                .await;
            let restore_summary = result_summary(&restore);
            let restart_summary = result_summary(&restart);
            if let Ok(consumers) = restart {
                self.replace_all_consumers(consumers).await?;
            }
            anyhow::bail!(
                "cannot stop previous queue consumers: {error}; definition rollback: {}; consumer restart: {}",
                restore_summary,
                restart_summary
            );
        }
        if let Err(error) = self
            .forget_removed_definitions(
                current_adapter.clone(),
                &old.queue_configs,
                &next.queue_configs,
            )
            .await
        {
            let restore = self
                .restore_definitions(
                    current_adapter.clone(),
                    &old.queue_configs,
                    &next.queue_configs,
                )
                .await;
            let restart = self
                .start_consumers(current_adapter, &old.queue_configs)
                .await;
            let restore_summary = result_summary(&restore);
            let restart_summary = result_summary(&restart);
            if let Ok(consumers) = restart {
                self.replace_all_consumers(consumers).await?;
            }
            anyhow::bail!(
                "cannot remove stale queue definitions: {error}; definition rollback: {}; consumer restart: {}",
                restore_summary,
                restart_summary
            );
        }
        match self
            .start_consumers(current_adapter.clone(), &next.queue_configs)
            .await
        {
            Ok(pending) => {
                *self.config.write().await = Arc::new(next);
                self.replace_all_consumers(pending).await?;
                Ok(())
            }
            Err(error) => {
                let restore = self
                    .restore_definitions(
                        current_adapter.clone(),
                        &old.queue_configs,
                        &next.queue_configs,
                    )
                    .await;
                let rollback = if restore.is_ok() {
                    self.start_consumers(current_adapter, &old.queue_configs)
                        .await
                } else {
                    Err(anyhow::anyhow!(result_summary(&restore)))
                };
                match rollback {
                    Ok(consumers) => {
                        self.replace_all_consumers(consumers).await?;
                        Err(error)
                    }
                    Err(rollback_error) => anyhow::bail!(
                        "queue configuration apply failed: {error}; previous consumers could not be restored: {rollback_error}"
                    ),
                }
            }
        }
    }

    async fn setup_consumers(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        definitions: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<()> {
        for (name, config) in definitions {
            self.setup_consumer(adapter.clone(), name, config).await?;
        }
        Ok(())
    }

    async fn start_consumers(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        definitions: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<PendingConsumers> {
        let mut pending = BTreeMap::new();
        for (name, config) in definitions {
            match self.start_consumer(adapter.clone(), name, config).await {
                Ok(handle) => {
                    pending.insert(name.clone(), handle);
                }
                Err(error) => {
                    let mut cleanup_errors = Vec::new();
                    for (name, handle) in pending {
                        if let Err(cleanup_error) =
                            self.stop_consumer(adapter.clone(), &name, handle).await
                        {
                            cleanup_errors.push(format!("{name}: {cleanup_error}"));
                        }
                    }
                    if !cleanup_errors.is_empty() {
                        anyhow::bail!(
                            "{error}; partial consumer cleanup failed: {}",
                            cleanup_errors.join("; ")
                        );
                    }
                    return Err(error);
                }
            }
        }
        Ok(pending)
    }

    async fn setup_and_start_consumers(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        definitions: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<PendingConsumers> {
        self.setup_consumers(adapter.clone(), definitions).await?;
        self.start_consumers(adapter, definitions).await
    }

    async fn setup_consumer(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        queue: &str,
        config: &FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        config.validate(queue).map_err(anyhow::Error::msg)?;
        adapter.setup_function_queue(queue, config).await?;
        Ok(())
    }

    async fn start_consumer(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        queue: &str,
        config: &FunctionQueueConfig,
    ) -> anyhow::Result<ConsumerHandle> {
        let receiver = adapter
            .consume_function_queue(queue, config.concurrency)
            .await?;
        let task = tokio::spawn(run_supervised_consumer(
            queue.to_string(),
            config.clone(),
            adapter,
            self.invoker.clone(),
            receiver,
        ));
        Ok(ConsumerHandle { task })
    }

    async fn restore_definition(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        queue: &str,
        previous: Option<&FunctionQueueConfig>,
    ) -> anyhow::Result<()> {
        match previous {
            Some(config) => self.setup_consumer(adapter, queue, config).await,
            None => adapter.forget_function_queue(queue).await,
        }
    }

    async fn restore_definitions(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        previous: &BTreeMap<String, FunctionQueueConfig>,
        attempted: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<()> {
        for name in attempted.keys() {
            if !previous.contains_key(name) {
                adapter.forget_function_queue(name).await?;
            }
        }
        self.setup_consumers(adapter, previous).await
    }

    async fn forget_removed_definitions(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        previous: &BTreeMap<String, FunctionQueueConfig>,
        next: &BTreeMap<String, FunctionQueueConfig>,
    ) -> anyhow::Result<()> {
        for name in previous.keys() {
            if !next.contains_key(name) {
                adapter.forget_function_queue(name).await?;
            }
        }
        Ok(())
    }

    async fn stop_consumer(
        &self,
        adapter: Arc<dyn QueueAdapter>,
        name: &str,
        handle: ConsumerHandle,
    ) -> anyhow::Result<()> {
        handle.abort().await;
        adapter.stop_function_queue_consumer(name).await
    }

    async fn replace_consumer(&self, name: String, handle: ConsumerHandle) -> anyhow::Result<()> {
        let previous = self.consumers.lock().await.remove(&name);
        if let Some(previous) = previous {
            let adapter = self.adapter.current().await;
            self.stop_consumer(adapter, &name, previous).await?;
        }
        self.consumers.lock().await.insert(name, handle);
        Ok(())
    }

    async fn replace_all_consumers(&self, pending: PendingConsumers) -> anyhow::Result<()> {
        let previous = std::mem::replace(&mut *self.consumers.lock().await, pending);
        if previous.is_empty() {
            return Ok(());
        }
        let adapter = self.adapter.current().await;
        for (name, handle) in previous {
            self.stop_consumer(adapter.clone(), &name, handle).await?;
        }
        Ok(())
    }

    async fn stop_all_consumers(&self, adapter: Arc<dyn QueueAdapter>) -> anyhow::Result<()> {
        let consumers = std::mem::take(&mut *self.consumers.lock().await);
        let mut errors = Vec::new();
        for (name, handle) in consumers {
            if let Err(error) = self.stop_consumer(adapter.clone(), &name, handle).await {
                errors.push(format!("{name}: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("; "))
        }
    }
}

async fn run_supervised_consumer(
    queue: String,
    config: FunctionQueueConfig,
    adapter: Arc<dyn QueueAdapter>,
    invoker: Arc<dyn Invoker>,
    mut receiver: mpsc::Receiver<QueueMessage>,
) {
    loop {
        run_consumer(
            queue.clone(),
            config.clone(),
            adapter.clone(),
            invoker.clone(),
            receiver,
        )
        .await;

        tracing::warn!(queue = %queue, "function queue consumer stream closed; reconnecting");
        receiver = loop {
            tokio::time::sleep(Duration::from_millis(config.poll_interval_ms)).await;
            match adapter
                .consume_function_queue(&queue, config.concurrency)
                .await
            {
                Ok(next) => break next,
                Err(error) => {
                    tracing::error!(queue = %queue, error = %error, "function queue consumer reconnect failed");
                }
            }
        };
    }
}

async fn run_consumer(
    queue: String,
    config: FunctionQueueConfig,
    adapter: Arc<dyn QueueAdapter>,
    invoker: Arc<dyn Invoker>,
    receiver: tokio::sync::mpsc::Receiver<QueueMessage>,
) {
    if config.r#type == "fifo" {
        run_grouped_fifo(queue, config, adapter, invoker, receiver).await;
    } else {
        run_concurrent(queue, config, adapter, invoker, receiver).await;
    }
}

async fn run_concurrent(
    queue: String,
    config: FunctionQueueConfig,
    adapter: Arc<dyn QueueAdapter>,
    invoker: Arc<dyn Invoker>,
    mut receiver: mpsc::Receiver<QueueMessage>,
) {
    let active = Arc::new(Semaphore::new(config.concurrency as usize));
    let mut tasks = tokio::task::JoinSet::new();

    while let Some(message) = receiver.recv().await {
        let queue_name = queue.clone();
        let adapter = adapter.clone();
        let invoker = invoker.clone();
        let active = active.clone();
        let max_retries = config.max_retries;
        let poll_interval_ms = config.poll_interval_ms;
        let timeout_ms = config.timeout_ms;
        tasks.spawn(async move {
            process_standard_message(
                &queue_name,
                max_retries,
                poll_interval_ms,
                timeout_ms,
                active,
                adapter,
                invoker,
                message,
            )
            .await;
        });

        reap_finished(&queue, &mut tasks);
    }

    // A normal receiver close drains work already accepted by this runtime.
    while tasks.join_next().await.is_some() {}
}

async fn run_grouped_fifo(
    queue: String,
    config: FunctionQueueConfig,
    adapter: Arc<dyn QueueAdapter>,
    invoker: Arc<dyn Invoker>,
    mut receiver: mpsc::Receiver<QueueMessage>,
) {
    let active = Arc::new(Semaphore::new(config.concurrency as usize));
    let mut groups: HashMap<String, mpsc::UnboundedSender<QueueMessage>> = HashMap::new();
    let (idle_tx, mut idle_rx) = mpsc::unbounded_channel::<String>();
    let mut tasks = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            message = receiver.recv() => {
                let Some(message) = message else { break };
                let group = match group_key(&message.data, config.message_group_field.as_deref()) {
                    Ok(group) => group,
                    Err(err) => {
                        tracing::warn!(queue = %queue, error = %err, "function queue message has no FIFO group");
                        if let Err(nack_err) = adapter
                            .nack_function_queue(
                                &queue,
                                message.delivery_id,
                                message.attempt,
                                config.max_retries,
                            )
                            .await
                        {
                            tracing::error!(queue = %queue, error = %nack_err, "failed to nack invalid FIFO message");
                        }
                        continue;
                    }
                };

                groups.retain(|_, sender| !sender.is_closed());
                if let Some(sender) = groups.get(&group) {
                    if sender.send(message.clone()).is_ok() {
                        continue;
                    }
                }

                let (sender, mut group_rx) = mpsc::unbounded_channel();
                // A fresh channel cannot be closed before its receiver is moved
                // into the task.
                let _ = sender.send(message);
                groups.insert(group.clone(), sender);

                let queue_name = queue.clone();
                let adapter = adapter.clone();
                let invoker = invoker.clone();
                let active = active.clone();
                let idle_tx = idle_tx.clone();
                let max_retries = config.max_retries;
                let backoff_ms = config.backoff_ms;
                let poll_interval_ms = config.poll_interval_ms;
                let timeout_ms = config.timeout_ms;
                tasks.spawn(async move {
                    while let Ok(Some(message)) =
                        tokio::time::timeout(Duration::from_secs(60), group_rx.recv()).await
                    {
                        process_fifo_message(
                            &queue_name,
                            max_retries,
                            backoff_ms,
                            poll_interval_ms,
                            timeout_ms,
                            active.clone(),
                            adapter.clone(),
                            invoker.clone(),
                            message,
                        )
                        .await;
                    }
                    let _ = idle_tx.send(group);
                });
            }
            Some(group) = idle_rx.recv() => {
                if groups.get(&group).is_some_and(mpsc::UnboundedSender::is_closed) {
                    groups.remove(&group);
                }
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(err) = result {
                    tracing::error!(queue = %queue, error = %err, "FIFO group task failed");
                }
            }
        }
    }

    // Closing every sender lets group actors drain their ordered backlog.
    groups.clear();
    while tasks.join_next().await.is_some() {}
}

fn reap_finished(queue: &str, tasks: &mut tokio::task::JoinSet<()>) {
    while let Some(result) = tasks.try_join_next() {
        if let Err(err) = result {
            tracing::error!(queue = %queue, error = %err, "function queue task failed");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_standard_message(
    queue: &str,
    max_retries: u32,
    poll_interval_ms: u64,
    timeout_ms: u64,
    active: Arc<Semaphore>,
    adapter: Arc<dyn QueueAdapter>,
    invoker: Arc<dyn Invoker>,
    message: QueueMessage,
) {
    wait_for_function(&invoker, queue, &message.function_id, poll_interval_ms).await;
    let Ok(_permit) = active.acquire_owned().await else {
        return;
    };
    let result = invoke_message(queue, &invoker, &message, message.attempt, timeout_ms).await;
    let operation = if result.is_ok() {
        adapter.ack_function_queue(queue, message.delivery_id).await
    } else {
        adapter
            .nack_function_queue(queue, message.delivery_id, message.attempt, max_retries)
            .await
    };
    if let Err(err) = operation {
        tracing::error!(queue = %queue, function_id = %message.function_id, error = %err, "function queue acknowledgement failed");
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_fifo_message(
    queue: &str,
    max_retries: u32,
    backoff_ms: u64,
    poll_interval_ms: u64,
    timeout_ms: u64,
    active: Arc<Semaphore>,
    adapter: Arc<dyn QueueAdapter>,
    invoker: Arc<dyn Invoker>,
    message: QueueMessage,
) {
    let mut attempt = message.attempt;
    loop {
        wait_for_function(&invoker, queue, &message.function_id, poll_interval_ms).await;
        let Ok(permit) = active.clone().acquire_owned().await else {
            return;
        };
        let succeeded = invoke_message(queue, &invoker, &message, attempt, timeout_ms)
            .await
            .is_ok();
        drop(permit);
        if succeeded {
            if let Err(err) = adapter.ack_function_queue(queue, message.delivery_id).await {
                tracing::error!(queue = %queue, function_id = %message.function_id, error = %err, "function queue acknowledgement failed");
            }
            return;
        }

        if attempt >= max_retries {
            if let Err(err) = adapter
                .nack_function_queue(queue, message.delivery_id, max_retries, max_retries)
                .await
            {
                tracing::error!(queue = %queue, function_id = %message.function_id, error = %err, "function queue acknowledgement failed");
            }
            return;
        }

        attempt = attempt.saturating_add(1);
        let delay_ms = retry_backoff_ms(backoff_ms, attempt);
        tracing::warn!(
            queue = %queue,
            function_id = %message.function_id,
            attempt,
            delay_ms,
            "FIFO function queue invocation failed; retrying in place"
        );
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

async fn invoke_message(
    queue: &str,
    invoker: &Arc<dyn Invoker>,
    message: &QueueMessage,
    attempt: u32,
    timeout_ms: u64,
) -> Result<Option<Value>, String> {
    let baggage = message.baggage.as_deref().and_then(scrub_relevance_tags);
    let span = tracing::info_span!(
        "function_queue_job",
        queue = %queue,
        function_id = %message.function_id,
        attempt,
        "messaging.system" = "queue",
        "messaging.destination.name" = %queue,
        "messaging.operation.type" = "process",
        "iii.tag.kind" = "queue.process",
    );
    if message.traceparent.is_some() || baggage.is_some() {
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let parent = iii_helpers::observability::extract_context(
            message.traceparent.as_deref(),
            baggage.as_deref(),
        );
        if let Err(error) = span.set_parent(parent) {
            tracing::warn!(queue = %queue, error = %error, "failed to attach queue trace context");
        }
    }
    invoker
        .call_with_timeout(&message.function_id, message.data.clone(), timeout_ms)
        .instrument(span)
        .await
}

/// Remove the publisher scope's display identity before starting the queue
/// delivery scope. Lineage fields such as session, message, and target
/// function remain available to the consumer.
fn scrub_relevance_tags(header: &str) -> Option<String> {
    const SCRUBBED: [&str; 2] = ["iii.tag.kind", "iii.tag.display_name"];
    let kept: Vec<&str> = header
        .split(',')
        .filter(|entry| {
            let key = entry.split(['=', ';']).next().unwrap_or("").trim();
            !SCRUBBED.contains(&key)
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join(","))
    }
}

async fn wait_for_function(
    invoker: &Arc<dyn Invoker>,
    queue: &str,
    function_id: &str,
    poll_interval_ms: u64,
) {
    let mut waiting = false;
    loop {
        match invoker.function_available(function_id).await {
            Ok(true) => {
                if waiting {
                    tracing::info!(queue = %queue, function_id = %function_id, "function queue target became available");
                }
                return;
            }
            Ok(false) => {
                if !waiting {
                    tracing::info!(queue = %queue, function_id = %function_id, "function queue target is not registered yet; holding delivery");
                    waiting = true;
                }
            }
            Err(error) => {
                if !waiting {
                    tracing::warn!(queue = %queue, function_id = %function_id, error = %error, "cannot check function queue target availability; holding delivery");
                    waiting = true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(poll_interval_ms.max(25))).await;
    }
}

fn retry_backoff_ms(base_ms: u64, attempt: u32) -> u64 {
    let factor = 1u64
        .checked_shl(attempt.saturating_sub(1).min(63))
        .unwrap_or(u64::MAX);
    base_ms.saturating_mul(factor)
}

fn result_summary<T, E: std::fmt::Display>(result: &Result<T, E>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(error) => error.to_string(),
    }
}

fn validate_queue_updates(
    adapter_name: &str,
    previous: &BTreeMap<String, FunctionQueueConfig>,
    next: &BTreeMap<String, FunctionQueueConfig>,
) -> Result<(), String> {
    for (name, config) in next {
        validate_queue_update(adapter_name, name, previous.get(name), config)?;
    }
    Ok(())
}

fn validate_queue_update(
    adapter_name: &str,
    queue: &str,
    previous: Option<&FunctionQueueConfig>,
    next: &FunctionQueueConfig,
) -> Result<(), String> {
    if adapter_name == "rabbitmq"
        && previous.is_some_and(|previous| previous.max_priority != next.max_priority)
    {
        return Err(format!(
            "RabbitMQ queue '{queue}' max_priority cannot be changed in place; delete and recreate its broker topology first"
        ));
    }
    Ok(())
}

fn group_key(data: &Value, field: Option<&str>) -> Result<String, String> {
    let field = field.ok_or_else(|| "fifo queue requires message_group_field".to_string())?;
    let value = data
        .get(field)
        .ok_or_else(|| format!("payload is missing message group field '{field}'"))?;
    match value {
        Value::String(value) if !value.is_empty() => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(format!(
            "message group field '{field}' must be a non-empty string or number"
        )),
    }
}

fn priority_from_data(data: &Value, field: Option<&str>) -> Option<u8> {
    field
        .and_then(|field| data.get(field))
        .and_then(Value::as_u64)
        .and_then(|priority| u8::try_from(priority).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    use crate::store::TopicStats;

    type PublishedMessage = (
        String,
        String,
        Value,
        String,
        Option<String>,
        Option<String>,
    );

    #[derive(Default)]
    struct RecordingAdapter {
        acked: StdMutex<Vec<u64>>,
        nacked: StdMutex<Vec<(u64, u32, u32)>>,
        published: StdMutex<Vec<PublishedMessage>>,
    }

    #[async_trait]
    impl QueueAdapter for RecordingAdapter {
        async fn enqueue(
            &self,
            _topic: &str,
            _data: Value,
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) {
        }

        async fn subscribe(
            &self,
            _topic: &str,
            _id: &str,
            _function_id: &str,
            _condition_function_id: Option<String>,
            _queue_config: Option<crate::subscriber_config::SubscriberQueueConfig>,
        ) {
        }

        async fn unsubscribe(&self, _topic: &str, _id: &str) {}

        async fn redrive_dlq(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn redrive_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn discard_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn dlq_count(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn topic_stats(&self, _topic: &str) -> anyhow::Result<TopicStats> {
            Ok(TopicStats::default())
        }

        async fn shutdown(&self) {}

        async fn publish_to_function_queue(
            &self,
            queue_name: &str,
            function_id: &str,
            data: Value,
            message_id: &str,
            _max_retries: u32,
            _backoff_ms: u64,
            traceparent: Option<String>,
            baggage: Option<String>,
            _priority: Option<u8>,
        ) -> anyhow::Result<()> {
            self.published.lock().unwrap().push((
                queue_name.to_string(),
                function_id.to_string(),
                data,
                message_id.to_string(),
                traceparent,
                baggage,
            ));
            Ok(())
        }

        async fn ack_function_queue(
            &self,
            _queue_name: &str,
            delivery_id: u64,
        ) -> anyhow::Result<()> {
            self.acked.lock().unwrap().push(delivery_id);
            Ok(())
        }

        async fn nack_function_queue(
            &self,
            _queue_name: &str,
            delivery_id: u64,
            attempt: u32,
            max_retries: u32,
        ) -> anyhow::Result<()> {
            self.nacked
                .lock()
                .unwrap()
                .push((delivery_id, attempt, max_retries));
            Ok(())
        }
    }

    #[derive(Default)]
    struct TimingInvoker {
        events: StdMutex<Vec<String>>,
    }

    #[async_trait]
    impl Invoker for TimingInvoker {
        async fn call(&self, _function_id: &str, payload: Value) -> Result<Option<Value>, String> {
            let session = payload["session_id"].as_str().unwrap();
            let sequence = payload["sequence"].as_u64().unwrap();
            let label = format!("{session}:{sequence}");
            self.events.lock().unwrap().push(format!("start:{label}"));
            tokio::time::sleep(Duration::from_millis(
                payload["delay_ms"].as_u64().unwrap_or_default(),
            ))
            .await;
            self.events.lock().unwrap().push(format!("end:{label}"));
            Ok(None)
        }
    }

    struct FailingInvoker {
        calls: AtomicUsize,
        failures: usize,
    }

    #[async_trait]
    impl Invoker for FailingInvoker {
        async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < self.failures {
                Err("expected failure".to_string())
            } else {
                Ok(None)
            }
        }
    }

    #[derive(Default)]
    struct TimeoutRecordingInvoker {
        timeout_ms: AtomicU64,
    }

    #[async_trait]
    impl Invoker for TimeoutRecordingInvoker {
        async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
            Ok(None)
        }

        async fn call_with_timeout(
            &self,
            _function_id: &str,
            _payload: Value,
            timeout_ms: u64,
        ) -> Result<Option<Value>, String> {
            self.timeout_ms.store(timeout_ms, Ordering::SeqCst);
            Ok(None)
        }
    }

    fn message(delivery_id: u64, session: &str, sequence: u64, delay_ms: u64) -> QueueMessage {
        QueueMessage {
            delivery_id,
            function_id: "harness::turn".to_string(),
            data: json!({
                "session_id": session,
                "sequence": sequence,
                "delay_ms": delay_ms,
            }),
            attempt: 0,
            message_id: Some(format!("receipt-{delivery_id}")),
            traceparent: None,
            baggage: None,
        }
    }

    #[test]
    fn fifo_group_key_accepts_string_and_number() {
        assert_eq!(
            group_key(&json!({"session_id": "s1"}), Some("session_id")).unwrap(),
            "s1"
        );
        assert_eq!(
            group_key(&json!({"session_id": 42}), Some("session_id")).unwrap(),
            "42"
        );
    }

    #[test]
    fn fifo_group_key_rejects_missing_or_invalid_value() {
        assert!(group_key(&json!({}), Some("session_id")).is_err());
        assert!(group_key(&json!({"session_id": null}), Some("session_id")).is_err());
    }

    #[test]
    fn priority_is_read_from_configured_top_level_field() {
        assert_eq!(
            priority_from_data(&json!({"priority": 7}), Some("priority")),
            Some(7)
        );
        assert_eq!(
            priority_from_data(&json!({"priority": 999}), Some("priority")),
            None
        );
    }

    #[test]
    fn rabbitmq_rejects_in_place_priority_topology_changes() {
        let previous = FunctionQueueConfig {
            max_priority: Some(5),
            ..FunctionQueueConfig::default()
        };
        let next = FunctionQueueConfig {
            max_priority: Some(10),
            ..FunctionQueueConfig::default()
        };

        assert!(validate_queue_update("rabbitmq", "jobs", Some(&previous), &next).is_err());
        assert!(validate_queue_update("builtin", "jobs", Some(&previous), &next).is_ok());
    }

    #[test]
    fn provider_inputs_accept_engine_caller_metadata_but_reject_other_fields() {
        let define: DefineQueueInput = serde_json::from_value(json!({
            "queue": "harness-turn",
            "_caller_worker_id": "engine-worker"
        }))
        .unwrap();
        assert_eq!(define._caller_worker_id.as_deref(), Some("engine-worker"));

        let enqueue: EnqueueInput = serde_json::from_value(json!({
            "queue": "harness-turn",
            "function_id": "harness::turn",
            "data": {"session_id": "s1"},
            "messageReceiptId": "receipt-1",
            "traceparent": "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01",
            "baggage": "iii.session.id=s1",
            "_caller_worker_id": "engine-worker"
        }))
        .unwrap();
        assert_eq!(enqueue._caller_worker_id.as_deref(), Some("engine-worker"));
        assert!(enqueue.traceparent.is_some());
        assert_eq!(enqueue.baggage.as_deref(), Some("iii.session.id=s1"));

        assert!(serde_json::from_value::<DefineQueueInput>(json!({
            "queue": "harness-turn",
            "unexpected": true
        }))
        .is_err());
    }

    #[tokio::test]
    async fn enqueue_forwards_trace_context_to_the_adapter() {
        let adapter = Arc::new(RecordingAdapter::default());
        let swappable = Arc::new(SwappableAdapter::new(adapter.clone(), "recording"));
        let mut config = QueueConfig::default();
        config
            .queue_configs
            .insert("harness-turn".to_string(), FunctionQueueConfig::default());
        let runtime = FunctionQueueRuntime::new(
            Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")),
            swappable,
            Arc::new(TimingInvoker::default()),
            Arc::new(tokio::sync::RwLock::new(Arc::new(config))),
            Arc::new(tokio::sync::RwLock::new(())),
        );

        let output = runtime
            .enqueue(EnqueueInput {
                queue: "harness-turn".to_string(),
                function_id: "harness::turn".to_string(),
                data: json!({"session_id": "s1"}),
                message_receipt_id: "receipt-1".to_string(),
                traceparent: Some(
                    "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01".to_string(),
                ),
                baggage: Some("iii.session.id=s1,iii.function.id=harness%3A%3Aturn".to_string()),
                _caller_worker_id: Some("engine-worker".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(output.message_receipt_id, "receipt-1");
        let published = adapter.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].0, "harness-turn");
        assert_eq!(published[0].1, "harness::turn");
        assert_eq!(published[0].3, "receipt-1");
        assert!(published[0].4.is_some());
        assert_eq!(
            published[0].5.as_deref(),
            Some("iii.session.id=s1,iii.function.id=harness%3A%3Aturn")
        );
    }

    #[test]
    fn queue_delivery_scrubs_only_publisher_display_identity() {
        assert_eq!(
            scrub_relevance_tags(
                "iii.tag.kind=harness.turn,iii.tag.display_name=Turn,iii.tag.message=hello,iii.session.id=s1"
            )
            .as_deref(),
            Some("iii.tag.message=hello,iii.session.id=s1")
        );
        assert_eq!(scrub_relevance_tags("iii.tag.kind=harness.turn"), None);
    }

    #[tokio::test]
    async fn grouped_fifo_orders_each_session_and_runs_sessions_concurrently() {
        let adapter = Arc::new(RecordingAdapter::default());
        let invoker = Arc::new(TimingInvoker::default());
        let config = FunctionQueueConfig {
            r#type: "fifo".to_string(),
            message_group_field: Some("session_id".to_string()),
            concurrency: 2,
            poll_interval_ms: 1,
            ..FunctionQueueConfig::default()
        };
        let (sender, receiver) = mpsc::channel(8);
        let consumer = tokio::spawn(run_grouped_fifo(
            "harness-turn".to_string(),
            config,
            adapter.clone(),
            invoker.clone(),
            receiver,
        ));

        sender.send(message(1, "s1", 1, 80)).await.unwrap();
        sender.send(message(2, "s1", 2, 0)).await.unwrap();
        sender.send(message(3, "s2", 1, 0)).await.unwrap();
        drop(sender);
        tokio::time::timeout(Duration::from_secs(2), consumer)
            .await
            .expect("consumer timed out")
            .expect("consumer task failed");

        let events = invoker.events.lock().unwrap().clone();
        let position = |event: &str| events.iter().position(|item| item == event).unwrap();
        assert!(position("end:s1:1") < position("start:s1:2"));
        assert!(position("end:s2:1") < position("end:s1:1"));
        assert_eq!(adapter.acked.lock().unwrap().len(), 3);
        assert!(adapter.nacked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn function_queue_invocation_uses_configured_timeout() {
        let adapter = Arc::new(RecordingAdapter::default());
        let invoker = Arc::new(TimeoutRecordingInvoker::default());
        let config = FunctionQueueConfig::default();
        let expected_timeout_ms = config.timeout_ms;
        let (sender, receiver) = mpsc::channel(1);
        let consumer = tokio::spawn(run_concurrent(
            "turns".to_string(),
            config,
            adapter.clone(),
            invoker.clone(),
            receiver,
        ));

        sender.send(message(7, "s1", 1, 0)).await.unwrap();
        drop(sender);
        consumer.await.unwrap();

        assert_eq!(
            invoker.timeout_ms.load(Ordering::SeqCst),
            expected_timeout_ms
        );
        assert_eq!(*adapter.acked.lock().unwrap(), vec![7]);
    }

    #[tokio::test]
    async fn fifo_retries_in_place_then_acks_without_requeueing() {
        let adapter = Arc::new(RecordingAdapter::default());
        let invoker = Arc::new(FailingInvoker {
            calls: AtomicUsize::new(0),
            failures: 2,
        });

        process_fifo_message(
            "harness-turn",
            3,
            1,
            1,
            1_800_000,
            Arc::new(Semaphore::new(1)),
            adapter.clone(),
            invoker.clone(),
            message(9, "s1", 1, 0),
        )
        .await;

        assert_eq!(invoker.calls.load(Ordering::SeqCst), 3);
        assert_eq!(*adapter.acked.lock().unwrap(), vec![9]);
        assert!(adapter.nacked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fifo_exhaustion_nacks_directly_to_dlq_threshold() {
        let adapter = Arc::new(RecordingAdapter::default());
        let invoker = Arc::new(FailingInvoker {
            calls: AtomicUsize::new(0),
            failures: usize::MAX,
        });

        process_fifo_message(
            "harness-turn",
            2,
            1,
            1,
            1_800_000,
            Arc::new(Semaphore::new(1)),
            adapter.clone(),
            invoker.clone(),
            message(11, "s1", 1, 0),
        )
        .await;

        assert_eq!(invoker.calls.load(Ordering::SeqCst), 3);
        assert!(adapter.acked.lock().unwrap().is_empty());
        assert_eq!(*adapter.nacked.lock().unwrap(), vec![(11, 2, 2)]);
    }
}
