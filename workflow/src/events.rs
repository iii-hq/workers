use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::{
    Call, CallAction, CallRequest, IIIClient, Keys, Match, ObserveRejection, Observer,
    RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::WorkflowError;
use crate::functions::Deps;
use crate::locks::WorkflowLocks;
use crate::types::{NodeState, RunStatus, WorkflowRunRecord};

/// The event function every terminal run calls. Private: only this worker
/// calls it; anyone may observe it.
pub const ON_RUN_COMPLETED_FN: &str = "workflow::on-run-completed";
pub const ON_RUN_COMPLETED_DESC: &str =
    "A workflow run reached a terminal state (completed, failed or cancelled); the event is \
     {run_id, status, result, result_error, failed_nodes}. To wait for one run, observe it \
     with args {run_id}: a run that already ended is replayed at once, and an unknown or \
     collected run is refused with UNKNOWN_RUN. Without args it reports every run; `filters` \
     on input.run_id or input.status narrow that but get no replay, so never wait for one run \
     with filters. Delivery is at-least-once: dedup on run_id. A replay reaches a call target \
     as the bare event, not the observed envelope. A notification ending in `…(truncated)`, or \
     with an array ending in an `…N more entries omitted` string, is incomplete: call \
     workflow::status once for the run. If the wake expires unfired, observe again the same \
     way. Called by the workflow worker only.";

/// The observer args of `workflow::on-run-completed`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCompletedArgs {
    /// The one run to report; a run that already ended is replayed at once.
    /// Omit it to report every run.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// A run's terminal outcome: the `workflow::on-run-completed` event and the
/// `notify` callback payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunCompletedEvent {
    pub run_id: String,
    pub status: RunStatus,
    /// The run's result; null unless it completed.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// Why the run failed; null otherwise.
    #[serde(default)]
    pub result_error: Option<String>,
    /// `node '<uid>': <error>` per node that failed in a completed run (a
    /// tolerated `on_error: continue` failure); omitted when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_nodes: Vec<String>,
}

impl From<&WorkflowRunRecord> for RunCompletedEvent {
    fn from(record: &WorkflowRunRecord) -> Self {
        // Failed only: the siblings finalize cancelled are not failures, and a
        // replay of the stored record must read the same list as the live emit.
        let failed_nodes = if record.status == RunStatus::Completed {
            crate::functions::tick::node_errors(&record.nodes, &[NodeState::Failed])
        } else {
            Vec::new()
        };
        Self {
            run_id: record.run_id.clone(),
            status: record.status,
            result: record.result.clone(),
            result_error: record.result_error.clone(),
            failed_nodes,
        }
    }
}

/// The terminal-state payload, shared by the event function and the
/// caller-supplied callback.
fn completed_payload(record: &WorkflowRunRecord) -> serde_json::Value {
    serde_json::to_value(RunCompletedEvent::from(record)).unwrap_or(serde_json::Value::Null)
}

/// How the worker's event reaches iii: the bus in production, a recorder in
/// tests.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    /// Whether `call` of `function_id` reaches any observer, by the
    /// function's match hook.
    fn observed_for(&self, function_id: &str, call: &Call) -> bool;

    /// Whether an observer of this one run (args `{run_id}`) is active. An
    /// every-run observer does not count: it wakes no caller.
    fn run_observed(&self, run_id: &str) -> bool;

    /// Send `request`, best-effort: a failure is dropped.
    async fn deliver(&self, request: CallRequest);
}

#[async_trait]
impl EventDeliverer for IIIClient {
    fn observed_for(&self, function_id: &str, call: &Call) -> bool {
        self.is_observed_for(function_id, call)
    }

    fn run_observed(&self, run_id: &str) -> bool {
        self.observers(ON_RUN_COMPLETED_FN)
            .iter()
            .any(|obs| observer_run(obs.args_value()) == run_id)
    }

    async fn deliver(&self, request: CallRequest) {
        let _ = self.call(request).await;
    }
}

