//! `shell::on-job-finished` — called once when a background job reaches a
//! terminal status, so a consumer never has to poll `shell::status`.
//!
//! `shell::exec_bg` hands back a `job_id` and nothing else; without this
//! function the only way to learn a job ended is to ask again. For an agent
//! that pays a model round trip per call that is the most expensive shape
//! available, and agents route around it with worse ones — `shell::exec
//! sleep 60`, which the foreground timeout clamps to 30s anyway (MOT-4766).
//!
//! This wake cannot strand its observer: the producer is the job, the job
//! is already running, and the runtime stamps the exact moment it ends.
//!
//! The function's match hook files an observer with `args: { job_id }` under
//! that id and one without under `*`; a call matches its job's id and `*`,
//! so it reaches the observers of its job and every catch-all, and iii
//! delivers it. An observer of one job added after that job finished, while
//! the worker still retains its record, receives the terminal event through
//! itself once it is accepted.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    AddedHook, Call, CallAction, CallRequest, IIIClient, Keys, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::jobs::{JobRecord, JobStatus};

/// The function a consumer observes to be woken when a job ends.
pub const ON_JOB_FINISHED_FN: &str = "shell::on-job-finished";

/// The match key of an observer without a `job_id`, and of every call.
const ANY_JOB: &str = "*";

/// An observer's args: one background job, or every future completion.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct JobFinishedArgs {
    /// The job to wake on. An observer added after the job finished
    /// receives its terminal event while the worker retains the job's
    /// record. Omitted: every future completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// What ended. Lean by design, like [`crate::events::ChangedEvent`]: a
/// consumer that wants the job's output asks `shell::status`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobFinishedEvent {
    pub job_id: String,
    /// The command as spawned, so an observer without a `job_id` can tell
    /// jobs apart without a second call.
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

/// The key an observer is filed under: its `job_id`, or `*`.
fn observer_key(args: &Value) -> &str {
    args.get("job_id")
        .and_then(Value::as_str)
        .unwrap_or(ANY_JOB)
}

/// The keys a call matches: its job's id and `*`.
fn call_keys(input: &Value) -> Vec<&str> {
    match input.get("job_id").and_then(Value::as_str) {
        Some(job_id) => vec![job_id, ANY_JOB],
        None => Vec::new(),
    }
}

/// The client calls go through, installed once at startup. `None` in unit
/// tests, which makes [`fire`] a no-op rather than a panic.
///
/// A process global next to [`crate::jobs::JOBS`] and its kill-signal map,
/// for the same reason those are: the job registry is process-wide, and
/// both `exec_bg` finalize paths (host drain and sandbox response) end in
/// detached tasks that carry no worker state.
static CLIENT: OnceLock<IIIClient> = OnceLock::new();

/// Observers of one job that its live call reached, by observer id: their
/// replay, when it has yet to run, stays silent. An entry goes with its
/// observer.
static REACHED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn reached() -> std::sync::MutexGuard<'static, HashSet<String>> {
    REACHED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Announce a terminal job record. Call it from a job's finalize path
/// AFTER the record is published, never while holding the job's lock: a
/// slow target must not delay the job that woke it.
///
/// Silent no-op when the job is not terminal, when nobody observes the job,
/// or before the function is registered.
pub async fn fire(record: &JobRecord) {
    let Some(event) = JobFinishedEvent::from_record(record) else {
        return;
    };
    announce(&event).await;
}

async fn announce(event: &JobFinishedEvent) {
    #[cfg(test)]
    tests::record_announced(event);
    let Some(iii) = CLIENT.get() else {
        return;
    };
    let input = json!({ "job_id": event.job_id });
    let probe = Call {
        input,
        ..Call::default()
    };
    if !iii.is_observed_for(ON_JOB_FINISHED_FN, &probe) {
        return;
    }
    let Ok(payload) = serde_json::to_value(event) else {
        return;
    };
    // The observers of this job active now are the ones the call reaches.
    reached().extend(
        iii.observers(ON_JOB_FINISHED_FN)
            .iter()
            .filter(|obs| observer_key(obs.args_value()) == event.job_id)
            .map(|obs| obs.id().to_string()),
    );
    let request = CallRequest {
        function_id: ON_JOB_FINISHED_FN.to_string(),
        payload,
        action: Some(CallAction::Void),
        timeout_ms: None,
    };
    if let Err(e) = iii.call(request).await {
        tracing::warn!(job_id = %event.job_id, error = %e, "{ON_JOB_FINISHED_FN} call failed");
    } else {
        tracing::info!(job_id = %event.job_id, "{ON_JOB_FINISHED_FN} called");
    }
}

