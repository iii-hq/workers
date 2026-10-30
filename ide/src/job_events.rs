//! `shell::on-job-finished` — called when a background job reaches a
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
//! The function's match hook picks the observers a call reaches: a job's
//! completion reaches every observer without a `job_id` and each observer of
//! that job, and iii delivers it. An observer of one job added after that
//! job finished, while the worker still retains its record, receives the
//! completion from a second call of the function once it is accepted. That
//! call carries metadata `{ "for_observer": <observer id> }`, which the
//! match hook gives to that observer alone, and iii delivers it as it
//! delivers the first: with the observer's `deliver`, filters and `on`. The
//! match hook claims a completion for an observer of its job under one
//! lock, whichever call comes first, so the observer receives it once.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    AddedHook, Call, CallAction, CallRequest, IIIClient, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::jobs::{JobRecord, JobStatus};

/// The function a consumer observes to be woken when a job ends.
pub const ON_JOB_FINISHED_FN: &str = "shell::on-job-finished";

/// The call metadata field naming the one observer a call is for.
const FOR_OBSERVER: &str = "for_observer";

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

/// One run of a job ending: what each observer of the job receives once.
/// A named job's id is free again once its record is pruned, so the times
/// tell its runs apart.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Completion {
    job_id: String,
    started_at_ms: u64,
    finished_at_ms: u64,
}

impl Completion {
    /// The completion a call carries in its input.
    fn of(input: &Value) -> Option<Self> {
        Some(Self {
            job_id: input.get("job_id")?.as_str()?.to_string(),
            started_at_ms: input.get("started_at_ms")?.as_u64()?,
            finished_at_ms: input.get("finished_at_ms")?.as_u64()?,
        })
    }
}

impl From<&JobFinishedEvent> for Completion {
    fn from(event: &JobFinishedEvent) -> Self {
        Self {
            job_id: event.job_id.clone(),
            started_at_ms: event.started_at_ms,
            finished_at_ms: event.finished_at_ms,
        }
    }
}

/// The client calls go through, installed once at startup. `None` in unit
/// tests, which makes [`fire`] and the replay no-ops rather than panics.
///
/// A process global next to [`crate::jobs::JOBS`] and its kill-signal map,
/// for the same reason those are: the job registry is process-wide, and
/// both `exec_bg` finalize paths (host drain and sandbox response) end in
/// detached tasks that carry no worker state.
static CLIENT: OnceLock<IIIClient> = OnceLock::new();

/// The completion each observer of one job has taken, by observer id. The
/// match hook claims it under this lock for the live call and the replay
/// alike, so an observer receives a completion once. An entry goes with its
/// observer.
static CLAIMED: OnceLock<Mutex<HashMap<String, Completion>>> = OnceLock::new();

fn claimed() -> MutexGuard<'static, HashMap<String, Completion>> {
    CLAIMED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Claims `completion` for observer `id`; false when the observer has it.
fn claim(id: &str, completion: &Completion) -> bool {
    let mut claimed = claimed();
    if claimed.get(id) == Some(completion) {
        return false;
    }
    claimed.insert(id.to_string(), completion.clone());
    true
}

/// Whether observer `id` has taken `completion`.
fn has_claimed(id: &str, completion: &Completion) -> bool {
    claimed().get(id) == Some(completion)
}

/// The observer a call is for, from its metadata `for_observer`; `None`
/// for a call of every observer it matches.
fn for_observer(call: &Call) -> Option<&str> {
    call.metadata.as_ref()?.get(FOR_OBSERVER)?.as_str()
}

/// The function's match hook: whether observer `id`, with `args`, takes
/// `call`. A call for one observer reaches that observer alone. A job's
/// completion reaches every observer without a `job_id`, and each observer
/// of that job that has not taken it.
fn wants(id: &str, args: &Value, call: &Call) -> bool {
    if for_observer(call).is_some_and(|target| target != id) {
        return false;
    }
    let Some(completion) = Completion::of(&call.input) else {
        return false;
    };
    match args.get("job_id").and_then(Value::as_str) {
        None => true,
        Some(job_id) => job_id == completion.job_id && claim(id, &completion),
    }
}