/// Call `workflow::on-run-completed` when any run reaches a terminal state,
/// fire-and-forget (`CallAction::Void`) and only while an observer wants this
/// run. For a per-run "notify me" callback, see `emit_notify` + `NotifySpec`.
pub async fn emit_run_completed(events: &dyn EventDeliverer, record: &WorkflowRunRecord) {
    // The match hook reads only `input.run_id`.
    let call = Call {
        function_id: ON_RUN_COMPLETED_FN.into(),
        input: serde_json::json!({ "run_id": record.run_id }),
        ..Call::default()
    };
    if let Some(caller) = stalled_caller(record).filter(|_| !events.run_observed(&record.run_id)) {
        // Also counts a run that ends before its caller's observe lands:
        // read it against the replay counter.
        tracing::warn!(
            run_id = %record.run_id,
            caller_session_id = caller,
            "run ended with no observer of its outcome and no notify"
        );
        crate::telemetry::record_run_unobserved();
    }
    if !events.observed_for(ON_RUN_COMPLETED_FN, &call) {
        return;
    }
    events
        .deliver(CallRequest {
            function_id: ON_RUN_COMPLETED_FN.into(),
            payload: completed_payload(record),
            action: Some(CallAction::Void),
            timeout_ms: None,
        })
        .await;
}

/// The agent caller a run that ended unobserved leaves waiting: a stamped
/// caller session, unless the run reports through `notify`. Pure.
fn stalled_caller(record: &WorkflowRunRecord) -> Option<&str> {
    record
        .caller_session_id
        .as_deref()
        .filter(|_| record.notify.is_none())
}

/// Fire the caller-supplied completion callback, if the run carries one. The
/// caller named a `function_id` at `workflow::start` time; we push it the run
/// outcome so the caller never has to poll `workflow::status`.
///
/// Durable delivery (enqueued, defaulting to the worker's `"workflow-notify"` queue) and
/// at-least-once: this fires inside `finalize`, before the terminal status is
/// persisted, so a crash mid-finalize re-ticks and re-fires. The handler must
/// dedup on `run_id`.
pub async fn emit_notify(deps: &Deps, record: &WorkflowRunRecord) {
    let Some(notify) = record.notify.as_ref() else {
        return;
    };
    if notify.function_id.trim().is_empty() {
        return;
    }
    let queue = notify
        .queue
        .clone()
        .unwrap_or_else(|| crate::queue::WORKFLOW_NOTIFY_QUEUE.to_string());
    if let Err(e) = deps
        .iii
        .call(iii_sdk::CallRequest {
            function_id: notify.function_id.clone(),
            payload: completed_payload(record),
            action: Some(iii_sdk::CallAction::Enqueue {
                queue: queue.clone(),
            }),
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(run_id = %record.run_id, %queue, error = %e, "notify enqueue failed");
    }
}

// ---------------------------------------------------------------------------
// Observers: one run, and the replay to a late one
// ---------------------------------------------------------------------------

/// The key of an arg-less observer: it wants every run.
const EVERY_RUN: &str = "*";

/// How long the added hook waits on the run's state read: inside the harness's
/// 5 s wait for an observer to become ready, so a refusal reaches the agent as
/// the observe's error.
const ADMIT_READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Retries of a failed replay read, on a doubling backoff from [`RETRY_BASE`].
/// A replay at boot can run before the state worker is up, and a terminal run
/// emits nothing more, so without them the outcome is lost for that observer.
const READ_RETRIES: u32 = 5;
const RETRY_BASE: Duration = Duration::from_secs(1);

/// The run an observer wants, from its args: `run_id`, or every run. Pure.
fn observer_run(args: &Value) -> &str {
    args.get("run_id")
        .and_then(Value::as_str)
        .unwrap_or(EVERY_RUN)
}

/// The observer keys a run end matches: every-run observers and those of its
/// own run. Pure.
fn call_runs(input: &Value) -> Vec<&str> {
    let mut keys = vec![EVERY_RUN];
    keys.extend(input.get("run_id").and_then(Value::as_str));
    keys
}

fn observer_keys(obs: &Observer) -> Keys<'_> {
    observer_run(obs.args_value()).into()
}