/// The terminal event of a job the worker still retains, once its record
/// is final.
async fn retained_event(job_id: &str) -> Option<JobFinishedEvent> {
    let handle = crate::jobs::get(job_id).await?;
    let h = handle.lock().await;
    // shell::kill stamps a terminal status before the drain or sandbox
    // response has finished collecting the output.
    if !h.finalized {
        return None;
    }
    JobFinishedEvent::from_record(&h.record)
}

/// Whether observer `id` still needs its job's terminal event replayed:
/// the live call has not reached it.
fn wants_replay(id: &str) -> bool {
    !reached().contains(id)
}

/// `shell::on-job-finished`'s hooks.
#[derive(Clone)]
struct JobFinishedSource {
    iii: IIIClient,
}

#[async_trait]
impl AddedHook for JobFinishedSource {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        let args: JobFinishedArgs = obs.args()?;
        tracing::info!(
            function_id = ON_JOB_FINISHED_FN,
            observer_id = %obs.id(),
            job_id = ?args.job_id,
            "job watch observed"
        );
        // An observer without a job_id waits for future completions.
        let Some(job_id) = args.job_id else {
            return Ok(());
        };
        // A call through an observer that is not active yet fails: replay
        // once the SDK has accepted it. A job that finishes after that
        // reaches it through the live call.
        let iii = self.iii.clone();
        tokio::spawn(async move {
            if obs.accepted().await.is_err() {
                return;
            }
            let Some(event) = retained_event(&job_id).await else {
                return;
            };
            if !wants_replay(obs.id()) {
                return;
            }
            let Ok(payload) = serde_json::to_value(&event) else {
                return;
            };
            let request = CallRequest::observer(&obs, payload).action(CallAction::Void);
            if let Err(e) = iii.call(request).await {
                tracing::warn!(job_id = %job_id, error = %e, "{ON_JOB_FINISHED_FN} replay failed");
            }
        });
        Ok(())
    }
}

#[async_trait]
impl RemovedHook for JobFinishedSource {
    async fn on_removed(&self, obs: Observer) {
        tracing::info!(function_id = ON_JOB_FINISHED_FN, observer_id = %obs.id(), "job watch released");
        reached().remove(obs.id());
    }
}

