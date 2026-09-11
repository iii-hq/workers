use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Child;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Running,
    Finished,
    Killed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobRecord {
    pub id: String,
    pub argv: Vec<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub struct JobHandle {
    pub record: JobRecord,
    pub child: Option<Child>,
    /// PID of the host child process, set ONLY for host-backed background jobs.
    /// The detached drain task takes `child` out of the handle (so the task can
    /// own the `Child` and `wait()` on it), which leaves `child: None` here even
    /// while the process is alive. `host_pid` is now only a DISCRIMINATOR — `Some`
    /// means "host bg job, terminate via the kill-signal channel" and `None` means
    /// "sandbox job, no local process". Termination never signals this pid
    /// directly (that risked SIGKILLing a reused pid after `wait()` reaped the
    /// child); see [`kill_signal_for`] and the drain task.
    pub host_pid: Option<u32>,
}

pub struct Jobs {
    pub map: Mutex<HashMap<String, Arc<Mutex<JobHandle>>>>,
}

impl Jobs {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }
}

pub static JOBS: Lazy<Jobs> = Lazy::new(Jobs::new);

/// Per-job kill-signal channels for live host background jobs, keyed by job id.
///
/// The detached drain task is the ONLY code that signals a host child, and it
/// does so while it still owns the un-reaped `Child` — so the signal can never
/// land on a pid that `wait()` already reaped and the OS reused. `shell::kill`
/// and the shutdown sweep REQUEST termination by notifying the job's channel
/// here; the drain task selects on it and kills the child's process group.
///
/// A `std::sync::Mutex` (not the tokio one) because every access is a short,
/// non-async map operation; it is never held across an `.await`. Registered when
/// a host bg job spawns and removed when its drain task finalizes.
static KILL_SIGNALS: Lazy<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

/// Register (or replace) a host bg job's kill-signal channel and return it for
/// the drain task to await. Called once at spawn, before the drain task starts.
pub fn register_kill_signal(id: &str) -> Arc<tokio::sync::Notify> {
    let notify = Arc::new(tokio::sync::Notify::new());
    KILL_SIGNALS
        .lock()
        .expect("kill-signal map poisoned")
        .insert(id.to_string(), notify.clone());
    notify
}

/// Look up a live host bg job's kill-signal channel. `shell::kill` and the
/// shutdown sweep call this and `notify_one()` the result to request termination.
/// Returns `None` once the drain task has finalized and unregistered (the job is
/// already terminal, so there is nothing to kill).
pub fn kill_signal_for(id: &str) -> Option<Arc<tokio::sync::Notify>> {
    KILL_SIGNALS
        .lock()
        .expect("kill-signal map poisoned")
        .get(id)
        .cloned()
}

/// Remove a job's kill-signal channel. Called by the drain task as it finalizes,
/// so the map does not grow without bound across the worker's lifetime.
pub fn unregister_kill_signal(id: &str) {
    KILL_SIGNALS
        .lock()
        .expect("kill-signal map poisoned")
        .remove(id);
}

/// Live count of background jobs in the `Running` state, maintained as a plain
/// atomic so the `shell.jobs.running` observable gauge can read it from a
/// synchronous OTel callback without taking any async lock (the `JOBS` map is
/// behind `tokio::sync::Mutex`, which cannot be locked inside a sync gauge
/// callback without risking a deadlock). Incremented exactly once when a job is
/// successfully reserved+inserted and decremented exactly once by
/// `RunningJobGuard::drop`, which the spawned finalize task holds — so any exit
/// path (normal completion, early return, or panic) still decrements.
static RUNNING_JOBS: AtomicUsize = AtomicUsize::new(0);

/// Snapshot of the live running-job count for the metrics gauge. Cheap,
/// lock-free, and safe to call from a synchronous OTel observe callback.
pub fn running_gauge_value() -> usize {
    RUNNING_JOBS.load(Ordering::Relaxed)
}

/// RAII decrement for the `RUNNING_JOBS` gauge. The exec_bg spawn path moves one
/// of these into its detached finalize task; when that task ends (for any
/// reason) the `Drop` fires and the gauge returns to its true value. This keeps
/// the gauge correct without instrumenting every scattered status-flip site
/// (the host wait arms, the sandbox trigger arms, shell::kill, the shutdown
/// sweep) — the single task that owns the job's lifetime owns the decrement.
#[derive(Debug)]
pub struct RunningJobGuard {
    _private: (),
}

