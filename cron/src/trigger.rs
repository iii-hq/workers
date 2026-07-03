//! Bridges engine trigger bindings into the scheduler. Mirrors
//! http/src/trigger.rs: table keyed by trigger id because the SDK only
//! populates `id` on unregister.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::scheduler::{JobSpec, Scheduler};

/// Trigger config schema, field-parity with the engine's CronTriggerConfig.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CronTriggerSpec {
    /// Cron expression (6-field format: sec min hour day month weekday; a 7th year field is accepted).
    pub expression: String,
    /// Optional function ID to evaluate before invoking the handler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_function_id: Option<String>,
}

#[derive(Clone)]
pub struct CronTriggerHandler {
    pub scheduler: Arc<Scheduler>,
}

impl CronTriggerHandler {
    pub fn new(scheduler: Arc<Scheduler>) -> Self {
        Self { scheduler }
    }
}

#[async_trait]
impl TriggerHandler for CronTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let spec: CronTriggerSpec = serde_json::from_value(config.config.clone()).map_err(|e| {
            Error::Handler(format!(
                "invalid cron trigger config (expression required): {e}"
            ))
        })?;
        self.scheduler
            .register(JobSpec {
                trigger_id: config.id,
                expression: spec.expression,
                function_id: config.function_id,
                condition_function_id: spec.condition_function_id,
            })
            .await
            .map_err(|e| Error::Handler(e.to_string()))
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let _ = self.scheduler.unregister(&config.id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use crate::scheduler::Invoker;

    #[derive(Default)]
    struct FakeInvoker {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Invoker for FakeInvoker {
        async fn call(
            &self,
            _function_id: &str,
            _payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(None)
        }
    }

    fn handler() -> CronTriggerHandler {
        let inv = Arc::new(FakeInvoker::default());
        CronTriggerHandler::new(Arc::new(Scheduler::new(
            Arc::new(crate::locks::LocalLock::new()),
            inv,
        )))
    }

    fn trigger_config(id: &str, cfg: serde_json::Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: "backend".to_string(),
            config: cfg,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn register_schedules_and_unregister_by_id_only() {
        let h = handler();
        h.register_trigger(trigger_config(
            "t1",
            serde_json::json!({"expression": "0 0 * * * *"}),
        ))
        .await
        .unwrap();
        assert_eq!(h.scheduler.job_specs().await.len(), 1);

        h.unregister_trigger(trigger_config("t1", serde_json::Value::Null))
            .await
            .unwrap();
        assert_eq!(h.scheduler.job_specs().await.len(), 0);
    }

    #[tokio::test]
    async fn register_rejects_missing_expression() {
        let h = handler();
        let err = h
            .register_trigger(trigger_config("t1", serde_json::json!({})))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("expression"));
    }

    #[tokio::test]
    async fn unregister_unknown_id_is_ok() {
        let h = handler();
        h.unregister_trigger(trigger_config("ghost", serde_json::Value::Null))
            .await
            .unwrap();
    }
}
