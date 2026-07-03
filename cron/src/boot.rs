//! Boot sequence: builtin guard, lock backend, scheduler, trigger type.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterTriggerType};

use crate::config::CronConfig;
use crate::locks;
use crate::scheduler::{Invoker, Scheduler};
use crate::trigger::{CronTriggerHandler, CronTriggerSpec};
use crate::TRIGGER_TYPE;

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
const BUILTIN_III_CRON_WORKER_ID: &str = "iii-cron";

pub type SchedulerCell = Arc<tokio::sync::RwLock<Arc<Scheduler>>>;
pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;
pub type ConfigCell = Arc<tokio::sync::RwLock<CronConfig>>;

#[derive(Clone)]
pub struct BootParts {
    pub scheduler: SchedulerCell,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
}

pub struct BootHandle {
    pub scheduler: SchedulerCell,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    pub fn parts(&self) -> BootParts {
        BootParts {
            scheduler: self.scheduler.clone(),
            config: self.config.clone(),
            apply_lock: self.apply_lock.clone(),
        }
    }

    pub async fn shutdown(&self) {
        self.scheduler.read().await.shutdown().await;
    }
}

struct SdkInvoker {
    iii: Arc<IIIClient>,
}

impl SdkInvoker {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait::async_trait]
impl Invoker for SdkInvoker {
    async fn call(
        &self,
        function_id: &str,
        payload: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

pub fn sdk_invoker(iii: Arc<IIIClient>) -> Arc<dyn Invoker> {
    Arc::new(SdkInvoker::new(iii))
}

pub async fn start(iii: Arc<IIIClient>, config: CronConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_cron(&iii).await?;

    let lock = locks::build_lock(&config).await?;
    let scheduler = Arc::new(Scheduler::new(lock, sdk_invoker(iii.clone())));
    let scheduler_cell: SchedulerCell = Arc::new(tokio::sync::RwLock::new(scheduler.clone()));
    let handler = CronTriggerHandler::new(scheduler_cell.clone());

    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(TRIGGER_TYPE, "Cron-based scheduled triggers", handler)
            .trigger_request_format::<CronTriggerSpec>(),
    );

    Ok(BootHandle {
        scheduler: scheduler_cell,
        config: Arc::new(tokio::sync::RwLock::new(config.normalized())),
        apply_lock: Arc::new(tokio::sync::Mutex::new(())),
    })
}

async fn guard_against_builtin_cron(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .trigger(TriggerRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_cron_active(&workers_list) {
        anyhow::bail!(
            "cannot start the cron worker: the built-in iii-cron worker is active and owns the \
             'cron' trigger type. Remove iii-cron from the engine config (a config.yaml that \
             doesn't list it won't run it), then start this worker."
        );
    }
    Ok(())
}

fn builtin_iii_cron_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|w| w.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_CRON_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_CRON_WORKER_ID)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_builtin_by_id() {
        let v = serde_json::json!({"workers": [{"id": "iii-cron"}]});
        assert!(builtin_iii_cron_active(&v));
    }

    #[test]
    fn detects_builtin_by_name() {
        let v = serde_json::json!({"workers": [{"id": "x", "name": "iii-cron"}]});
        assert!(builtin_iii_cron_active(&v));
    }

    #[test]
    fn absent_builtin_passes() {
        let v = serde_json::json!({"workers": [{"id": "iii-http"}]});
        assert!(!builtin_iii_cron_active(&v));
    }

    #[test]
    fn empty_list_passes() {
        assert!(!builtin_iii_cron_active(
            &serde_json::json!({"workers": []})
        ));
    }

    #[test]
    fn missing_key_passes() {
        assert!(!builtin_iii_cron_active(&serde_json::json!({})));
    }
}