impl Drop for RunningJobGuard {
    fn drop(&mut self) {
        RUNNING_JOBS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Test-only serialization gate. `kill_running_host_jobs()` sweeps the *entire*
/// shared `JOBS` singleton, so any test that runs the sweep would SIGKILL every
/// other concurrently-running host job's child (the test binary runs tests in
/// parallel on one runtime). Tests that either run the global sweep or keep a
/// real long-lived host `Running` job in the map hold this mutex so they never
/// interleave. Not compiled into the production binary.
#[cfg(test)]
pub static HOST_SWEEP_TEST_GUARD: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

/// Test-only serialization gate for the `RUNNING_JOBS` gauge. The gauge is a
/// process-global atomic that every `try_reserve_and_insert` mutates, so any
/// unit test asserting an exact gauge delta must not interleave with another
/// test's reserve. Every reserve-using unit test in this module holds this
/// mutex; not compiled into the production binary.
#[cfg(test)]
pub static GAUGE_TEST_GUARD: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Atomically count running jobs and insert a new one if under `max`. Holds
/// the map mutex across both operations so two concurrent callers cannot
/// each pass the count check and then both insert. A handle whose lock is
/// currently held by another task is conservatively counted as running —
/// it's mid-finalization and either still Running or about to be — so the
/// soft cap may briefly under-allow at the boundary, but never over-allows.
///
/// On success, returns the job id and a [`RunningJobGuard`] that owns this job's
/// contribution to the `shell.jobs.running` gauge: the gauge is incremented here
/// and decremented when the guard drops. The caller MUST move the guard into the
/// task that owns the job's lifetime so the running count tracks reality.
///
/// On rejection, returns the running count and the original handle so the
/// caller can reclaim the spawned child process and kill it. The gauge is not
/// touched on rejection.
pub async fn try_reserve_and_insert(
    handle: JobHandle,
    max: usize,
) -> Result<(String, RunningJobGuard), (usize, JobHandle)> {
    let mut guard = JOBS.map.lock().await;
    let mut running = 0usize;
    for h in guard.values() {
        match h.try_lock() {
            Ok(g) => {
                if g.record.status == JobStatus::Running {
                    running += 1;
                }
            }
            Err(_) => running += 1,
        }
    }
    if running >= max {
        return Err((running, handle));
    }
    let id = handle.record.id.clone();
    let boxed = Arc::new(Mutex::new(handle));
    guard.insert(id.clone(), boxed);
    // Increment under the map lock so the gauge increment is ordered with the
    // insert; the matching decrement is the returned guard's Drop.
    RUNNING_JOBS.fetch_add(1, Ordering::Relaxed);
    Ok((id, RunningJobGuard { _private: () }))
}

pub async fn get(id: &str) -> Option<Arc<Mutex<JobHandle>>> {
    JOBS.map.lock().await.get(id).cloned()
}

// Snapshot the map before awaiting per-job locks. Holding the map guard
// across `handle.lock().await` head-of-line-blocks every other job
// operation (insert, get) for the duration of the iteration.
async fn snapshot() -> Vec<(String, Arc<Mutex<JobHandle>>)> {
    let guard = JOBS.map.lock().await;
    guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

pub async fn remove_old(retention_secs: u64) {
    let now = now_ms();
    let threshold_ms = retention_secs.saturating_mul(1000);
    let handles = snapshot().await;
    let mut to_remove: Vec<String> = Vec::new();
    for (id, handle) in handles {
        let h = handle.lock().await;
        if let Some(fin) = h.record.finished_at_ms {
            if now.saturating_sub(fin) > threshold_ms {
                to_remove.push(id);
            }
        }
    }
    if !to_remove.is_empty() {
        let mut guard = JOBS.map.lock().await;
        for id in to_remove {
            guard.remove(&id);
        }
    }
}

pub async fn list_all() -> Vec<JobRecord> {
    let handles = snapshot().await;
    let mut out = Vec::with_capacity(handles.len());
    for (_, handle) in handles {
        out.push(handle.lock().await.record.clone());
    }
    out
}

/// Best-effort terminate every still-running host-backed job. Called on shutdown
/// so the worker does not leave orphaned OS processes behind.
///
/// A running host bg job's `Child` is owned by its detached drain task (taken out
/// of the handle at spawn), so we do NOT signal a pid directly here — that risked
/// SIGKILLing a reused pid after the drain task's `wait()` had reaped the child.
/// Instead we notify each running host job's kill-signal channel; the drain task,
/// which still owns the un-reaped `Child`, kills the child's process group (so
/// grandchildren die too) and finalizes. We then poll briefly so shutdown is
/// deterministic rather than racing process exit; `kill_on_drop(true)` on each
/// child is the backstop if a drain task does not finish within the window.
///
/// Sandbox-backed jobs (`host_pid: None`) own no local process and are skipped, as
/// are terminal jobs. Returns the number of jobs signalled.
pub async fn kill_running_host_jobs() -> usize {
    let handles = snapshot().await;
    let mut requested: Vec<String> = Vec::new();
    for (id, handle) in &handles {
        let h = handle.lock().await;
        let is_running_host = h.record.status == JobStatus::Running && h.host_pid.is_some();
        drop(h);
        if is_running_host {
            if let Some(notify) = kill_signal_for(id) {
                notify.notify_one();
                requested.push(id.clone());
                tracing::info!(job_id = %id, "requested shutdown kill of running host job");
            }
        }
    }
    if requested.is_empty() {
        return 0;
    }
    // Bounded grace for the notified drain tasks to group-kill and finalize.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let mut still_running = 0usize;
        for id in &requested {
            if let Some(h) = get(id).await {
                if h.lock().await.record.status == JobStatus::Running {
                    still_running += 1;
                }
            }
        }
        if still_running == 0 || std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    requested.len()
}

/// Used by integration tests to assert lifecycle invariants. The
/// production exec_bg path uses `try_reserve_and_insert` which does the
/// counting and insertion atomically.
#[allow(dead_code)]
pub async fn running_count() -> usize {
    let handles = snapshot().await;
    let mut n = 0;
    for (_, handle) in handles {
        if handle.lock().await.record.status == JobStatus::Running {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_insert_and_get() {
        let _gate = GAUGE_TEST_GUARD.lock().await;
        let id = format!("test-insert-{}", uuid::Uuid::new_v4());
        match try_reserve_and_insert(make_handle(&id, JobStatus::Running), usize::MAX).await {
            Ok(_) => {}
            Err(_) => panic!("usize::MAX cap must always succeed"),
        }
        let got = get(&id).await.expect("job exists");
        assert_eq!(got.lock().await.record.id, id);
        JOBS.map.lock().await.remove(&id);
    }

    /// The `shell.jobs.running` gauge reads `running_gauge_value()`, which a
    /// successful reserve increments and the returned `RunningJobGuard` Drop
    /// decrements. Assert the delta is exactly +1 while the guard is held and
    /// returns to baseline after it drops.
    ///
    /// The gauge is a process-global atomic that any concurrent test's reserve
    /// perturbs, so this holds `GAUGE_TEST_GUARD` to read a stable baseline.
    /// The rejected-reserve case is folded in here (rather than a second test)
    /// so a single critical section covers both the +1/-1 and the no-op paths
    /// without a second lock acquisition racing the first.
    #[tokio::test]
    async fn running_gauge_tracks_reserve_and_guard_drop() {
        let _gate = GAUGE_TEST_GUARD.lock().await;

        // The two reads that bracket `drop(guard)` are synchronous with no
        // intervening await, so the guard's `fetch_sub` is the only mutation
        // between them — the delta is deterministic regardless of any other
        // task's pending gauge activity (the gate keeps reserve-using tests out,
        // but cross-module finalize tasks may still settle nearby).
        let id = format!("gauge-guard-{}", uuid::Uuid::new_v4());
        let (got_id, guard) =
            match try_reserve_and_insert(make_handle(&id, JobStatus::Running), usize::MAX).await {
                Ok(pair) => pair,
                Err(_) => panic!("usize::MAX cap must always succeed"),
            };
        assert_eq!(got_id, id);
        let with_guard = running_gauge_value();
        drop(guard);
        let after_drop = running_gauge_value();
        assert_eq!(
            after_drop + 1,
            with_guard,
            "dropping the RunningJobGuard must decrement the running gauge by exactly one"
        );
        JOBS.map.lock().await.remove(&id);

        // A rejected reserve (cap exceeded) must NOT touch the gauge and yields
        // no guard to later decrement. Again bracket with synchronous reads.
        let rid = format!("gauge-reject-{}", uuid::Uuid::new_v4());
        let before_reject = running_gauge_value();
        match try_reserve_and_insert(make_handle(&rid, JobStatus::Running), 0).await {
            Ok(_) => panic!("cap=0 must reject"),
            Err((_, returned)) => assert_eq!(returned.record.id, rid),
        }
        assert_eq!(
            running_gauge_value(),
            before_reject,
            "a rejected reserve must leave the running gauge unchanged"
        );
    }

    fn make_handle(id: &str, status: JobStatus) -> JobHandle {
        JobHandle {
            record: JobRecord {
                id: id.into(),
                argv: vec!["x".into()],
                started_at_ms: now_ms(),
                finished_at_ms: if status == JobStatus::Running {
                    None
                } else {
                    Some(now_ms())
                },
                status,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
            child: None,
            host_pid: None,
        }
    }

    // The `JOBS` singleton is shared across all tests in the binary and
    // tokio runs them concurrently, so these tests use deterministic
    // assertions that don't depend on absolute counts.

    #[tokio::test]
    async fn try_reserve_with_zero_cap_rejects_and_returns_handle() {
        let id = format!("reserve-zero-{}", uuid::Uuid::new_v4());
        match try_reserve_and_insert(make_handle(&id, JobStatus::Running), 0).await {
            Ok(_) => panic!("cap=0 must reject"),
            Err((_, returned)) => {
                assert_eq!(returned.record.id, id);
            }
        }
        assert!(
            get(&id).await.is_none(),
            "rejected reservation must not insert into the map"
        );
    }

    /// The reaper drives eviction by calling `remove_old` on a timer. This
    /// asserts the prune path that the reaper relies on: a finished record
    /// whose `finished_at_ms` is older than the retention window is removed.
    /// The reaper wiring in `main()` is a thin sleep loop that calls this same
    /// function with the live retention; it is not unit-tested without a
    /// running engine, but the eviction it triggers is covered here.
    #[tokio::test]
    async fn remove_old_evicts_finished_job_past_retention() {
        let _gate = GAUGE_TEST_GUARD.lock().await;
        let id = format!("reaper-evict-{}", uuid::Uuid::new_v4());
        // Finished one hour ago.
        let stale = now_ms().saturating_sub(60 * 60 * 1000);
        let handle = JobHandle {
            record: JobRecord {
                finished_at_ms: Some(stale),
                ..make_handle(&id, JobStatus::Finished).record
            },
            child: None,
            host_pid: None,
        };
        try_reserve_and_insert(handle, usize::MAX)
            .await
            .ok()
            .expect("seed insert");
        assert!(get(&id).await.is_some(), "pre-prune sanity");

        // Retention of 1s: a job finished an hour ago is well past it.
        remove_old(1).await;
        assert!(
            get(&id).await.is_none(),
            "finished job older than retention must be evicted by the reaper prune path"
        );
    }

    #[tokio::test]
    async fn kill_running_host_jobs_skips_sandbox_and_terminal_jobs() {
        // Acquire GAUGE before SWEEP (consistent global lock order) — this test
        // reserves jobs, perturbing the running gauge other tests assert on.
        let _gauge_gate = GAUGE_TEST_GUARD.lock().await;
        // The sweep is global: hold the gate so we don't SIGKILL another test's
        // live host child while scanning the shared map.
        let _guard = HOST_SWEEP_TEST_GUARD.lock().await;

        // A sandbox-backed running job (host_pid: None) must NOT be marked Killed
        // by the host sweep — it owns no local process.
        let sb = format!("kill-sweep-sandbox-{}", uuid::Uuid::new_v4());
        try_reserve_and_insert(make_handle(&sb, JobStatus::Running), usize::MAX)
            .await
            .ok()
            .expect("seed sandbox job");
        // A terminal job must be left untouched.
        let done = format!("kill-sweep-done-{}", uuid::Uuid::new_v4());
        try_reserve_and_insert(make_handle(&done, JobStatus::Finished), usize::MAX)
            .await
            .ok()
            .expect("seed finished job");

        kill_running_host_jobs().await;

        let sb_status = get(&sb).await.unwrap().lock().await.record.status.clone();
        assert_eq!(
            sb_status,
            JobStatus::Running,
            "sandbox job (host_pid: None) must survive the host kill sweep"
        );
        let done_status = get(&done).await.unwrap().lock().await.record.status.clone();
        assert_eq!(done_status, JobStatus::Finished, "terminal job untouched");

        JOBS.map.lock().await.remove(&sb);
        JOBS.map.lock().await.remove(&done);
    }

    // The end-to-end "shutdown sweep terminates a real child" test now lives in
    // `functions::exec_bg` (`shutdown_sweep_terminates_real_host_job`), where it
    // can drive the real `spawn_host_job` drain task that owns the Child and
    // performs the process-group kill — the sweep only requests termination via
    // the kill-signal channel, so a faked handle here could not be killed.
}
