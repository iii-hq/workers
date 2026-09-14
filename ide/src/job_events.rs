//! `shell::job-finished` — fires once when a background job reaches a
//! terminal status, so a subscriber never has to poll `shell::status`.
//!
//! `shell::exec_bg` hands back a `job_id` and nothing else; without this
//! type the only way to learn a job ended is to ask again. For an agent
//! that pays a model round trip per call that is the most expensive shape
//! available, and agents route around it with worse ones — `shell::exec
//! sleep 60`, which the foreground timeout clamps to 30s anyway (MOT-4766).
//!
//! Unlike a fan-in over a state key, this wake cannot strand its
//! subscriber: the producer is the job, the job is already running, and
//! the runtime stamps the exact moment it ends.
//!
//! Delivery mirrors [`crate::events`]: `TriggerAction::Void`, each fire on
//! its own task, and the binding's `metadata` and `namespace` passed
//! through. That passthrough is load-bearing — a fire that drops the
//! `__binding` key is dropped by the harness with no error anywhere
//! (MOT-4719), which reads exactly like a trigger type that was never
//! registered.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jobs::{JobRecord, JobStatus};

/// The trigger type a subscriber binds to be woken when a job ends.
pub const JOB_FINISHED: &str = "shell::job-finished";

/// Select one background job, or observe future completions without a filter.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobFinishedConfig {
    /// Replay a terminal job while its record is retained in worker memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// What ended. Lean by design, like [`crate::events::ChangedEvent`]: a
/// subscriber that wants the job's output asks `shell::status`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct JobFinishedEvent {
    pub job_id: String,
    /// The command as spawned, so a subscriber bound without a `job_id`
    /// filter can tell jobs apart without a second call.
    pub argv: Vec<String>,
    /// `finished`, `killed`, or `failed` — never `running`.
    pub status: JobStatus,
    /// `None` when the job never produced one (killed before exit, or a
    /// sandbox response that carried no code).
    pub exit_code: Option<i32>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub duration_ms: u64,
}

impl JobFinishedEvent {
    fn from_record(record: &JobRecord) -> Option<Self> {
        if record.status == JobStatus::Running {
            return None;
        }
        let finished_at_ms = record.finished_at_ms?;
        Some(Self {
            job_id: record.id.clone(),
            argv: record.argv.clone(),
            status: record.status.clone(),
            exit_code: record.exit_code,
            started_at_ms: record.started_at_ms,
            finished_at_ms,
            duration_ms: finished_at_ms.saturating_sub(record.started_at_ms),
        })
    }
}

/// One binding: where to fire, what to pass through, and which job it
/// cares about.
#[derive(Debug, Clone)]
struct Subscriber {
    /// Distinguish replacements using the same engine trigger-instance id.
    registration_id: uuid::Uuid,
    function_id: String,
    metadata: Option<Value>,
    namespace: Option<String>,
    /// Bind with `config: { job_id }` to be woken by one job only. Absent
    /// means every job — useful for a dashboard, wasteful for a wake.
    job_id: Option<String>,
    /// A filtered binding can be claimed by either registration replay or
    /// live completion. Both paths update this under the subscriber lock.
    delivered: bool,
}

impl Subscriber {
    fn from_config(config: &TriggerConfig) -> Result<Self, Error> {
        let filter: JobFinishedConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| Error::Handler(format!("invalid shell::job-finished config: {e}")))?;
        Ok(Self {
            registration_id: uuid::Uuid::new_v4(),
            function_id: config.function_id.clone(),
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
            job_id: filter.job_id,
            delivered: false,
        })
    }

    fn wants(&self, job_id: &str) -> bool {
        self.job_id.as_deref().is_none_or(|id| id == job_id)
    }

    fn claim(&mut self, job_id: &str) -> bool {
        if !self.wants(job_id) || self.delivered {
            return false;
        }
        // Catch-all bindings stay active for other jobs and never replay.
        self.delivered = self.job_id.is_some();
        true
    }

    /// The fire for one event: `Void` routing (fire-and-forget) with the
    /// binding's metadata and namespace intact.
    fn request(&self, payload: Value) -> iii_sdk::protocol::TriggerRequestWithMetadata {
        let mut request: iii_sdk::protocol::TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id.clone(),
            payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(metadata) = &self.metadata {
            request = request.metadata(metadata.clone());
        }
        if let Some(namespace) = &self.namespace {
            request = request.namespace(namespace.clone());
        }
        request
    }
}

