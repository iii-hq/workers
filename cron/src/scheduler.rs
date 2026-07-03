//! Per-job scheduling loop. Port of the builtin's structs.rs:
//! one tokio task per job; UTC only; no catch-up (always the NEXT upcoming
//! fire from now -- missed runs while down are skipped, never replayed).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use cron::Schedule;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::locks::CronLock;

#[async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(
        &self,
        function_id: &str,
        payload: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String>;
}

#[derive(Debug, Clone)]
pub struct JobSpec {
    pub trigger_id: String,
    pub expression: String,
    pub function_id: String,
    pub condition_function_id: Option<String>,
}

struct JobEntry {
    spec: JobSpec,
    handle: JoinHandle<()>,
}

pub struct Scheduler {
    lock: Arc<dyn CronLock>,
    invoker: Arc<dyn Invoker>,
    jobs: Mutex<HashMap<String, JobEntry>>,
}

impl Scheduler {
    pub fn new(lock: Arc<dyn CronLock>, invoker: Arc<dyn Invoker>) -> Self {
        Self {
            lock,
            invoker,
            jobs: Mutex::new(HashMap::new()),
        }
    }

    pub async fn register(&self, spec: JobSpec) -> anyhow::Result<()> {
        if spec.expression.trim().is_empty() {
            anyhow::bail!("Cron expression is required");
        }
        let schedule = Schedule::from_str(&spec.expression)
            .map_err(|e| anyhow::anyhow!("invalid cron expression '{}': {e}", spec.expression))?;

        let mut jobs = self.jobs.lock().await;
        if jobs.contains_key(&spec.trigger_id) {
            anyhow::bail!("cron trigger '{}' is already registered", spec.trigger_id);
        }
        let handle = spawn_job(
            schedule,
            spec.clone(),
            self.lock.clone(),
            self.invoker.clone(),
        );
        jobs.insert(spec.trigger_id.clone(), JobEntry { spec, handle });
        Ok(())
    }

    pub async fn unregister(&self, trigger_id: &str) -> anyhow::Result<()> {
        let mut jobs = self.jobs.lock().await;
        match jobs.remove(trigger_id) {
            Some(entry) => {
                entry.handle.abort();
                self.lock.release(trigger_id).await;
                Ok(())
            }
            None => anyhow::bail!("cron trigger '{trigger_id}' not found"),
        }
    }

    /// Current specs, used by config hot-swap to re-register on a new lock backend.
    pub async fn job_specs(&self) -> Vec<JobSpec> {
        self.jobs
            .lock()
            .await
            .values()
            .map(|e| e.spec.clone())
            .collect()
    }

    pub async fn shutdown(&self) {
        let mut jobs = self.jobs.lock().await;
        for (trigger_id, entry) in jobs.drain() {
            entry.handle.abort();
            self.lock.release(&trigger_id).await;
        }
    }
}

fn spawn_job(
    schedule: Schedule,
    spec: JobSpec,
    lock: Arc<dyn CronLock>,
    invoker: Arc<dyn Invoker>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            let Some(next) = schedule.upcoming(Utc).next() else {
                tracing::info!(trigger_id = %spec.trigger_id, "schedule exhausted; stopping job");
                break;
            };
            let wait = (next - now).to_std().unwrap_or(std::time::Duration::ZERO);
            tokio::time::sleep(wait).await;

            if !lock.try_acquire(&spec.trigger_id).await {
                continue;
            }

            let payload = serde_json::json!({
                "trigger": "cron",
                "job_id": spec.trigger_id,
                "scheduled_time": next.to_rfc3339(),
                "actual_time": Utc::now().to_rfc3339(),
            });

            if let Some(cond) = &spec.condition_function_id {
                match invoker.call(cond, payload.clone()).await {
                    Ok(Some(v)) if v.as_bool() == Some(false) => {
                        lock.release(&spec.trigger_id).await;
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(trigger_id = %spec.trigger_id, error = %e, "condition check failed; skipping fire");
                        lock.release(&spec.trigger_id).await;
                        continue;
                    }
                }
            }

            if let Err(e) = invoker.call(&spec.function_id, payload).await {
                tracing::error!(trigger_id = %spec.trigger_id, function_id = %spec.function_id, error = %e, "cron fire failed");
            }
            lock.release(&spec.trigger_id).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingInvoker {
        calls: AtomicUsize,
        last_payload: Mutex<Option<serde_json::Value>>,
        condition_result: Option<serde_json::Value>,
    }

    #[async_trait]
    impl Invoker for CountingInvoker {
        async fn call(
            &self,
            function_id: &str,
            payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, String> {
            if function_id.starts_with("cond::") {
                return Ok(self.condition_result.clone());
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_payload.lock().await = Some(payload);
            Ok(None)
        }
    }

    fn every_second() -> &'static str {
        "*/1 * * * * *"
    }

    fn scheduler_with(invoker: Arc<CountingInvoker>) -> Scheduler {
        Scheduler::new(Arc::new(crate::locks::LocalLock::new()), invoker)
    }

    #[tokio::test]
    async fn register_rejects_empty_expression() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv);
        let err = s
            .register(JobSpec {
                trigger_id: "t1".into(),
                expression: "".into(),
                function_id: "f".into(),
                condition_function_id: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Cron expression is required"));
    }

    #[tokio::test]
    async fn register_rejects_duplicate_trigger_id() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv);
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "f".into(),
            condition_function_id: None,
        })
        .await
        .unwrap();
        let err = s
            .register(JobSpec {
                trigger_id: "t1".into(),
                expression: every_second().into(),
                function_id: "f".into(),
                condition_function_id: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already registered"));
        s.shutdown().await;
    }

    #[tokio::test]
    async fn fires_within_two_seconds_with_parity_payload() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: None,
        })
        .await
        .unwrap();
        for _ in 0..25 {
            if inv.calls.load(Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(inv.calls.load(Ordering::SeqCst) >= 1, "job never fired");
        let payload = inv.last_payload.lock().await.clone().unwrap();
        assert_eq!(payload["trigger"], "cron");
        assert_eq!(payload["job_id"], "t1");
        assert!(payload["scheduled_time"].as_str().unwrap().contains('T'));
        assert!(payload["actual_time"].as_str().unwrap().contains('T'));
        s.shutdown().await;
    }

    #[tokio::test]
    async fn condition_false_blocks_fire() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            condition_result: Some(serde_json::json!(false)),
        });
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: Some("cond::c".into()),
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        assert_eq!(
            inv.calls.load(Ordering::SeqCst),
            0,
            "condition=false must block"
        );
        s.shutdown().await;
    }

    #[tokio::test]
    async fn unregister_stops_firing() {
        let inv = Arc::new(CountingInvoker {
            calls: AtomicUsize::new(0),
            last_payload: Mutex::new(None),
            condition_result: None,
        });
        let s = scheduler_with(inv.clone());
        s.register(JobSpec {
            trigger_id: "t1".into(),
            expression: every_second().into(),
            function_id: "backend".into(),
            condition_function_id: None,
        })
        .await
        .unwrap();
        s.unregister("t1").await.unwrap();
        let before = inv.calls.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        assert_eq!(
            inv.calls.load(Ordering::SeqCst),
            before,
            "unregistered job must not fire"
        );
    }
}