fn call_keys(call: &Call) -> Keys<'_> {
    call_runs(&call.input).into()
}

/// Register `workflow::on-run-completed` with its hooks, set first so they run
/// for every observer. Keys rather than a predicate: `is_observed_for` answers
/// only from a key index, and the emit guard needs a per-run answer.
pub fn register_run_completed(iii: &Arc<IIIClient>, deps: &Deps) {
    iii.on_observation_match_for(ON_RUN_COMPLETED_FN, Match::keys(observer_keys, call_keys));
    let d = deps.clone();
    let state_down = StateDown::default();
    iii.on_observer_added_for(ON_RUN_COMPLETED_FN, move |obs: Observer| {
        let (d, state_down) = (d.clone(), state_down.clone());
        async move { observer_added(d, &state_down, obs).await }
    });
    iii.register_function(
        ON_RUN_COMPLETED_FN,
        RegisterFunction::new(|event: RunCompletedEvent| Ok::<_, iii_sdk::errors::Error>(event))
            .description(ON_RUN_COMPLETED_DESC)
            .args_format::<RunCompletedArgs>()
            .private(),
    );
}

/// Added hook: reject bad args (a typo would otherwise make an every-run
/// observer), refuse an observer of an unknown run, and replay a run that
/// already ended once the observer is accepted.
async fn observer_added(
    deps: Deps,
    state_down: &StateDown,
    obs: Observer,
) -> Result<(), ObserveRejection> {
    let RunCompletedArgs { run_id } = obs.args()?;
    let Some(run_id) = run_id else {
        return Ok(());
    };
    let found = if state_down.skip_read() {
        None
    } else {
        let found = tokio::time::timeout(
            ADMIT_READ_TIMEOUT,
            crate::state::get_run(&deps.iii, &run_id),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|record| record.is_some());
        state_down.record(found.is_some());
        found
    };
    admit(&run_id, found)?;
    tokio::spawn(replay(deps, obs, run_id));
    Ok(())
}

/// After a failed or slow admit read, the reads of the next
/// [`ADMIT_READ_TIMEOUT`] are skipped and their observers admitted at once (the
/// replay reads again). Hooks of one function run one at a time, so at a
/// restart each re-listed observer would otherwise wait the full bound in
/// turn, and the later ones would pass the engine's 30 s owner timeout.
#[derive(Clone, Default)]
struct StateDown(Arc<std::sync::Mutex<Option<tokio::time::Instant>>>);

impl StateDown {
    fn skip_read(&self) -> bool {
        let until = *self.0.lock().unwrap_or_else(|e| e.into_inner());
        until.is_some_and(|until| tokio::time::Instant::now() < until)
    }

    fn record(&self, read_ok: bool) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) =
            (!read_ok).then(|| tokio::time::Instant::now() + ADMIT_READ_TIMEOUT);
    }
}

/// Refuse an observer of a run that does not exist; admit it when the read
/// failed or ran out (`None`), since the replay reads again. Pure.
fn admit(run_id: &str, found: Option<bool>) -> Result<(), ObserveRejection> {
    if found == Some(false) {
        return Err(ObserveRejection::new(
            "UNKNOWN_RUN",
            format!("no run {run_id}: unknown, or it ended more than 24 h ago and was collected"),
        ));
    }
    Ok(())
}

/// Send an accepted observer the event of its run if the run already ended.
async fn replay(deps: Deps, obs: Observer, run_id: String) {
    if obs.accepted().await.is_err() {
        return;
    }
    let read = || crate::state::get_run(&deps.iii, &run_id);
    let payload = match replay_event(&deps.locks, &run_id, read).await {
        Ok(Some(payload)) => payload,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%run_id, observer = obs.id(), %error, "run unreadable; no replay");
            return;
        }
    };
    // A call through an observer skips the engine's filter step.
    let call = Call {
        function_id: ON_RUN_COMPLETED_FN.into(),
        input: payload.clone(),
        output: Some(payload.clone()),
        ..Call::default()
    };
    if !obs.matches(&call) {
        return;
    }
    let request = CallRequest::observer(&obs, payload).action(CallAction::Void);
    match deps.iii.call(request).await {
        Ok(_) => {
            tracing::info!(%run_id, observer = obs.id(), "replayed the run's end");
            crate::telemetry::record_run_replayed();
        }
        Err(error) => tracing::warn!(%run_id, observer = obs.id(), %error, "replay failed"),
    }
}