/// Announce a terminal job record. Call it from a job's finalize path
/// AFTER the record is published, never while holding the job's lock: a
/// slow target must not delay the job that woke it.
///
/// Silent no-op when the job is not terminal, when nobody observes the
/// function, or before the function is registered.
pub async fn fire(record: &JobRecord) {
    let Some(event) = JobFinishedEvent::from_record(record) else {
        return;
    };
    #[cfg(test)]
    tests::record_announced(&event);
    if CLIENT
        .get()
        .is_some_and(|iii| iii.is_observed(ON_JOB_FINISHED_FN))
    {
        call(&event, None).await;
    }
}

/// Calls the function with `event`, for every observer it reaches or, with
/// `for_observer`, for that one observer.
async fn call(event: &JobFinishedEvent, for_observer: Option<&str>) {
    let Some(iii) = CLIENT.get() else {
        return;
    };
    let Ok(payload) = serde_json::to_value(event) else {
        return;
    };
    let request = CallRequest {
        function_id: ON_JOB_FINISHED_FN.to_string(),
        payload,
        action: Some(CallAction::Void),
        timeout_ms: None,
    };
    let result = match for_observer {
        None => iii.call(request).await,
        Some(observer_id) => {
            iii.call(request.metadata(json!({ FOR_OBSERVER: observer_id })))
                .await
        }
    };
    match result {
        Err(e) => {
            tracing::warn!(job_id = %event.job_id, for_observer, error = %e, "{ON_JOB_FINISHED_FN} call failed")
        }
        Ok(_) => {
            tracing::info!(job_id = %event.job_id, for_observer, "{ON_JOB_FINISHED_FN} called")
        }
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

/// Replays job `job_id`'s completion to observer `observer_id`, an accepted
/// observer of that job: a call for that observer alone, when the job has
/// finished and the observer has not taken its completion. A job that ends
/// later reaches the observer through its first call.
async fn replay(observer_id: &str, job_id: &str) {
    let Some(event) = retained_event(job_id).await else {
        return;
    };
    if has_claimed(observer_id, &Completion::from(&event)) {
        return;
    }
    #[cfg(test)]
    tests::record_replayed(observer_id, &event);
    call(&event, Some(observer_id)).await;
}

/// `shell::on-job-finished`'s hooks.
#[derive(Clone)]
struct JobFinishedSource;

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
        // A call made before the observer is active does not reach it:
        // replay once the SDK has accepted it. A rejected observer gets
        // nothing.
        tokio::spawn(async move {
            if obs.accepted().await.is_ok() {
                replay(obs.id(), &job_id).await;
            }
        });
        Ok(())
    }
}

#[async_trait]
impl RemovedHook for JobFinishedSource {
    async fn on_removed(&self, obs: Observer) {
        tracing::info!(function_id = ON_JOB_FINISHED_FN, observer_id = %obs.id(), "job watch released");
        claimed().remove(obs.id());
    }
}