/// Register `shell::on-job-finished` with its hooks, set by id first so they
/// run for every observer, and install the client calls go through.
pub fn register_job_finished(iii: &IIIClient) {
    let _ = CLIENT.set(iii.clone());
    let source = JobFinishedSource { iii: iii.clone() };
    iii.on_observer_added_for(ON_JOB_FINISHED_FN, source.clone())
        .on_observer_removed_for(ON_JOB_FINISHED_FN, source)
        .on_observer_match_for(
            ON_JOB_FINISHED_FN,
            Match::keys(
                |obs| Keys::from(observer_key(obs.args_value())),
                |call| Keys::from(call_keys(&call.input)),
            ),
        );
    iii.register_function(
        ON_JOB_FINISHED_FN,
        RegisterFunction::new(|event: JobFinishedEvent| Ok::<_, Error>(event))
            .description(
                "Called once when a shell::exec_bg background job reaches a terminal status \
                 (finished, killed, or failed); observers receive { job_id, argv, status, \
                 exit_code, started_at_ms, finished_at_ms, duration_ms }. Observe with args \
                 { job_id } to wake on one job (an observer added after the job finished \
                 receives its result while the job is retained), or without args for every \
                 future completion. Call shell::status for the job's output.",
            )
            .args_format::<JobFinishedArgs>()
            .private(),
    );
    tracing::info!(
        function_id = ON_JOB_FINISHED_FN,
        "registered the background-job completion feed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    static TEST_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static ANNOUNCED: OnceLock<Mutex<HashMap<String, Vec<JobFinishedEvent>>>> = OnceLock::new();

    /// Every announced event, by job id: the SDK boundary without a live
    /// engine.
    pub(super) fn record_announced(event: &JobFinishedEvent) {
        ANNOUNCED
            .get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .entry(event.job_id.clone())
            .or_default()
            .push(event.clone());
    }

    fn announced(job_id: &str) -> Vec<JobFinishedEvent> {
        ANNOUNCED
            .get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .remove(job_id)
            .unwrap_or_default()
    }

    /// The events announced for `job_id`, waiting for the first: the
    /// finalize task announces right after it marks the job final.
    async fn announced_after_finalize(job_id: &str) -> Vec<JobFinishedEvent> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while ANNOUNCED
                .get_or_init(Mutex::default)
                .lock()
                .unwrap()
                .get(job_id)
                .is_none()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        announced(job_id)
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

    fn retain(
        record: JobRecord,
        finalized: bool,
    ) -> std::sync::Arc<tokio::sync::Mutex<crate::jobs::JobHandle>> {
        std::sync::Arc::new(tokio::sync::Mutex::new(crate::jobs::JobHandle {
            record,
            finalized,
            child: None,
            host_pid: None,
            owner_session_id: None,
        }))
    }

    #[tokio::test]
    async fn an_observer_added_after_completion_gets_the_retained_event() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut finished = record(JobStatus::Finished, Some(crate::jobs::now_ms()));
        finished.id = "job-late-observer".into();
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(finished.id.clone(), retain(finished.clone(), true));
        let event = retained_event(&finished.id).await;
        crate::jobs::JOBS.map.lock().await.remove(&finished.id);
        let event = event.expect("a finalized terminal job replays");
        assert_eq!(event.job_id, finished.id);
        assert_eq!(event.status, JobStatus::Finished);
        assert!(retained_event("job-never-ran").await.is_none());
    }

    #[tokio::test]
    async fn a_named_exec_bg_job_announces_its_completion_once() {
        let _test_guard = TEST_GUARD.lock().await;
        let _gauge_guard = crate::jobs::GAUGE_TEST_GUARD.lock().await;
        let _sweep_guard = crate::jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let job_id = format!("pre-observed-{}", uuid::Uuid::new_v4());
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
        let events = announced_after_finalize(&job_id).await;
        crate::jobs::JOBS.map.lock().await.remove(&response.job_id);
        assert_eq!(response.job_id, job_id);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].job_id, job_id);
        assert_eq!(events[0].status, JobStatus::Finished);
        assert_eq!(events[0].exit_code, Some(0));
    }

    #[tokio::test]
    async fn a_named_sandbox_job_runs_once_and_announces_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let _test_guard = TEST_GUARD.lock().await;
        let _gauge_guard = crate::jobs::GAUGE_TEST_GUARD.lock().await;
        struct Engine(AtomicUsize);
        #[async_trait]
        impl crate::calls::CallFwd for Engine {
            async fn call(&self, _: &str, _: Value) -> Result<Value, Error> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(
                    json!({"stdout": "sandbox output", "stderr": "", "exit_code": 0,
                    "duration_ms": 1, "timed_out": false}),
                )
            }
        }
        let job_id = format!("sandbox-pre-observed-{}", uuid::Uuid::new_v4());
        let engine = Arc::new(Engine(AtomicUsize::new(0)));
        let shell_cfg = Arc::new(crate::config::ShellConfig::default());
        let sandbox_id = uuid::Uuid::new_v4();
        let response = crate::functions::exec_bg::spawn_sandbox_job(
            None,
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
            None,
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
        let events = announced_after_finalize(&job_id).await;
        let record = handle.lock().await.record.clone();
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
    async fn a_cancelled_job_waits_for_its_output_before_replay() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut killed = record(JobStatus::Killed, Some(crate::jobs::now_ms()));
        killed.id = "job-cancelled-before-finalization".into();
        killed.exit_code = None;
        killed.stdout.clear();
        let job_id = killed.id.clone();
        let handle = retain(killed, false);
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(job_id.clone(), handle.clone());
        let early = retained_event(&job_id).await;
        let finished = {
            let mut h = handle.lock().await;
            h.record.stdout = "final output".into();
            h.record.exit_code = Some(137);
            h.finalized = true;
            h.record.clone()
        };
        fire(&finished).await;
        let replayed = retained_event(&job_id).await;
        let live = announced(&job_id);
        crate::jobs::JOBS.map.lock().await.remove(&job_id);
        assert!(early.is_none(), "a kill request is not a completed job");
        let replayed = replayed.expect("final once the output is in");
        assert_eq!(replayed.exit_code, Some(137));
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].exit_code, Some(137));
        assert_eq!(live[0].status, JobStatus::Killed);
    }

    #[test]
    fn args_and_event_schemas_describe_the_wire_fields() {
        let args = serde_json::to_value(schemars::schema_for!(JobFinishedArgs)).unwrap();
        assert_eq!(args["type"], "object");
        assert_eq!(
            args["properties"]["job_id"]["type"],
            json!(["string", "null"])
        );
        assert!(args.get("required").is_none(), "job_id is optional");
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

    #[test]
    fn a_malformed_job_id_is_refused_instead_of_watching_all_jobs() {
        assert!(serde_json::from_value::<JobFinishedArgs>(json!({ "job_id": 123 })).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn sandbox_rpc_timeout_announces_failure() {
        let _test_guard = TEST_GUARD.lock().await;
        struct UnresponsiveEngine;
        #[async_trait]
        impl crate::calls::CallFwd for UnresponsiveEngine {
            async fn call(&self, _: &str, _: Value) -> Result<Value, Error> {
                std::future::pending().await
            }
        }
        let _guard = crate::jobs::GAUGE_TEST_GUARD.lock().await;
        let response = crate::functions::exec_bg::spawn_sandbox_job(
            None,
            None,
            std::sync::Arc::new(crate::config::ShellConfig::default()),
            std::sync::Arc::new(UnresponsiveEngine),
            uuid::Uuid::new_v4(),
            vec!["echo".into(), "ok".into()],
            1_000,
        )
        .await
        .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(32)).await;
        tokio::task::yield_now().await;
        let handle = crate::jobs::get(&response.job_id).await.unwrap();
        let finished = handle.lock().await.record.clone();
        let delivered = announced(&response.job_id);
        crate::jobs::JOBS.map.lock().await.remove(&response.job_id);
        assert_eq!(finished.status, JobStatus::Failed);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].status, JobStatus::Failed);
        assert!(finished.stderr.contains("RPC timed out"));
    }

    // A running job has no completion to announce, and a terminal one with
    // no finished_at_ms is mid-finalize: both must stay silent rather than
    // wake an observer with a half-written record.
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

    // The whole point of the job_id arg: a wake armed on one job must not
    // fire for another's, and one without it hears every job.
    #[test]
    fn a_job_id_selects_its_job_and_an_absent_one_takes_every_job() {
        let (job_1, job_2, any) = (
            json!({ "job_id": "job-1" }),
            json!({ "job_id": "job-2" }),
            json!({}),
        );
        assert_eq!(observer_key(&job_1), "job-1");
        assert_eq!(observer_key(&any), ANY_JOB);
        let keys = call_keys(&job_1);
        assert_eq!(keys, ["job-1", ANY_JOB]);
        assert!(keys.contains(&observer_key(&job_1)));
        assert!(!keys.contains(&observer_key(&job_2)));
        assert!(keys.contains(&observer_key(&any)));
        assert!(call_keys(&any).is_empty());
    }

    // An observer the live call reached is not replayed; its entry goes
    // with it.
    #[test]
    fn an_observer_reached_live_is_not_replayed() {
        let id = format!("obs-{}", uuid::Uuid::new_v4());
        assert!(wants_replay(&id));
        reached().insert(id.clone());
        assert!(!wants_replay(&id));
        reached().remove(&id);
        assert!(wants_replay(&id));
    }

    // Without a client installed (unit tests, or before startup wiring
    // lands) the call must be a no-op, not a panic.
    #[tokio::test]
    async fn fire_without_a_client_is_silent() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut finished = record(JobStatus::Finished, Some(3_000));
        finished.id = "job-no-client".into();
        fire(&finished).await;
        assert_eq!(announced(&finished.id).len(), 1);
    }
}