/// The event a late observer of `run_id` is owed: the run's event once it is
/// terminal; `None` while it runs (the live emit then reaches the now-active
/// observer) or when it does not exist. A failed read is retried.
async fn replay_event<R, F>(
    locks: &WorkflowLocks,
    run_id: &str,
    read: R,
) -> Result<Option<Value>, WorkflowError>
where
    R: Fn() -> F,
    F: Future<Output = Result<Option<WorkflowRunRecord>, WorkflowError>>,
{
    let Some(mut record) = read_retrying(&read).await? else {
        return Ok(None);
    };
    if !record.status.is_terminal() {
        // tick holds this lock from its load through the emit to the persist,
        // so this read lands before the emit or after the persist.
        let _guard = locks.guard(run_id).await;
        match read_retrying(&read).await? {
            Some(again) if again.status.is_terminal() => record = again,
            _ => return Ok(None),
        }
    }
    Ok(Some(completed_payload(&record)))
}

async fn read_retrying<R, F>(read: &R) -> Result<Option<WorkflowRunRecord>, WorkflowError>
where
    R: Fn() -> F,
    F: Future<Output = Result<Option<WorkflowRunRecord>, WorkflowError>>,
{
    let mut attempt = 0;
    loop {
        match read().await {
            Err(error) if attempt < READ_RETRIES => {
                let backoff = RETRY_BASE * 2u32.pow(attempt);
                tracing::warn!(%error, ?backoff, "run read failed; retrying the replay");
                tokio::time::sleep(backoff).await;
                attempt += 1;
            }
            other => return other,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use super::*;

    /// Records every delivered call. Reports every call as observed unless
    /// `observed` is false.
    pub struct RecordingDeliverer {
        pub observed: bool,
        pub calls: Mutex<Vec<CallRequest>>,
    }

    impl RecordingDeliverer {
        pub fn new(observed: bool) -> Self {
            Self {
                observed,
                calls: Mutex::new(Vec::new()),
            }
        }

        pub fn calls(&self) -> Vec<CallRequest> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        fn observed_for(&self, _function_id: &str, _call: &Call) -> bool {
            self.observed
        }

        fn run_observed(&self, _run_id: &str) -> bool {
            self.observed
        }

        async fn deliver(&self, request: CallRequest) {
            self.calls.lock().unwrap().push(request);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use super::test_support::RecordingDeliverer;
    use super::*;
    use crate::types::NodeCheckpoint;
    use serde_json::json;

    #[tokio::test]
    async fn emit_skips_an_unobserved_function() {
        let events = RecordingDeliverer::new(false);
        emit_run_completed(&events, &record_with("completed", json!({"ok": true}))).await;
        assert!(events.calls().is_empty());
    }

    #[tokio::test]
    async fn emit_calls_the_event_function_with_void() {
        let events = RecordingDeliverer::new(true);
        let mut rec = record_with("failed", json!(null));
        rec.result_error = Some("node 'a': boom".into());
        emit_run_completed(&events, &rec).await;

        let calls = events.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, ON_RUN_COMPLETED_FN);
        assert!(matches!(calls[0].action, Some(CallAction::Void)));
        assert_eq!(calls[0].timeout_ms, None);
        let event: RunCompletedEvent =
            serde_json::from_value(calls[0].payload.clone()).expect("event shape");
        assert_eq!(
            event,
            RunCompletedEvent {
                run_id: "r1".into(),
                status: RunStatus::Failed,
                result: None,
                result_error: Some("node 'a': boom".into()),
                failed_nodes: vec![],
            }
        );
    }

    fn record_with(status: &str, result: serde_json::Value) -> WorkflowRunRecord {
        serde_json::from_value(json!({
            "run_id": "r1", "step": 0, "status": status, "def_ref": "r1",
            "input": {}, "result": result, "created_at": 0, "updated_at": 0
        }))
        .expect("minimal record")
    }

    fn checkpoint(state: NodeState, error: &str) -> NodeCheckpoint {
        NodeCheckpoint {
            state,
            result_error: Some(error.into()),
            ..Default::default()
        }
    }

    #[test]
    fn completed_payload_carries_the_outcome() {
        let rec = record_with("completed", json!({"ok": true}));
        let p = completed_payload(&rec);
        assert_eq!(p["run_id"], "r1");
        assert_eq!(p["status"], "completed");
        assert_eq!(p["result"], json!({"ok": true}));
        // result_error is absent on a clean completion → serialized as null.
        assert!(p["result_error"].is_null());
        assert!(p.get("failed_nodes").is_none(), "omitted when empty: {p}");
        // The payload is the event the event function returns.
        let event: RunCompletedEvent = serde_json::from_value(p).expect("event shape");
        assert_eq!(event.status, RunStatus::Completed);
    }

    #[test]
    fn failed_nodes_lists_only_the_failures_a_completed_run_tolerated() {
        let mut rec = record_with("completed", json!({"ok": true}));
        rec.nodes
            .insert("side".into(), checkpoint(NodeState::Failed, "boom"));
        rec.nodes.insert(
            "sibling".into(),
            checkpoint(NodeState::Cancelled, "cancelled"),
        );
        rec.nodes.insert(
            "main".into(),
            NodeCheckpoint {
                state: NodeState::Done,
                ..Default::default()
            },
        );
        assert_eq!(
            RunCompletedEvent::from(&rec).failed_nodes,
            ["node 'side': boom"]
        );
        assert_eq!(
            completed_payload(&rec)["failed_nodes"],
            json!(["node 'side': boom"])
        );
        for status in [RunStatus::Failed, RunStatus::Cancelled] {
            rec.status = status;
            assert!(
                RunCompletedEvent::from(&rec).failed_nodes.is_empty(),
                "{status:?}"
            );
        }
    }

    #[test]
    fn an_unobserved_end_stalls_only_a_stamped_caller_without_notify() {
        let mut rec = record_with("completed", json!(null));
        assert_eq!(stalled_caller(&rec), None, "a worker caller");
        rec.caller_session_id = Some("s1".into());
        assert_eq!(stalled_caller(&rec), Some("s1"));
        rec.notify = Some(crate::types::NotifySpec {
            function_id: "w::done".into(),
            queue: None,
        });
        assert_eq!(stalled_caller(&rec), None, "notify reports it");
    }

    #[test]
    fn an_argless_observer_wants_every_run_and_a_run_observer_only_its_own() {
        let wants = |args: Value, run: &str| {
            let input = json!({ "run_id": run });
            call_runs(&input).contains(&observer_run(&args))
        };
        assert!(wants(json!({}), "r1") && wants(json!({}), "r2"));
        assert!(wants(json!({ "run_id": "r1" }), "r1"));
        assert!(!wants(json!({ "run_id": "r1" }), "r2"));
    }

    #[test]
    fn observer_args_refuse_unknown_keys() {
        let args = |v: Value| serde_json::from_value::<RunCompletedArgs>(v);
        assert_eq!(args(json!({})).unwrap().run_id, None);
        assert_eq!(
            args(json!({ "run_id": "r1" })).unwrap().run_id.as_deref(),
            Some("r1")
        );
        assert!(args(json!({ "runid": "r1" })).is_err(), "a typo is refused");
    }

    #[test]
    fn admit_refuses_only_a_run_known_to_be_missing() {
        let refusal = admit("r_x", Some(false)).expect_err("a missing run is refused");
        assert_eq!(refusal.code, "UNKNOWN_RUN");
        assert!(refusal.message.contains("r_x"), "{refusal}");
        assert!(admit("r_x", Some(true)).is_ok());
        assert!(admit("r_x", None).is_ok(), "a failed or slow read admits");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_admit_read_skips_the_reads_of_the_next_bound() {
        let down = StateDown::default();
        assert!(!down.skip_read());
        down.record(false);
        assert!(down.skip_read(), "the next observers are admitted at once");
        tokio::time::advance(ADMIT_READ_TIMEOUT).await;
        assert!(!down.skip_read(), "read again once the bound passed");
        down.record(false);
        down.record(true);
        assert!(!down.skip_read(), "a good read clears it");
    }

    /// A state read of one run: `failures` reads fail first, then the slot.
    #[derive(Default)]
    struct FakeRuns {
        record: Mutex<Option<WorkflowRunRecord>>,
        failures: AtomicU32,
        reads: AtomicU32,
    }

    impl FakeRuns {
        fn with(record: Option<WorkflowRunRecord>, failures: u32) -> Arc<Self> {
            Arc::new(Self {
                record: Mutex::new(record),
                failures: AtomicU32::new(failures),
                reads: AtomicU32::new(0),
            })
        }

        fn read(
            self: &Arc<Self>,
        ) -> impl Fn() -> std::future::Ready<Result<Option<WorkflowRunRecord>, WorkflowError>>
        {
            let runs = self.clone();
            move || {
                runs.reads.fetch_add(1, Ordering::SeqCst);
                let failed = runs
                    .failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                std::future::ready(if failed {
                    Err(WorkflowError::State("state worker down".into()))
                } else {
                    Ok(runs.record.lock().unwrap().clone())
                })
            }
        }
    }

    #[tokio::test]
    async fn replay_gives_the_event_of_a_terminal_run_only() {
        let locks = WorkflowLocks::default();
        let none = FakeRuns::with(None, 0);
        assert_eq!(replay_event(&locks, "r1", none.read()).await.unwrap(), None);
        let running = FakeRuns::with(Some(record_with("running", json!(null))), 0);
        assert_eq!(
            replay_event(&locks, "r1", running.read()).await.unwrap(),
            None
        );
        let done = FakeRuns::with(Some(record_with("completed", json!({"ok": true}))), 0);
        let event = replay_event(&locks, "r1", done.read())
            .await
            .unwrap()
            .expect("the event");
        assert_eq!(
            event,
            completed_payload(&record_with("completed", json!({"ok": true})))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn replay_retries_a_failed_read() {
        let locks = WorkflowLocks::default();
        let done = || Some(record_with("cancelled", json!(null)));
        let flaky = FakeRuns::with(done(), 1);
        let event = replay_event(&locks, "r1", flaky.read()).await.unwrap();
        assert_eq!(event.expect("one event")["status"], "cancelled");
        assert_eq!(flaky.reads.load(Ordering::SeqCst), 2);

        let down = FakeRuns::with(done(), READ_RETRIES + 1);
        assert!(replay_event(&locks, "r1", down.read()).await.is_err());
        assert_eq!(down.reads.load(Ordering::SeqCst), READ_RETRIES + 1);
    }

    #[tokio::test]
    async fn replay_of_a_finalizing_run_waits_for_its_persist() {
        let locks = WorkflowLocks::default();
        let runs = FakeRuns::with(Some(record_with("running", json!(null))), 0);
        // The tick holds the run from its load through the emit to the persist.
        let tick = locks.guard("r1").await;
        let task = tokio::spawn({
            let (locks, read) = (locks.clone(), runs.read());
            async move { replay_event(&locks, "r1", read).await }
        });
        while runs.reads.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        *runs.record.lock().unwrap() = Some(record_with("completed", json!({"ok": true})));
        drop(tick);

        let event = task.await.unwrap().unwrap().expect("one event");
        assert_eq!(event["status"], "completed");
        assert_eq!(
            runs.reads.load(Ordering::SeqCst),
            2,
            "read again under the lock"
        );
    }
}