/// Register `shell::on-job-finished` with its hooks, set by id first so they
/// run for every observer, and install the client calls go through.
pub fn register_job_finished(iii: &IIIClient) {
    let _ = CLIENT.set(iii.clone());
    iii.on_observer_added_for(ON_JOB_FINISHED_FN, JobFinishedSource)
        .on_observer_removed_for(ON_JOB_FINISHED_FN, JobFinishedSource)
        .on_observer_match_for(
            ON_JOB_FINISHED_FN,
            Match::predicate(|obs, call| wants(obs.id(), obs.args_value(), call)),
        );
    iii.register_function(
        ON_JOB_FINISHED_FN,
        RegisterFunction::new(|event: JobFinishedEvent| Ok::<_, Error>(event))
            .description(
                "Called when a shell::exec_bg background job reaches a terminal status \
                 (finished, killed, or failed); observers receive { job_id, argv, status, \
                 exit_code, started_at_ms, finished_at_ms, duration_ms }. Observe with args \
                 { job_id } to wake on one job, or without args for every future completion. \
                 An observer of one job added after the job finished receives its completion \
                 while the job is retained, from a call for that observer alone (metadata \
                 { for_observer: <observer id> }), delivered like every call. Each observer of \
                 a job receives its completion once. Call shell::status for the job's output.",
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
    static REPLAYED: OnceLock<Mutex<HashMap<String, Vec<JobFinishedEvent>>>> = OnceLock::new();

    /// Every replayed event, by observer id: the SDK boundary without a live
    /// engine.
    pub(super) fn record_replayed(observer_id: &str, event: &JobFinishedEvent) {
        REPLAYED
            .get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .entry(observer_id.to_string())
            .or_default()
            .push(event.clone());
    }

    fn replayed(observer_id: &str) -> Vec<JobFinishedEvent> {
        REPLAYED
            .get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .remove(observer_id)
            .unwrap_or_default()
    }

    /// The call that announces `event` to every observer it reaches.
    fn live_call(event: &JobFinishedEvent) -> Call {
        Call {
            function_id: ON_JOB_FINISHED_FN.into(),
            input: serde_json::to_value(event).unwrap(),
            ..Call::default()
        }
    }

    /// The call that replays `event` to observer `observer_id`.
    fn replay_call(event: &JobFinishedEvent, observer_id: &str) -> Call {
        Call {
            metadata: Some(json!({ "for_observer": observer_id })),
            ..live_call(event)
        }
    }

    fn observer_id() -> String {
        format!("obs-{}", uuid::Uuid::new_v4())
    }

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
        finished.id = format!("job-late-observer-{}", uuid::Uuid::new_v4());
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(finished.id.clone(), retain(finished.clone(), true));
        let (late, never) = (observer_id(), observer_id());
        replay(&late, &finished.id).await;
        replay(&never, "job-never-ran").await;
        crate::jobs::JOBS.map.lock().await.remove(&finished.id);
        let events = replayed(&late);
        assert_eq!(events.len(), 1, "a finalized terminal job replays");
        assert_eq!(events[0].job_id, finished.id);
        assert_eq!(events[0].status, JobStatus::Finished);
        // The replay names its observer: that observer takes it, no other.
        let call = replay_call(&events[0], &late);
        let args = json!({ "job_id": finished.id });
        assert!(!wants(&observer_id(), &args, &call));
        assert!(!wants(&observer_id(), &json!({}), &call));
        assert!(wants(&late, &args, &call));
        claimed().remove(&late);
        assert!(replayed(&never).is_empty(), "a job never retained");
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
    fn a_completion_reaches_the_observers_of_its_job_and_every_catch_all() {
        let mut event = JobFinishedEvent::from_record(&record(JobStatus::Finished, Some(2_000)))
            .expect("terminal record");
        event.job_id = format!("job-{}", uuid::Uuid::new_v4());
        let call = live_call(&event);
        let (mine, other, any) = (observer_id(), observer_id(), observer_id());
        assert!(wants(&mine, &json!({ "job_id": event.job_id }), &call));
        assert!(!wants(&other, &json!({ "job_id": "job-2" }), &call));
        assert!(wants(&any, &json!({}), &call));
        assert!(wants(&any, &json!({ "job_id": null }), &call));
        // A catch-all takes every completion and is never claimed.
        assert!(wants(&any, &json!({}), &call));
        assert!(!claimed().contains_key(&any));
        assert!(
            !wants(&any, &json!({}), &Call::default()),
            "no job, no match"
        );
        claimed().remove(&mine);
    }

    // A call with metadata `for_observer` reaches the observer it names and
    // no other: not another observer of the job, not a catch-all.
    #[test]
    fn a_call_for_one_observer_reaches_that_observer_alone() {
        let mut event = JobFinishedEvent::from_record(&record(JobStatus::Finished, Some(2_000)))
            .expect("terminal record");
        event.job_id = format!("job-{}", uuid::Uuid::new_v4());
        let args = json!({ "job_id": event.job_id });
        let (named, sibling, any) = (observer_id(), observer_id(), observer_id());
        let call = replay_call(&event, &named);
        assert_eq!(call.metadata, Some(json!({ "for_observer": named })));
        assert!(!wants(&sibling, &args, &call));
        assert!(!wants(&any, &json!({}), &call));
        assert!(!claimed().contains_key(&sibling), "nothing claimed for it");
        assert!(wants(&named, &args, &call));
        // The sibling still takes the live call.
        assert!(wants(&sibling, &args, &live_call(&event)));
        // Metadata without for_observer is a call of every observer.
        let tagged = Call {
            metadata: Some(json!({ "source": "test" })),
            ..live_call(&event)
        };
        assert!(wants(&any, &json!({}), &tagged));
        claimed().remove(&named);
        claimed().remove(&sibling);
    }

    // An observer added while its job ends sees two calls, the live one and
    // its replay; whichever is matched first is the one it takes.
    #[test]
    fn an_observer_takes_a_completion_once_whichever_call_comes_first() {
        let mut event = JobFinishedEvent::from_record(&record(JobStatus::Finished, Some(2_000)))
            .expect("terminal record");
        event.job_id = format!("job-{}", uuid::Uuid::new_v4());
        let args = json!({ "job_id": event.job_id });
        let (live_first, replay_first) = (observer_id(), observer_id());
        assert!(wants(&live_first, &args, &live_call(&event)));
        assert!(!wants(
            &live_first,
            &args,
            &replay_call(&event, &live_first)
        ));
        assert!(wants(
            &replay_first,
            &args,
            &replay_call(&event, &replay_first)
        ));
        assert!(!wants(&replay_first, &args, &live_call(&event)));
        assert!(!wants(
            &replay_first,
            &args,
            &replay_call(&event, &replay_first)
        ));
        // The same id run again after its record was pruned is a new
        // completion.
        let mut again = event.clone();
        again.started_at_ms += 10_000;
        again.finished_at_ms += 10_000;
        assert!(wants(&live_first, &args, &live_call(&again)));
        // An entry goes with its observer.
        claimed().remove(&live_first);
        claimed().remove(&replay_first);
        assert!(!has_claimed(&live_first, &Completion::from(&again)));
    }

    // The SDK matches the live call and a replay on different tasks: each
    // observer of the job takes the completion exactly once.
    #[test]
    fn racing_matches_give_each_observer_the_completion_once() {
        let mut event = JobFinishedEvent::from_record(&record(JobStatus::Finished, Some(2_000)))
            .expect("terminal record");
        event.job_id = format!("job-{}", uuid::Uuid::new_v4());
        let args = json!({ "job_id": event.job_id });
        let observers: Vec<String> = (0..64).map(|_| observer_id()).collect();
        let taken = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for id in &observers {
                    if wants(id, &args, &live_call(&event)) {
                        taken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
            scope.spawn(|| {
                for id in observers.iter().rev() {
                    if wants(id, &args, &replay_call(&event, id)) {
                        taken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        });
        assert_eq!(taken.into_inner(), observers.len());
        for id in &observers {
            claimed().remove(id);
        }
    }

    // A job that ends while an observer's replay task is reading it: the
    // live call is matched before the replay looks, and the replay stays
    // silent; or the replay is sent first, and its observer takes one of
    // the two calls.
    #[tokio::test]
    async fn a_completion_landing_while_the_replay_reads_the_job_reaches_the_observer_once() {
        let _test_guard = TEST_GUARD.lock().await;
        let mut running = record(JobStatus::Running, None);
        running.id = format!("job-race-{}", uuid::Uuid::new_v4());
        let job_id = running.id.clone();
        let handle = retain(running, false);
        crate::jobs::JOBS
            .map
            .lock()
            .await
            .insert(job_id.clone(), handle.clone());
        let args = json!({ "job_id": job_id });

        let (first, second) = (observer_id(), observer_id());
        let mut h = handle.lock().await;
        let reading = tokio::spawn({
            let (first, job_id) = (first.clone(), job_id.clone());
            async move { replay(&first, &job_id).await }
        });
        tokio::task::yield_now().await;
        assert!(!reading.is_finished(), "the replay waits on the job's lock");
        h.record.status = JobStatus::Finished;
        h.record.finished_at_ms = Some(2_000);
        h.finalized = true;
        let event = JobFinishedEvent::from_record(&h.record).unwrap();
        drop(h);
        // The finalize path's call is matched before the replay looks.
        assert!(wants(&first, &args, &live_call(&event)));
        reading.await.unwrap();
        assert!(replayed(&first).is_empty(), "the live call reached it");

        // The replay looks first: it is sent, and the observer takes
        // whichever of the two calls the SDK matches first.
        replay(&second, &job_id).await;
        assert_eq!(replayed(&second).len(), 1);
        let replay_taken = wants(&second, &args, &replay_call(&event, &second));
        let live_taken = wants(&second, &args, &live_call(&event));
        assert!(replay_taken ^ live_taken);

        crate::jobs::JOBS.map.lock().await.remove(&job_id);
        claimed().remove(&first);
        claimed().remove(&second);
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
