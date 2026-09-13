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
use serde::Serialize;
use serde_json::Value;

use crate::jobs::{JobRecord, JobStatus};

/// The trigger type a subscriber binds to be woken when a job ends.
pub const JOB_FINISHED: &str = "shell::job-finished";

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
    function_id: String,
    metadata: Option<Value>,
    namespace: Option<String>,
    /// Bind with `config: { job_id }` to be woken by one job only. Absent
    /// means every job — useful for a dashboard, wasteful for a wake.
    job_id: Option<String>,
}

impl Subscriber {
    fn from_config(config: &TriggerConfig) -> Self {
        Self {
            function_id: config.function_id.clone(),
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
            job_id: config
                .config
                .get("job_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }

    fn wants(&self, job_id: &str) -> bool {
        self.job_id.as_deref().is_none_or(|id| id == job_id)
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

fn matching(job_id: &str) -> Vec<Subscriber> {
    subscribers()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter(|s| s.wants(job_id))
        .cloned()
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
    let targets = matching(&event.job_id);
    if targets.is_empty() {
        return;
    }
    let Some(iii) = CLIENT.get() else {
        return;
    };
    let Ok(payload) = serde_json::to_value(&event) else {
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
        let subscriber = Subscriber::from_config(&config);
        tracing::info!(
            trigger_type = JOB_FINISHED,
            id = %config.id,
            job_id = ?subscriber.job_id,
            "job watch registered"
        );
        subscribers()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(config.id, subscriber);
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
    let _handle = iii.register_trigger_type(RegisterTriggerType::new(
        JOB_FINISHED,
        "Fires once when a shell::exec_bg background job reaches a terminal status \
         (finished, killed, or failed). Bind with config: { job_id } to wake on one \
         job, or omit it to receive every job. The event carries job_id, argv, status, \
         exit_code and timings; call shell::status for the job's output.",
        JobFinishedTriggerHandler,
    ));
    tracing::info!(
        trigger_type = JOB_FINISHED,
        "sent the trigger type registration; delivery is confirmed by the first subscription"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(status: JobStatus, finished_at_ms: Option<u64>) -> JobRecord {
        JobRecord {
            id: "job-1".into(),
            argv: vec!["npm".into(), "install".into()],
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
        let mine = Subscriber::from_config(&config("t-1", Some("job-1")));
        let all = Subscriber::from_config(&config("t-2", None));
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
        let subscriber = Subscriber::from_config(&config("t-1", Some("job-1")));
        let wire = format!("{:?}", subscriber.request(json!({ "job_id": "job-1" })));
        assert!(wire.contains("__binding"), "metadata dropped: {wire}");
        assert!(wire.contains("b-1"), "metadata dropped: {wire}");
        assert!(wire.contains("default"), "namespace dropped: {wire}");

        // A binding that carried neither must not invent them.
        let bare = Subscriber {
            function_id: "agent::wake".into(),
            metadata: None,
            namespace: None,
            job_id: None,
        };
        let wire = format!("{:?}", bare.request(json!({})));
        assert!(wire.contains("metadata: None"), "{wire}");
        assert!(wire.contains("namespace: None"), "{wire}");
    }

    // register/unregister are the only writers; a removed binding must stop
    // matching immediately.
    #[tokio::test]
    async fn registering_then_unregistering_leaves_no_subscriber() {
        let handler = JobFinishedTriggerHandler;
        let cfg = config("t-unreg", Some("job-unreg"));
        handler.register_trigger(cfg.clone()).await.unwrap();
        assert_eq!(matching("job-unreg").len(), 1);
        handler.unregister_trigger(cfg).await.unwrap();
        assert!(matching("job-unreg").is_empty());
    }

    // Without a client installed (unit tests, or before startup wiring
    // lands) the fan-out must be a no-op, not a panic.
    #[tokio::test]
    async fn fire_without_a_client_is_silent() {
        let handler = JobFinishedTriggerHandler;
        let cfg = config("t-noclient", Some("job-1"));
        handler.register_trigger(cfg.clone()).await.unwrap();
        fire(&record(JobStatus::Finished, Some(3_000))).await;
        handler.unregister_trigger(cfg).await.unwrap();
    }
}