/// Live bindings, keyed by trigger-instance id.
///
/// A process global next to [`crate::jobs::JOBS`] and its kill-signal map,
/// for the same reason those are: the job registry is process-wide, and
/// both `exec_bg` finalize paths (host drain and sandbox response) end in
/// detached tasks that carry no worker state. Threading a handle into them
/// would mean changing both spawn signatures to deliver one notification.
static SUBSCRIBERS: OnceLock<Mutex<HashMap<String, Subscriber>>> = OnceLock::new();

/// The client the fan-out fires through, installed once at startup. `None`
/// in unit tests, which makes [`fire`] a no-op rather than a panic.
static CLIENT: OnceLock<IIIClient> = OnceLock::new();

fn subscribers() -> &'static Mutex<HashMap<String, Subscriber>> {
    SUBSCRIBERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn take_matching(job_id: &str) -> Vec<Subscriber> {
    subscribers()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values_mut()
        .filter_map(|s| s.claim(job_id).then(|| s.clone()))
        .collect()
}

/// Announce a terminal job record. Call it from a job's finalize path
/// AFTER the record is published, never while holding the job's lock: a
/// slow subscriber must not delay the job that woke it.
///
/// Silent no-op when the job is not terminal, when nothing is bound, or
/// before the trigger type is registered.
pub async fn fire(record: &JobRecord) {
    let Some(event) = JobFinishedEvent::from_record(record) else {
        return;
    };
    let targets = take_matching(&event.job_id);
    dispatch(&event, targets);
}

fn dispatch(event: &JobFinishedEvent, targets: Vec<Subscriber>) {
    #[cfg(test)]
    tests::observe_delivery(event, &targets);
    if targets.is_empty() {
        return;
    }
    let Some(iii) = CLIENT.get() else {
        return;
    };
    let Ok(payload) = serde_json::to_value(event) else {
        return;
    };
    for subscriber in targets {
        let iii = iii.clone();
        let request = subscriber.request(payload.clone());
        let function_id = subscriber.function_id.clone();
        let job_id = event.job_id.clone();
        // Own task per delivery, so one hung subscriber cannot hold up the
        // next one or the finalize task that called us.
        tokio::spawn(async move {
            if let Err(e) = iii.trigger(request).await {
                tracing::warn!(
                    function_id = %function_id,
                    error = %e,
                    "shell::job-finished fan-out failed"
                );
            } else {
                tracing::info!(
                    function_id = %function_id,
                    job_id = %job_id,
                    "shell::job-finished delivered"
                );
            }
        });
    }
}

struct JobFinishedTriggerHandler;

#[async_trait]
impl TriggerHandler for JobFinishedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let subscriber = Subscriber::from_config(&config)?;
        let registration_id = subscriber.registration_id;
        let job_id = subscriber.job_id.clone();
        tracing::info!(
            trigger_type = JOB_FINISHED,
            id = %config.id,
            job_id = ?subscriber.job_id,
            "job watch registered"
        );
        subscribers()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(config.id.clone(), subscriber);
        // Publish the binding BEFORE reading the retained job. If completion
        // races with this read, fire() claims it or replay does, never both.
        // Neither registry lock is held across a job lock or a delivery.
        if let Some(job_id) = job_id {
            if let Some(handle) = crate::jobs::get(&job_id).await {
                let event = {
                    let h = handle.lock().await;
                    // shell::kill stamps a terminal status before the drain or
                    // sandbox response has finished collecting the output.
                    if h.finalized {
                        JobFinishedEvent::from_record(&h.record)
                    } else {
                        None
                    }
                };
                if let Some(event) = event {
                    let target = subscribers()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get_mut(&config.id)
                        .filter(|s| s.registration_id == registration_id)
                        .and_then(|s| s.claim(&job_id).then(|| s.clone()));
                    if let Some(target) = target {
                        dispatch(&event, vec![target]);
                    }
                }
            }
        }
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = JOB_FINISHED, id = %config.id, "job watch unregistered");
        subscribers()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&config.id);
        Ok(())
    }
}

/// Register the trigger type and install the client the fan-out fires
/// through. The SDK queues the registration and cannot report failure; a
/// dead type surfaces as bindings that never fire.
pub fn register_job_finished_trigger(iii: &IIIClient) {
    let _ = CLIENT.set(iii.clone());
    let _handle = iii.register_trigger_type(
        RegisterTriggerType::new(
            JOB_FINISHED,
            "Fires once when a shell::exec_bg background job reaches a terminal status \
         (finished, killed, or failed). Bind with config: { job_id } to wake on one \
         job (replaying its result if already terminal and still retained), or omit \
         it to receive future completions. The event carries job_id, argv, status, \
         exit_code and timings; call shell::status for the job's output.",
            JobFinishedTriggerHandler,
        )
        .trigger_request_format::<JobFinishedConfig>()
        .call_request_format::<JobFinishedEvent>(),
    );
    tracing::info!(
        trigger_type = JOB_FINISHED,
        "sent the trigger type registration; delivery is confirmed by the first subscription"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    static TEST_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static DELIVERIES: OnceLock<Mutex<HashMap<String, Vec<JobFinishedEvent>>>> = OnceLock::new();

    // Observe the SDK delivery boundary without requiring a live engine.
    // Each probe is isolated by target function and removes itself on drop.
    pub(super) fn observe_delivery(event: &JobFinishedEvent, targets: &[Subscriber]) {
        let mut deliveries = DELIVERIES.get_or_init(Mutex::default).lock().unwrap();
        for target in targets {
            if let Some(events) = deliveries.get_mut(&target.function_id) {
                events.push(event.clone());
            }
        }
    }

    struct DeliveryProbe(String);

    impl DeliveryProbe {
        fn new(config: &mut TriggerConfig) -> Self {
            config.function_id = format!("test::{}", uuid::Uuid::new_v4());
            DELIVERIES
                .get_or_init(Mutex::default)
                .lock()
                .unwrap()
                .insert(config.function_id.clone(), Vec::new());
            Self(config.function_id.clone())
        }

        fn take(&self) -> Vec<JobFinishedEvent> {
            std::mem::take(
                DELIVERIES
                    .get()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .get_mut(&self.0)
                    .unwrap(),
            )
        }
    }

    impl Drop for DeliveryProbe {
        fn drop(&mut self) {
            DELIVERIES.get().unwrap().lock().unwrap().remove(&self.0);
        }
    }

    fn record(status: JobStatus, finished_at_ms: Option<u64>) -> JobRecord {
        JobRecord {
            id: "job-1".into(),
            argv: vec!["pnpm".into(), "install".into()],
            started_at_ms: 1_000,
            finished_at_ms,
            status,
            exit_code: Some(0),
            stdout: "lots of output".into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn config(id: &str, job_id: Option<&str>) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: "agent::wake".into(),
            config: job_id.map_or_else(|| json!({}), |j| json!({ "job_id": j })),
            metadata: Some(json!({ "__binding": "b-1" })),
            namespace: Some("default".into()),
        }
    }

    #[tokio::test]
    async fn subscribing_after_completion_claims_the_retained_event() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut finished = record(JobStatus::Finished, Some(crate::jobs::now_ms()));
        finished.id = "job-late-registration".into();
        crate::jobs::JOBS.map.lock().await.insert(
            finished.id.clone(),
            std::sync::Arc::new(tokio::sync::Mutex::new(crate::jobs::JobHandle {
                record: finished.clone(),
                finalized: true,
                child: None,
                host_pid: None,
            })),
        );
        fire(&finished).await;
        let mut cfg = config("t-late-registration", Some(&finished.id));
        let probe = DeliveryProbe::new(&mut cfg);
        let handler = JobFinishedTriggerHandler;
        handler.register_trigger(cfg.clone()).await.unwrap();
        let remaining = take_matching(&finished.id);
        handler.unregister_trigger(cfg).await.unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&finished.id);
        let delivered = probe.take();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].job_id, finished.id);
        assert!(
            remaining.is_empty(),
            "registration must claim the completed event"
        );
    }

    #[tokio::test]
    async fn a_watch_registered_before_exec_bg_receives_the_named_jobs_completion() {
        let _test_guard = TEST_GUARD.lock().await;
        let _gauge_guard = crate::jobs::GAUGE_TEST_GUARD.lock().await;
        let _sweep_guard = crate::jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let job_id = format!("pre-registered-{}", uuid::Uuid::new_v4());
        let mut cfg = config("t-before-exec-bg", Some(&job_id));
        let probe = DeliveryProbe::new(&mut cfg);
        JobFinishedTriggerHandler
            .register_trigger(cfg.clone())
            .await
            .unwrap();
        assert!(probe.take().is_empty(), "the job has not started");
        let shell_cfg = crate::config::ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            ..Default::default()
        };
        let response = crate::functions::exec_bg::handle(
            std::sync::Arc::new(shell_cfg),
            IIIClient::new("ws://stub-not-connected:0"),
            serde_json::from_value(json!({"command": "echo", "args": ["ready"], "job_id": job_id}))
                .unwrap(),
        )
        .await
        .unwrap();
        let handle = crate::jobs::get(&response.job_id).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !handle.lock().await.finalized {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let events = probe.take();
        JobFinishedTriggerHandler
            .unregister_trigger(cfg)
            .await
            .unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&response.job_id);
        assert_eq!(response.job_id, job_id);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].job_id, job_id);
        assert_eq!(events[0].status, JobStatus::Finished);
        assert_eq!(events[0].exit_code, Some(0));
    }

    #[tokio::test]
    async fn a_filtered_binding_claims_completion_only_once() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut finished = record(JobStatus::Finished, Some(3_000));
        finished.id = "job-delivered-once".into();
        let mut cfg = config("t-delivered-once", Some(&finished.id));
        let probe = DeliveryProbe::new(&mut cfg);
        let handler = JobFinishedTriggerHandler;
        handler.register_trigger(cfg.clone()).await.unwrap();
        fire(&finished).await;
        fire(&finished).await;
        let remaining = take_matching(&finished.id);
        handler.unregister_trigger(cfg).await.unwrap();
        assert_eq!(probe.take().len(), 1);
        assert!(
            remaining.is_empty(),
            "a second completion must not deliver again"
        );
    }

    #[tokio::test]
    async fn a_pre_registered_sandbox_job_runs_once_and_notifies_its_subscriber() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let _test_guard = TEST_GUARD.lock().await;
        let _gauge_guard = crate::jobs::GAUGE_TEST_GUARD.lock().await;
        struct Engine(AtomicUsize);
        #[async_trait]
        impl crate::triggers::TriggerFwd for Engine {
            async fn trigger(&self, _: &str, _: Value) -> Result<Value, Error> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(
                    json!({"stdout": "sandbox output", "stderr": "", "exit_code": 0,
                    "duration_ms": 1, "timed_out": false}),
                )
            }
        }
        let job_id = format!("sandbox-pre-registered-{}", uuid::Uuid::new_v4());
        let mut cfg = config("t-sandbox-before-exec-bg", Some(&job_id));
        let probe = DeliveryProbe::new(&mut cfg);
        JobFinishedTriggerHandler
            .register_trigger(cfg.clone())
            .await
            .unwrap();
        let engine = Arc::new(Engine(AtomicUsize::new(0)));
        let shell_cfg = Arc::new(crate::config::ShellConfig::default());
        let sandbox_id = uuid::Uuid::new_v4();
        let response = crate::functions::exec_bg::spawn_sandbox_job(
            Some(job_id.clone()),
            shell_cfg.clone(),
            engine.clone(),
            sandbox_id,
            vec!["echo".into()],
            1_000,
        )
        .await
        .unwrap();
        let duplicate = crate::functions::exec_bg::spawn_sandbox_job(
            Some(job_id.clone()),
            shell_cfg,
            engine.clone(),
            sandbox_id,
            vec!["echo".into(), "duplicate".into()],
            1_000,
        )
        .await;
        let handle = crate::jobs::get(&response.job_id).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !handle.lock().await.finalized {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let events = probe.take();
        let record = handle.lock().await.record.clone();
        JobFinishedTriggerHandler
            .unregister_trigger(cfg)
            .await
            .unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&response.job_id);
        assert_eq!(response.job_id, job_id);
        assert!(duplicate.unwrap_err().contains("already exists"));
        assert_eq!(engine.0.load(Ordering::Relaxed), 1);
        assert_eq!(record.stdout, "sandbox output");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].job_id, job_id);
        assert_eq!(events[0].status, JobStatus::Finished);
    }

    #[tokio::test]
    async fn live_completion_claims_a_binding_while_registration_reads_the_job() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut finished = record(JobStatus::Finished, Some(crate::jobs::now_ms()));
        finished.id = "job-registration-race".into();
        let handle = std::sync::Arc::new(tokio::sync::Mutex::new(crate::jobs::JobHandle {
            record: finished.clone(),
            finalized: true,
            child: None,
            host_pid: None,
        }));
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(finished.id.clone(), handle.clone());
        let held = handle.lock().await;
        let mut cfg = config("t-registration-race", Some(&finished.id));
        let probe = DeliveryProbe::new(&mut cfg);
        let registering = tokio::spawn({
            let cfg = cfg.clone();
            async move { JobFinishedTriggerHandler.register_trigger(cfg).await }
        });
        // Hold the snapshot read at the job lock until the binding is live.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if subscribers().lock().unwrap().contains_key(&cfg.id) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        fire(&finished).await;
        drop(held);
        registering.await.unwrap().unwrap();
        let duplicate_targets = take_matching(&finished.id);
        JobFinishedTriggerHandler
            .unregister_trigger(cfg)
            .await
            .unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&finished.id);
        assert_eq!(
            probe.take().len(),
            1,
            "live completion and replay must deliver once"
        );
        assert!(
            duplicate_targets.is_empty(),
            "replay must share the live claim"
        );
    }

    #[test]
    fn catch_all_bindings_keep_receiving_different_jobs() {
        let mut subscriber = Subscriber::from_config(&config("t-all", None)).unwrap();
        assert!(subscriber.claim("job-1"));
        assert!(subscriber.claim("job-2"));
    }

    #[tokio::test]
    async fn an_old_registration_does_not_replay_to_its_replacement() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut finished = record(JobStatus::Finished, Some(crate::jobs::now_ms()));
        finished.id = "job-replaced-registration".into();
        let handle = std::sync::Arc::new(tokio::sync::Mutex::new(crate::jobs::JobHandle {
            record: finished.clone(),
            finalized: true,
            child: None,
            host_pid: None,
        }));
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(finished.id.clone(), handle.clone());
        let held = handle.lock().await;
        let mut cfg = config("t-replacement", Some(&finished.id));
        let old_probe = DeliveryProbe::new(&mut cfg);
        let registering = tokio::spawn({
            let cfg = cfg.clone();
            async move { JobFinishedTriggerHandler.register_trigger(cfg).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !subscribers().lock().unwrap().contains_key(&cfg.id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut replacement = config(&cfg.id, None);
        let new_probe = DeliveryProbe::new(&mut replacement);
        JobFinishedTriggerHandler
            .register_trigger(replacement.clone())
            .await
            .unwrap();
        drop(held);
        registering.await.unwrap().unwrap();
        let old_events = old_probe.take();
        let replayed = new_probe.take();
        fire(&finished).await;
        let live_events = new_probe.take();
        JobFinishedTriggerHandler
            .unregister_trigger(replacement)
            .await
            .unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&finished.id);
        assert!(old_events.is_empty(), "the removed target must stay silent");
        assert!(
            replayed.is_empty(),
            "a catch-all must not inherit an old replay"
        );
        assert_eq!(
            live_events.len(),
            1,
            "the replacement must receive future events"
        );
    }

    #[tokio::test]
    async fn a_cancelled_job_waits_for_its_output_before_replay() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut killed = record(JobStatus::Killed, Some(crate::jobs::now_ms()));
        killed.id = "job-cancelled-before-finalization".into();
        killed.exit_code = None;
        killed.stdout.clear();
        let handle = std::sync::Arc::new(tokio::sync::Mutex::new(crate::jobs::JobHandle {
            record: killed,
            finalized: false,
            child: None,
            host_pid: None,
        }));
        let job_id = handle.lock().await.record.id.clone();
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(job_id.clone(), handle.clone());
        let mut cfg = config("t-cancelled", Some(&job_id));
        let probe = DeliveryProbe::new(&mut cfg);
        JobFinishedTriggerHandler
            .register_trigger(cfg.clone())
            .await
            .unwrap();
        let early_events = probe.take();
        let finished = {
            let mut h = handle.lock().await;
            h.record.stdout = "final output".into();
            h.record.exit_code = Some(137);
            h.finalized = true;
            h.record.clone()
        };
        fire(&finished).await;
        let final_events = probe.take();
        JobFinishedTriggerHandler
            .unregister_trigger(cfg)
            .await
            .unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&job_id);
        assert!(
            early_events.is_empty(),
            "a kill request is not a completed job"
        );
        assert_eq!(final_events.len(), 1);
        assert_eq!(final_events[0].exit_code, Some(137));
        assert_eq!(final_events[0].status, JobStatus::Killed);
    }

    #[test]
    fn trigger_config_and_event_schemas_describe_the_wire_fields() {
        let config = serde_json::to_value(schemars::schema_for!(JobFinishedConfig)).unwrap();
        assert_eq!(config["type"], "object");
        assert_eq!(
            config["properties"]["job_id"]["type"],
            json!(["string", "null"])
        );
        assert!(config.get("required").is_none(), "the filter is optional");
        let event = serde_json::to_value(schemars::schema_for!(JobFinishedEvent)).unwrap();
        assert_eq!(event["properties"]["argv"]["items"]["type"], "string");
        for field in [
            "job_id",
            "argv",
            "status",
            "started_at_ms",
            "finished_at_ms",
            "duration_ms",
        ] {
            assert!(
                event["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(field)),
                "missing {field}"
            );
        }
        assert!(event["properties"].get("stdout").is_none());
        assert!(event["properties"].get("stderr").is_none());
    }

    #[tokio::test]
    async fn invalid_job_id_is_rejected_instead_of_watching_all_jobs() {
        let _test_guard = TEST_GUARD.lock().await;
        let handler = JobFinishedTriggerHandler;
        let mut cfg = config("t-invalid-filter", None);
        cfg.config = json!({ "job_id": 123 });
        let result = handler.register_trigger(cfg.clone()).await;
        handler.unregister_trigger(cfg).await.unwrap();
        assert!(
            result.is_err(),
            "a malformed filter must not become a catch-all"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sandbox_rpc_timeout_announces_failure() {
        let _test_guard = TEST_GUARD.lock().await;
        struct UnresponsiveEngine;
        #[async_trait]
        impl crate::triggers::TriggerFwd for UnresponsiveEngine {
            async fn trigger(&self, _: &str, _: Value) -> Result<Value, Error> {
                std::future::pending().await
            }
        }
        let _guard = crate::jobs::GAUGE_TEST_GUARD.lock().await;
        let response = crate::functions::exec_bg::spawn_sandbox_job(
            None,
            std::sync::Arc::new(crate::config::ShellConfig::default()),
            std::sync::Arc::new(UnresponsiveEngine),
            uuid::Uuid::new_v4(),
            vec!["echo".into(), "ok".into()],
            1_000,
        )
        .await
        .unwrap();
        let handler = JobFinishedTriggerHandler;
        let mut cfg = config("t-rpc-timeout", Some(&response.job_id));
        let probe = DeliveryProbe::new(&mut cfg);
        handler.register_trigger(cfg.clone()).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(32)).await;
        tokio::task::yield_now().await;
        let handle = crate::jobs::get(&response.job_id).await.unwrap();
        let finished = handle.lock().await.record.clone();
        let remaining = take_matching(&response.job_id);
        handler.unregister_trigger(cfg).await.unwrap();
        crate::jobs::JOBS.map.lock().await.remove(&response.job_id);
        assert_eq!(finished.status, JobStatus::Failed);
        let delivered = probe.take();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].status, JobStatus::Failed);
        assert!(finished.stderr.contains("RPC timed out"));
        assert!(
            remaining.is_empty(),
            "the failed job must notify its subscriber"
        );
    }

    // A running job has no completion to announce, and a terminal one with
    // no finished_at_ms is mid-finalize: both must stay silent rather than
    // wake a subscriber with a half-written record.
    #[test]
    fn only_terminal_records_with_a_timestamp_become_events() {
        assert!(JobFinishedEvent::from_record(&record(JobStatus::Running, Some(3_000))).is_none());
        assert!(JobFinishedEvent::from_record(&record(JobStatus::Finished, None)).is_none());
        let event = JobFinishedEvent::from_record(&record(JobStatus::Finished, Some(3_500)))
            .expect("terminal record with a timestamp");
        assert_eq!(event.duration_ms, 2_500);
        assert_eq!(event.job_id, "job-1");
        // Output stays out of the payload; shell::status serves it.
        let payload = serde_json::to_value(&event).unwrap();
        assert!(payload.get("stdout").is_none());
    }

    // A clock that goes backwards between spawn and finalize must not
    // underflow the duration.
    #[test]
    fn a_backwards_clock_saturates_to_zero() {
        let event = JobFinishedEvent::from_record(&record(JobStatus::Killed, Some(1)))
            .expect("terminal record");
        assert_eq!(event.duration_ms, 0);
    }

    // The whole point of the job_id filter: a wake armed on one job must
    // not fire for another's.
    #[test]
    fn the_job_id_filter_selects_and_an_absent_one_takes_everything() {
        let mine = Subscriber::from_config(&config("t-1", Some("job-1"))).unwrap();
        let all = Subscriber::from_config(&config("t-2", None)).unwrap();
        assert!(mine.wants("job-1"));
        assert!(!mine.wants("job-2"));
        assert!(all.wants("job-1"));
        assert!(all.wants("job-2"));
    }

    // MOT-4719: a fire that loses the binding's metadata is dropped by the
    // harness silently, which is indistinguishable from a dead trigger type.
    // The SDK keeps the assembled request's fields `pub(crate)` and does not
    // implement Serialize on it, so Debug is the only seam from here — worth
    // the brittleness to pin the one bug that made a whole trigger type
    // invisible for two releases.
    #[test]
    fn the_fire_carries_the_bindings_metadata_and_namespace() {
        let subscriber = Subscriber::from_config(&config("t-1", Some("job-1"))).unwrap();
        let wire = format!("{:?}", subscriber.request(json!({ "job_id": "job-1" })));
        assert!(wire.contains("__binding"), "metadata dropped: {wire}");
        assert!(wire.contains("b-1"), "metadata dropped: {wire}");
        assert!(wire.contains("default"), "namespace dropped: {wire}");

        // A binding that carried neither must not invent them.
        let bare = Subscriber {
            registration_id: uuid::Uuid::new_v4(),
            function_id: "agent::wake".into(),
            metadata: None,
            namespace: None,
            job_id: None,
            delivered: false,
        };
        let wire = format!("{:?}", bare.request(json!({})));
        assert!(wire.contains("metadata: None"), "{wire}");
        assert!(wire.contains("namespace: None"), "{wire}");
    }

    // register/unregister are the only writers; a removed binding must stop
    // matching immediately.
    #[tokio::test]
    async fn registering_then_unregistering_leaves_no_subscriber() {
        let _test_guard = TEST_GUARD.lock().await;
        let handler = JobFinishedTriggerHandler;
        let cfg = config("t-unreg", Some("job-unreg"));
        handler.register_trigger(cfg.clone()).await.unwrap();
        assert_eq!(take_matching("job-unreg").len(), 1);
        handler.unregister_trigger(cfg).await.unwrap();
        assert!(take_matching("job-unreg").is_empty());
    }

    // Without a client installed (unit tests, or before startup wiring
    // lands) the fan-out must be a no-op, not a panic.
    #[tokio::test]
    async fn fire_without_a_client_is_silent() {
        let _test_guard = TEST_GUARD.lock().await;
        let handler = JobFinishedTriggerHandler;
        let cfg = config("t-noclient", Some("job-1"));
        handler.register_trigger(cfg.clone()).await.unwrap();
        fire(&record(JobStatus::Finished, Some(3_000))).await;
        handler.unregister_trigger(cfg).await.unwrap();
    }
}
