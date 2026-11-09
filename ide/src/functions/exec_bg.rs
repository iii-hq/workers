use std::sync::Arc;

use uuid::Uuid;

use crate::calls::{CallFwd, IiiCallFwd};
use crate::config::ShellConfig;
use crate::exec::error::ExecError;
use crate::exec::host::{build_command, parse_argv};
use crate::exec::policy::{build_overrides, scope_root_for_target, ExecOverrides};
use crate::exec::sandbox::{is_engine_timeout, SandboxExecResponse};
use crate::functions::types::{ExecBgRequest, ExecBgResponse};
use crate::jobs::{self, JobHandle, JobRecord, JobStatus};
use crate::target::Target;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

/// Grace for joining a host job's stdout/stderr drain tasks after the child has
/// exited. A process-group kill closes the pipes so the drains finish at once;
/// this only bounds the pathological case where the direct child exited but a
/// grandchild inherited and still holds the pipe — there the job finalizes
/// (freeing its slot) instead of wedging forever.
const DRAIN_GRACE_MS: u64 = if cfg!(test) { 2_000 } else { 10_000 };

/// Slack added to a sandbox job's in-VM `timeout_ms` to form the client-side RPC
/// deadline. The engine should answer within timeout_ms; the slack covers
/// transport + queueing before we declare it unresponsive and finalize the job.
const SANDBOX_RPC_SLACK_MS: u64 = 30_000;

/// How often a running host job is checked for having stopped.
const STOP_POLL: Duration = Duration::from_millis(if cfg!(test) { 10 } else { 1_000 });

/// How long a host job may stay stopped before it is killed. Nothing here
/// resumes it, so without this its `wait()` never returns and
/// `shell::on-job-finished` never fires. The grace leaves room for an operator's
/// SIGSTOP and SIGCONT.
const STOPPED_GRACE: Duration = Duration::from_millis(if cfg!(test) { 1_000 } else { 30_000 });

/// While a job runs, its record holds only this many of the newest bytes of each
/// stream. `shell::status` then stays small enough to reach an agent and to poll:
/// the harness replaces a result over 256 KiB, status included, with a marker, and
/// counts both the result's text and its details. There a control character costs 7
/// and 6 bytes of JSON, so two streams of this size reach about 213 KB at worst. The
/// first `max_output_bytes` replace it when the job ends.
const LIVE_OUTPUT_BYTES: usize = 8 * 1024;

fn resolve_job_id(job_id: Option<String>) -> Result<String, String> {
    match job_id {
        Some(id) if id.trim().is_empty() => Err("'job_id' must not be empty or whitespace".into()),
        Some(id) => Ok(id),
        None => Ok(format!("job-{}", Uuid::new_v4())),
    }
}

pub async fn handle(
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::IIIClient,
    req: ExecBgRequest,
) -> Result<ExecBgResponse, String> {
    // Field-level type errors (wrong-type `command`, non-string `args[i]`,
    // bad `target.kind`) come from the per-field deserializers in
    // `functions::types`; the SDK forwards them as the call's `Err` with
    // the actionable text the LLM needs to self-correct.
    // See `functions::exec` — `args.as_ref()` preserves the shell-words
    // tokenization contract when the caller omits `args`.
    let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    // Gate the per-call cwd/env up front, BEFORE branching on target. Same
    // rules as shell::exec (jail-confined cwd, deny-only dangerous-key env
    // gating). exec_bg returns its spawn-time failures as plain strings (its
    // documented contract), so we stringify the S-code into the message — the
    // agent still sees the code (e.g. "S215") and the self-correcting text.
    // `scope_root` only scopes the host working directory: drop it for a sandbox
    // target so the harness-injected session dir does not surface as a host cwd
    // override and trip the host-only rejection below (see `scope_root_for_target`).
    let scope_root =
        scope_root_for_target(&req.target, crate::fs::scope_anchor(req.fs_scope.as_ref()));
    let mut overrides = build_overrides(
        req.cwd.as_deref(),
        req.env.as_ref(),
        scope_root,
        crate::fs::scope_grants(req.fs_scope.as_ref()),
        crate::fs::scope_boundary(req.fs_scope.as_ref()),
        &cfg,
    )
    .map_err(|e| format!("{}: {}", e.code, e.message))?;
    // stdin needs no gating (opaque input bytes); host-only via is_empty().
    overrides.stdin = req.stdin;

    match req.target {
        Target::Host => spawn_host_job(req.session_id, req.job_id, cfg, argv, overrides).await,
        Target::Sandbox { sandbox_id } => {
            // cwd/env/stdin are host-only — the sandbox::exec payload does not
            // forward them. Reject loudly (S210 in the message) rather than
            // silently ignoring an override on a sandbox-targeted job.
            // `is_empty()` is false when ANY of cwd/env/stdin is set, so the
            // message must name all three or it sends `stdin` callers down the
            // wrong correction path.
            if !overrides.is_empty() {
                return Err(
                    "S210: cwd/env/stdin overrides are host-only; the sandbox exec protocol \
                     does not forward them. Drop cwd/env/stdin, or use target: host."
                        .to_string(),
                );
            }
            // Resolve+clamp timeout for the sandbox path. The host path ignores
            // the per-call timeout_ms and uses cfg.max_bg_timeout_ms instead
            // (zero leaves background host jobs unbounded).
            let resolved = cfg.resolve_timeout(req.timeout_ms);
            let fwd: Arc<dyn CallFwd> = Arc::new(IiiCallFwd::new(iii));
            spawn_sandbox_job(
                req.session_id,
                req.job_id,
                cfg,
                fwd,
                sandbox_id,
                argv,
                resolved,
            )
            .await
        }
    }
}

pub(crate) async fn spawn_host_job(
    owner_session_id: Option<String>,
    job_id: Option<String>,
    cfg: Arc<ShellConfig>,
    argv: Vec<String>,
    overrides: ExecOverrides,
) -> Result<ExecBgResponse, String> {
    let id = resolve_job_id(job_id)?;
    let mut cmd = build_command(&argv, &cfg, &overrides)?;
    let record = JobRecord {
        id: id.clone(),
        argv: argv.clone(),
        started_at_ms: jobs::now_ms(),
        finished_at_ms: None,
        status: JobStatus::Running,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    };
    // Reserve before spawning: a rejected duplicate must never execute the
    // command, even briefly. The registry lock covers check, spawn and insert.
    let (_, running_guard) = jobs::try_create(&id, cfg.max_concurrent_jobs, || {
        let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
        crate::exec::host::pump_stdin(&mut child, &overrides.stdin);
        let host_pid = child.id();
        Ok(JobHandle {
            record,
            finalized: false,
            child: Some(child),
            host_pid,
            owner_session_id,
        })
    })
    .await?;

    let id_clone = id.clone();
    let limit = cfg.max_output_bytes;
    // Host bg hard cap, separate from the foreground `max_timeout_ms`: a bg job
    // is how callers run long work (installs, builds, dev servers), so binding
    // it to the short foreground cap would kill legitimate jobs. `0` means
    // UNBOUNDED — run until exit or shell::kill. A positive value force-kills a
    // runaway job. Snapshotting at spawn is intentional: a later hot-reload does
    // not retroactively re-bound an already-running job; new jobs pick up the
    // new cap on their next spawn.
    let cap_ms = cfg.max_bg_timeout_ms;
    // Register the kill-signal channel BEFORE spawning the drain task so a
    // shell::kill (or shutdown sweep) arriving immediately can request
    // termination. The drain task below owns the un-reaped Child and is the ONLY
    // code that signals the process — so a kill request can never land on a pid
    // that wait() already reaped and the OS reused.
    let kill_notify = jobs::register_kill_signal(&id);
    tokio::spawn(async move {
        // Owns this job's contribution to the shell.jobs.running gauge: dropped
        // when this task ends (any path — completion, early return, or panic),
        // which is exactly when the job leaves the Running state.
        let _running_guard = running_guard;
        let handle = match jobs::get(&id_clone).await {
            Some(h) => h,
            None => {
                jobs::unregister_kill_signal(&id_clone);
                return;
            }
        };

        // Take the child and pipes together before starting the drains. The
        // finalize task owns the process without holding its handle lock.
        let mut child = handle.lock().await.child.take();
        let stdout_pipe = child.as_mut().and_then(|child| child.stdout.take());
        let stderr_pipe = child.as_mut().and_then(|child| child.stderr.take());

        // Drain stdout/stderr concurrently — sequential reads deadlock
        // once the child fills one pipe's ~64 KiB buffer before closing
        // the other. Each drain publishes into the record as output arrives, so
        // shell::status shows a running job's output so far.
        let stdout_task = stdout_pipe.map(|mut out| {
            let job = handle.clone();
            tokio::spawn(async move {
                drain_to_record(&mut out, limit, &job, |r| {
                    (&mut r.stdout, &mut r.stdout_truncated)
                })
                .await
            })
        });
        let stderr_task = stderr_pipe.map(|mut err| {
            let job = handle.clone();
            tokio::spawn(async move {
                drain_to_record(&mut err, limit, &job, |r| {
                    (&mut r.stderr, &mut r.stderr_truncated)
                })
                .await
            })
        });

        // Race the child's exit against (a) the hard cap, (b) an external kill
        // request (shell::kill / shutdown sweep, delivered via `kill_notify`) and
        // (c) the job staying stopped. On any trigger we kill the child's WHOLE
        // process group while we still own the un-reaped Child — so the pid/pgid
        // cannot have been reused and grandchildren die too — then loop back to
        // reap via wait(). The `if` guards make these arms fire at most once.
        let mut kill_reason: Option<String> = None;
        let mut killed_by_request = false;
        let exit_status = if let Some(mut ch) = child {
            let pid = ch.id();
            // cap_ms == 0 → unbounded: a future that never resolves, so the cap
            // arm never fires and only natural exit / shell::kill ends the job.
            let cap = async move {
                if cap_ms == 0 {
                    std::future::pending::<()>().await
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(cap_ms)).await
                }
            };
            tokio::pin!(cap);
            let stopped = stayed_stopped(pid);
            tokio::pin!(stopped);
            loop {
                tokio::select! {
                    r = ch.wait() => break r.ok(),
                    _ = &mut cap, if kill_reason.is_none() && !killed_by_request => {
                        kill_reason = Some(format!(
                            "background job exceeded the host hard cap of {cap_ms}ms and was killed"
                        ));
                        crate::exec::host::kill_process_group(pid);
                    }
                    _ = kill_notify.notified(), if kill_reason.is_none() && !killed_by_request => {
                        killed_by_request = true;
                        crate::exec::host::kill_process_group(pid);
                    }
                    signal = &mut stopped, if kill_reason.is_none() && !killed_by_request => {
                        kill_reason = Some(stopped_note(signal));
                        crate::exec::host::kill_process_group(pid);
                    }
                }
            }
        } else {
            None
        };

        // Join the drains under a bounded grace. After a group-kill the pipes
        // close so these complete at once; the grace only bites the pathological
        // case where the direct child exited naturally but a grandchild still
        // holds the pipe open — there we abort the reader (freeing the fd) and
        // report what we have as truncated, so the job finalizes and frees its
        // concurrency slot instead of wedging forever.
        let (stdout_kept, stderr_kept) =
            tokio::join!(join_drain(stdout_task), join_drain(stderr_task));

        // Single finalize critical section: status, exit_code, output and
        // finished_at_ms are published together, after the drains' last write. Only
        // shell::kill shows a terminal status sooner: it flips the status first, so
        // a poller can see `killed` with the live view until this runs.
        let mut h = handle.lock().await;
        // Finalize-once: an external killer (shell::kill / shutdown sweep) may
        // have already flipped status to Killed and stamped finished_at_ms. Never
        // downgrade a terminal status, and never clobber that kill-time timestamp.
        if h.record.status == JobStatus::Running {
            h.record.status = if kill_reason.is_some() || killed_by_request {
                JobStatus::Killed
            } else {
                match &exit_status {
                    Some(s) if s.success() => JobStatus::Finished,
                    _ => JobStatus::Failed,
                }
            };
        }
        h.record.exit_code = exit_status.and_then(|s| s.code());
        let record = &mut h.record;
        publish_kept(
            (&mut record.stdout, &mut record.stdout_truncated),
            stdout_kept,
        );
        publish_kept(
            (&mut record.stderr, &mut record.stderr_truncated),
            stderr_kept,
        );
        if let Some(note) = kill_reason {
            // Append the cause without discarding any stderr the child emitted
            // before it was killed.
            if !h.record.stderr.is_empty() {
                h.record.stderr.push('\n');
            }
            h.record.stderr.push_str(&note);
        }
        if h.record.finished_at_ms.is_none() {
            h.record.finished_at_ms = Some(jobs::now_ms());
        }
        h.finalized = true;
        let finished = h.record.clone();
        drop(h);
        jobs::unregister_kill_signal(&id_clone);
        // Announce AFTER the lock is released: a slow subscriber must never
        // hold up the job that woke it, or the next job's finalize.
        crate::job_events::fire(&finished).await;
    });

    Ok(ExecBgResponse { job_id: id, argv })
}

pub(crate) async fn spawn_sandbox_job(
    owner_session_id: Option<String>,
    job_id: Option<String>,
    cfg: Arc<ShellConfig>,
    fwd: Arc<dyn CallFwd>,
    sandbox_id: Uuid,
    argv: Vec<String>,
    timeout_ms: u64,
) -> Result<ExecBgResponse, String> {
    let id = resolve_job_id(job_id)?;
    let record = JobRecord {
        id: id.clone(),
        argv: argv.clone(),
        started_at_ms: jobs::now_ms(),
        finished_at_ms: None,
        status: JobStatus::Running,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    };
    // For sandbox jobs, child is None — there is no local OS process.
    // The concurrency cap (max_concurrent_jobs) covers both backends
    // uniformly via the running-status count in try_reserve_and_insert.
    // On rejection, there is no orphan process to kill.
    let (_, running_guard) = jobs::try_reserve_and_insert(
        JobHandle {
            record,
            finalized: false,
            child: None,
            host_pid: None,
            owner_session_id,
        },
        cfg.max_concurrent_jobs,
    )
    .await
    .map_err(|(error, _)| error)?;

    let id_clone = id.clone();
    let argv_for_payload = argv.clone();
    tokio::spawn(async move {
        // Owns this job's contribution to the shell.jobs.running gauge (see the
        // host path); dropped when this task ends, i.e. when the job finalizes.
        let _running_guard = running_guard;
        let cmd = argv_for_payload[0].clone();
        let args: Vec<String> = argv_for_payload.iter().skip(1).cloned().collect();
        let payload = serde_json::json!({
            "sandbox_id": sandbox_id.to_string(),
            "cmd": cmd,
            "args": args,
            "timeout_ms": timeout_ms,
        });
        // Bound the engine RPC with a client-side deadline. Without one, a hung or
        // partitioned engine would park this task forever, permanently holding the
        // job's concurrency slot and keeping the shell.jobs.running gauge inflated.
        // The in-VM work is bounded by the payload's timeout_ms; we allow that plus
        // transport/queueing slack before declaring the engine unresponsive.
        let rpc_timeout =
            std::time::Duration::from_millis(timeout_ms.saturating_add(SANDBOX_RPC_SLACK_MS));
        let res = match tokio::time::timeout(rpc_timeout, fwd.call("sandbox::exec", payload)).await
        {
            Ok(r) => r,
            Err(_) => {
                // RPC deadline blown: finalize the job (unless an external kill
                // already did), then publish its completion just like a
                // normal response before releasing the slot/gauge.
                if let Some(handle) = jobs::get(&id_clone).await {
                    let mut h = handle.lock().await;
                    if h.record.status == JobStatus::Running {
                        h.record.status = JobStatus::Failed;
                        h.record.stderr = format!(
                            "sandbox::exec RPC timed out after {}ms (engine unresponsive)",
                            rpc_timeout.as_millis()
                        );
                        if h.record.finished_at_ms.is_none() {
                            h.record.finished_at_ms = Some(jobs::now_ms());
                        }
                    }
                    h.finalized = true;
                    let finished = h.record.clone();
                    drop(h);
                    crate::job_events::fire(&finished).await;
                }
                return;
            }
        };

        let handle = match jobs::get(&id_clone).await {
            Some(h) => h,
            None => return,
        };
        let mut h = handle.lock().await;

        // If shell::kill marked this Killed before the call returned, do not
        // overwrite the status — but capture stdout/stderr for completeness.
        let already_killed = h.record.status == JobStatus::Killed;

        match res {
            Ok(value) => {
                let parsed: Result<SandboxExecResponse, _> = serde_json::from_value(value);
                match parsed {
                    Ok(p) => {
                        h.record.stdout = p.stdout;
                        h.record.stderr = p.stderr;
                        h.record.stdout_truncated = p.stdout_truncated;
                        h.record.stderr_truncated = p.stderr_truncated;
                        h.record.exit_code = p.exit_code;
                        if !already_killed {
                            h.record.status = if p.timed_out {
                                JobStatus::Killed
                            } else if p.exit_code == Some(0) {
                                JobStatus::Finished
                            } else {
                                JobStatus::Failed
                            };
                        }
                    }
                    Err(e) => {
                        if !already_killed {
                            h.record.status = JobStatus::Failed;
                        }
                        h.record.stderr = format!("bad engine response: {e}");
                    }
                }
            }
            Err(err) => {
                if !already_killed {
                    // The adapter reports an in-VM timeout as an S200 error,
                    // not `timed_out: true`; it is the same kill.
                    let mapped = crate::scode::map_iii_err(&err, ExecError::new);
                    h.record.status = if is_engine_timeout(&mapped) {
                        JobStatus::Killed
                    } else {
                        JobStatus::Failed
                    };
                }
                h.record.stderr = format!("engine error: {err:?}");
            }
        }
        // Same guard as the status writes: if shell::kill already
        // recorded a finished_at_ms when it flipped status to Killed,
        // a late call response must NOT clobber that timestamp —
        // pollers reading finished_at_ms expect the kill-time, not
        // the in-VM process's eventual completion time.
        if !already_killed {
            h.record.finished_at_ms = Some(jobs::now_ms());
        }
        h.finalized = true;
        let finished = h.record.clone();
        drop(h);
        crate::job_events::fire(&finished).await;
    });

    Ok(ExecBgResponse { job_id: id, argv })
}

/// Resolves with the stop signal once `pid` has stayed stopped for
/// [`STOPPED_GRACE`]. Only polled while the caller owns the un-reaped child.
// shortcut: watches the direct child only, so a stopped grandchild that the
// child waits on is not seen; walk the process tree if that shows up in practice.
async fn stayed_stopped(pid: Option<u32>) -> i32 {
    let mut stopped_since = None;
    loop {
        tokio::time::sleep(STOP_POLL).await;
        match pid.and_then(crate::exec::host::stop_signal) {
            Some(signal)
                if stopped_since.get_or_insert_with(Instant::now).elapsed() >= STOPPED_GRACE =>
            {
                return signal;
            }
            Some(_) => {}
            None => stopped_since = None,
        }
    }
}

/// What a job's stderr says when it was killed for staying stopped.
fn stopped_note(signal: i32) -> String {
    format!(
        "background job was stopped by signal {signal} and stayed stopped for {}s, so it was \
         killed: nothing would have resumed it",
        STOPPED_GRACE.as_secs()
    )
}

/// What a stream leaves for its job's end: the kept text, and whether it was cut.
type KeptOutput = (String, bool);

/// Join one drain task under [`DRAIN_GRACE_MS`]; `None` when it had to be
/// aborted. On grace expiry (a grandchild is holding the pipe open after the
/// child exited) the reader task is aborted to release its file descriptor, so
/// the owning job can still finalize with the output published so far.
async fn join_drain(task: Option<tokio::task::JoinHandle<KeptOutput>>) -> Option<KeptOutput> {
    let Some(mut task) = task else {
        return Some(KeptOutput::default());
    };
    let grace = Duration::from_millis(DRAIN_GRACE_MS);
    match tokio::time::timeout(grace, &mut task).await {
        Ok(joined) => joined.ok(),
        Err(_) => {
            task.abort();
            // Wait it out: an abort lands at the next await, and a write after this
            // point would overwrite what finalize adds to the record.
            let _ = task.await;
            None
        }
    }
}

/// A stream's final text. Without kept output, what the record shows so far stands,
/// flagged as cut.
fn publish_kept((text, flag): (&mut String, &mut bool), kept: Option<KeptOutput>) {
    match kept {
        Some((kept, truncated)) => {
            *text = kept;
            *flag = truncated;
        }
        None => *flag = true,
    }
}

/// The newest [`LIVE_OUTPUT_BYTES`] of `bytes`, starting on a character.
fn live_tail(bytes: &[u8]) -> &[u8] {
    let mut start = bytes.len().saturating_sub(LIVE_OUTPUT_BYTES);
    while start > 0 && start < bytes.len() && bytes[start] & 0xC0 == 0x80 {
        start += 1;
    }
    &bytes[start..]
}

/// Read one stream to its end and return its first `limit` bytes. The job's record
/// holds the newest bytes as they arrive, past `limit` too so a long job's view
/// keeps moving. The kept bytes wait for the job's end because a stream can close
/// long before it, and a running job's status must stay small. `field` picks the
/// stream's text and flag.
async fn drain_to_record<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    limit: usize,
    job: &Mutex<JobHandle>,
    field: fn(&mut JobRecord) -> (&mut String, &mut bool),
) -> KeptOutput {
    let mut buf = Vec::new();
    let mut newest = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let take = n.min(limit.saturating_sub(buf.len()));
                buf.extend_from_slice(&chunk[..take]);
                truncated |= take < n;
                newest.extend_from_slice(&chunk[..n]);
                if newest.len() > 2 * LIVE_OUTPUT_BYTES {
                    newest.drain(..newest.len() - LIVE_OUTPUT_BYTES);
                }
                let live = live_tail(&newest);
                let mut h = job.lock().await;
                let (text, flag) = field(&mut h.record);
                *text = String::from_utf8_lossy(live).into_owned();
                *flag = truncated || live.len() < buf.len();
            }
        }
    }
    (String::from_utf8_lossy(&buf).into_owned(), truncated)
}

#[cfg(test)]
mod host_path_tests {
    use super::spawn_host_job;
    use crate::config::ShellConfig;
    use crate::exec::policy::ExecOverrides;
    use crate::jobs::{self, JobStatus};
    use std::sync::Arc;
    use std::time::Duration;

    // The first arg sets the HOST BG hard cap (`max_bg_timeout_ms`) for these
    // background-job tests; `max_timeout_ms` is set to match for good measure.
    // (0 → unbounded bg job.)
    fn cfg(bg_cap_ms: u64, max_concurrent_jobs: usize) -> Arc<ShellConfig> {
        let mut c = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 4096,
            max_timeout_ms: bg_cap_ms,
            max_bg_timeout_ms: bg_cap_ms,
            max_concurrent_jobs,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        Arc::new(c)
    }

    /// Poll a job until it is fully finalized or the deadline expires, then return
    /// its status. Waiting on `finalized` rather than a non-Running status or
    /// `finished_at_ms` avoids racing a kill, which sets both before the trailing
    /// stdout/stderr writes. Tighter than a fixed sleep:
    /// returns the instant the task finalizes, so the test stays fast.
    async fn await_terminal(job_id: &str, deadline_ms: u64) -> JobStatus {
        let start = std::time::Instant::now();
        loop {
            if let Some(h) = jobs::get(job_id).await {
                let r = h.lock().await;
                if r.finalized {
                    return r.record.status.clone();
                }
            }
            if start.elapsed() >= Duration::from_millis(deadline_ms) {
                // Deadline hit without finalization — the job is still Running
                // (a genuine hang), which is what we report.
                return JobStatus::Running;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn host_bg_job_exceeding_hard_cap_is_killed_and_marked_killed() {
        // Acquire GAUGE before SWEEP (consistent global lock order): a successful
        // reserve here perturbs the running gauge that jobs.rs tests assert on,
        // and the gate is held through await_terminal so the guard's decrement
        // (which fires when the finalize task ends) lands inside the critical
        // section rather than racing another test's gauge assertion.
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        // Serialize with the global kill-sweep test: it would otherwise SIGKILL
        // this job's `sleep 5` child by pid before the 200ms cap fires, which
        // would finalize the job as (externally) Killed with no "hard cap" note.
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;

        // Hard cap of 200ms vs a 5s sleep: the detached wait must time out,
        // kill the child, and finalize the record as Killed (the mapping the
        // sandbox path uses for timed_out).
        let cfg = cfg(200, 16);
        let resp = spawn_host_job(
            None,
            None,
            cfg,
            vec!["sleep".into(), "5".into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");

        // Generous deadline (2s) so we never flake on a loaded CI box, but the
        // poll returns the instant the 200ms cap fires.
        let status = await_terminal(&resp.job_id, 2000).await;
        assert_eq!(
            status,
            JobStatus::Killed,
            "a host bg job past the hard cap must be killed"
        );

        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert!(
            r.record.stderr.contains("hard cap"),
            "timed-out job records the cause in stderr: {:?}",
            r.record.stderr
        );
        assert!(r.record.finished_at_ms.is_some(), "finalized");
        assert!(r.child.is_none(), "child reaped/taken after finalization");

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn host_bg_short_job_finishes_within_cap() {
        // Hold the gauge gate through finalization for the same reason as the
        // hard-cap test: a successful reserve perturbs the process-global gauge.
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        // Control: a fast command finishes normally well within the cap.
        let cfg = cfg(5_000, 16);
        let resp = spawn_host_job(
            None,
            None,
            cfg,
            vec!["true".into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let status = await_terminal(&resp.job_id, 2000).await;
        assert_eq!(status, JobStatus::Finished);
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        assert_eq!(h.lock().await.record.exit_code, Some(0));
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn host_bg_rejected_at_cap_returns_rejection_error() {
        // The cap check must win before trying to spawn even an invalid
        // executable. Rejection cannot leave a child or side effects behind.
        let cfg = cfg(30_000, 0);
        let err = spawn_host_job(
            None,
            None,
            cfg,
            vec!["/no-such-executable-for-cap-rejection".into()],
            ExecOverrides::default(),
        )
        .await
        .expect_err("cap=0 must reject before spawning");
        assert!(
            err.contains("max concurrent jobs"),
            "rejection message: {err}"
        );
    }

    /// `kill(pid, 0)` probes for a live process without sending a signal.
    /// Once the finalize task reaps the child, this returns false.
    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        // SAFETY: signal 0 performs error checking only; it never delivers a
        // signal or touches process memory.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    /// The body of [`a_job_cannot_stop_on_the_workers_controlling_terminal`]:
    /// it runs in a re-executed copy of this test binary, whose controlling
    /// terminal is a pty, as a worker started from a terminal has.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "run under a pty by a_job_cannot_stop_on_the_workers_controlling_terminal"]
    async fn job_under_a_controlling_terminal() {
        assert!(
            std::fs::File::open("/dev/tty").is_ok(),
            "this copy must own a controlling terminal, or it proves nothing"
        );
        let resp = spawn_host_job(
            None,
            None,
            cfg(0, 16),
            vec!["sh".into(), "-c".into(), "cat /dev/tty".into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let status = await_terminal(&resp.job_id, 5_000).await;
        assert_eq!(
            status,
            JobStatus::Failed,
            "a job reading the terminal fails"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_job_cannot_stop_on_the_workers_controlling_terminal() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .expect("openpty");
        let mut cmd =
            portable_pty::CommandBuilder::new(std::env::current_exe().expect("test binary"));
        cmd.args([
            "--exact",
            "functions::exec_bg::host_path_tests::job_under_a_controlling_terminal",
            "--ignored",
        ]);
        let mut child = pair.slave.spawn_command(cmd).expect("spawn under the pty");
        drop(pair.slave);
        let mut output = pair.master.try_clone_reader().expect("pty reader");
        let reader = std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = std::io::Read::read_to_end(&mut output, &mut text);
            String::from_utf8_lossy(&text).into_owned()
        });
        let status = tokio::task::spawn_blocking(move || child.wait())
            .await
            .expect("join")
            .expect("wait");
        drop(pair.master);
        let text = reader.join().expect("reader");
        assert!(status.success(), "under the pty:\n{text}");
        // A renamed inner test would select nothing and still exit 0.
        assert!(text.contains("1 passed"), "the inner test ran:\n{text}");
    }

    /// A stopped job looks running to `wait()`. Nothing would resume it, so it
    /// ends as killed, says why, and frees its observer.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_job_that_stays_stopped_is_killed_and_says_why() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let resp = spawn_host_job(
            None,
            None,
            cfg(0, 16),
            vec![
                "sh".into(),
                "-c".into(),
                "echo before; kill -STOP $$".into(),
            ],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        assert_eq!(await_terminal(&resp.job_id, 5_000).await, JobStatus::Killed);
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.stdout, "before\n");
        assert!(
            r.record.stderr.contains("stayed stopped"),
            "{}",
            r.record.stderr
        );
        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// A job stopped for less than the grace is an operator's pause, not a hang,
    /// however many pauses add up past it: each resume restarts the grace.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_job_resumed_within_the_grace_keeps_running() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        // The resume waits to see the stop, so it cannot land first and strand the job.
        let pause = r#"sh -c 'until ps -o state= -p "$1" | grep -q T; do sleep 0.01; done; sleep 0.2; kill -CONT "$1"' _ $$ & kill -STOP $$;"#;
        // Six pauses add up past the grace, and the sleeps outlast a stop check, so
        // each resume is seen.
        let script = format!("{}echo done", format!("{pause} sleep 0.1; ").repeat(6));
        let resp = spawn_host_job(
            None,
            None,
            cfg(0, 16),
            vec!["sh".into(), "-c".into(), script],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        assert_eq!(
            await_terminal(&resp.job_id, 10_000).await,
            JobStatus::Finished
        );
        let handle = jobs::get(&resp.job_id).await.expect("job exists");
        assert_eq!(handle.lock().await.record.stdout, "done\n");
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// A stream can close long before its job ends, and until then the status must
    /// stay small enough to reach an agent.
    #[tokio::test]
    async fn a_closed_stream_keeps_the_live_view_until_the_job_ends() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let mut roomy = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 1024 * 1024,
            max_concurrent_jobs: 16,
            ..Default::default()
        };
        roomy.compile_denylist().unwrap();
        let script =
            "head -c 100000 /dev/zero | tr '\\0' x; echo; echo newest; exec 1>&-; sleep 30";
        let resp = spawn_host_job(
            None,
            None,
            Arc::new(roomy),
            vec!["sh".into(), "-c".into(), script.into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let handle = jobs::get(&resp.job_id).await.expect("job exists");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !handle.lock().await.record.stdout.ends_with("newest\n") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the newest output reaches the record");
        for _ in 0..20 {
            assert!(handle.lock().await.record.stdout.len() <= super::LIVE_OUTPUT_BYTES);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(jobs::kill_running_host_jobs().await >= 1);
        assert_eq!(await_terminal(&resp.job_id, 5_000).await, JobStatus::Killed);
        assert!(handle.lock().await.record.stdout.len() > 100_000);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// A grandchild can hold a pipe past its job's end: the job still ends, showing
    /// what it showed so far and saying it is partial.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_grandchild_holding_the_pipe_leaves_the_live_view() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let resp = spawn_host_job(
            None,
            None,
            cfg(0, 16),
            vec!["sh".into(), "-c".into(), "echo out; sleep 8 &".into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        assert_eq!(
            await_terminal(&resp.job_id, 7_000).await,
            JobStatus::Finished
        );
        let handle = jobs::get(&resp.job_id).await.expect("job exists");
        {
            let h = handle.lock().await;
            assert_eq!(h.record.stdout, "out\n");
            assert!(h.record.stdout_truncated);
        }
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn a_running_job_shows_its_output_so_far() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let resp = spawn_host_job(
            None,
            None,
            cfg(0, 16),
            vec![
                "sh".into(),
                "-c".into(),
                "echo out; echo err >&2; sleep 30".into(),
            ],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let handle = jobs::get(&resp.job_id).await.expect("job exists");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                {
                    let h = handle.lock().await;
                    assert_eq!(h.record.status, JobStatus::Running);
                    if h.record.stdout == "out\n" && h.record.stderr == "err\n" {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the output of a running job reaches its record");
        assert!(jobs::kill_running_host_jobs().await >= 1);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// A chatty job's status must stay small while it runs: the harness replaces
    /// an oversized result with a marker, which would hide `status` itself.
    #[tokio::test]
    async fn a_running_job_shows_its_newest_output_then_all_of_it() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let mut roomy = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 1024 * 1024,
            max_concurrent_jobs: 16,
            ..Default::default()
        };
        roomy.compile_denylist().unwrap();
        let script = "head -c 100000 /dev/zero | tr '\\0' x; echo; echo newest; sleep 30";
        let resp = spawn_host_job(
            None,
            None,
            Arc::new(roomy),
            vec!["sh".into(), "-c".into(), script.into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let handle = jobs::get(&resp.job_id).await.expect("job exists");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !handle.lock().await.record.stdout.ends_with("newest\n") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the newest output reaches the record");
        {
            let h = handle.lock().await;
            assert_eq!(h.record.status, JobStatus::Running);
            assert!(h.record.stdout.len() <= super::LIVE_OUTPUT_BYTES);
            assert!(h.record.stdout_truncated, "a partial view says so");
        }
        assert!(jobs::kill_running_host_jobs().await >= 1);
        assert_eq!(await_terminal(&resp.job_id, 5_000).await, JobStatus::Killed);
        let h = handle.lock().await;
        assert!(h.record.stdout.len() > 100_000, "all of it once it ended");
        assert!(!h.record.stdout_truncated);
        drop(h);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// A long job's view must keep moving once the kept output is full.
    #[tokio::test]
    async fn the_live_view_keeps_moving_past_the_limit() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let mut small = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 4,
            max_concurrent_jobs: 16,
            ..Default::default()
        };
        small.compile_denylist().unwrap();
        let resp = spawn_host_job(
            None,
            None,
            Arc::new(small),
            vec![
                "sh".into(),
                "-c".into(),
                "echo first; sleep 0.2; echo newest; sleep 30".into(),
            ],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let handle = jobs::get(&resp.job_id).await.expect("job exists");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !handle.lock().await.record.stdout.contains("newest") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("output past the limit reaches the live view");
        assert!(handle.lock().await.record.stdout_truncated);
        assert!(jobs::kill_running_host_jobs().await >= 1);
        assert_eq!(await_terminal(&resp.job_id, 5_000).await, JobStatus::Killed);
        assert_eq!(handle.lock().await.record.stdout, "firs", "the kept output");
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[test]
    fn the_live_view_starts_on_a_character() {
        let text = "é".repeat(super::LIVE_OUTPUT_BYTES);
        let tail = super::live_tail(text.as_bytes());
        assert!(tail.len() <= super::LIVE_OUTPUT_BYTES);
        assert!(std::str::from_utf8(tail).is_ok());
        assert_eq!(super::live_tail(b"short"), b"short");
    }

    #[tokio::test]
    async fn output_past_the_limit_is_cut_and_flagged() {
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;
        let mut small = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 4,
            max_concurrent_jobs: 16,
            ..Default::default()
        };
        small.compile_denylist().unwrap();
        let resp = spawn_host_job(
            None,
            None,
            Arc::new(small),
            vec!["sh".into(), "-c".into(), "echo 0123456789".into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        assert_eq!(
            await_terminal(&resp.job_id, 5_000).await,
            JobStatus::Finished
        );
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.stdout, "0123");
        assert!(r.record.stdout_truncated);
        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// End-to-end: the shutdown sweep terminates a real running host child.
    /// Drives the real `spawn_host_job` so a real drain task listens on the
    /// kill-signal channel; `kill_running_host_jobs` notifies it and the drain
    /// task group-kills the live process. (Relocated from jobs.rs, which could
    /// only fake the handle — the sweep no longer signals a bare pid, so a faked
    /// job with no registered drain task can't be killed.)
    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_sweep_terminates_real_host_job() {
        use std::time::{Duration, Instant};
        let _gauge_gate = jobs::GAUGE_TEST_GUARD.lock().await;
        let _guard = jobs::HOST_SWEEP_TEST_GUARD.lock().await;

        let resp = spawn_host_job(
            None,
            None,
            cfg(60_000, 16),
            vec!["sleep".into(), "30".into()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn_host_job");
        let pid = jobs::get(&resp.job_id)
            .await
            .expect("job exists")
            .lock()
            .await
            .host_pid
            .expect("host bg job has a pid");
        assert!(pid_is_alive(pid), "child alive before sweep");

        let signalled = jobs::kill_running_host_jobs().await;
        assert!(signalled >= 1, "sweep must signal the running host job");

        // The sweep's bounded grace waits for finalization, so the child should
        // already be dead; assert it within a tight margin and check the status.
        let start = Instant::now();
        loop {
            if !pid_is_alive(pid) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "swept host child (pid {pid}) must die within 3s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let status = jobs::get(&resp.job_id)
            .await
            .unwrap()
            .lock()
            .await
            .record
            .status
            .clone();
        assert_eq!(status, JobStatus::Killed, "swept job is marked Killed");

        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }
}

#[cfg(test)]
mod sandbox_path_tests {
    use crate::calls::CallFwd;
    use crate::config::ShellConfig;
    use crate::jobs::{self, JobStatus};
    use async_trait::async_trait;
    use iii_sdk::errors::Error;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    struct ImmediateOk(Mutex<Option<Value>>);

    #[async_trait]
    impl CallFwd for ImmediateOk {
        async fn call(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
            Ok(self.0.lock().unwrap().take().unwrap())
        }
    }

    fn cfg_open() -> Arc<ShellConfig> {
        let mut c = ShellConfig {
            env: crate::config::EnvConfig::inherit_all(),
            max_output_bytes: 4096,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        Arc::new(c)
    }

    #[tokio::test]
    async fn sandbox_job_transitions_to_finished() {
        let cfg = cfg_open();
        let fwd: Arc<dyn CallFwd> = Arc::new(ImmediateOk(Mutex::new(Some(json!({
            "stdout": "ok\n",
            "stderr": "",
            "exit_code": 0,
            "duration_ms": 5,
            "timed_out": false,
        })))));
        let sandbox_id = Uuid::new_v4();

        let resp = handle_sandbox_for_test(
            cfg.clone(),
            fwd.clone(),
            sandbox_id,
            vec!["echo".into(), "ok".into()],
            5000,
        )
        .await
        .unwrap();

        // Wait for the background task to finalize. 200ms is generous;
        // the stub responds immediately.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.status, JobStatus::Finished);
        assert_eq!(r.record.exit_code, Some(0));
        assert_eq!(r.record.stdout, "ok\n");
        assert!(r.child.is_none(), "sandbox jobs do not own a child");
        assert_eq!(r.record.argv, vec!["echo", "ok"]);

        // Cleanup to avoid polluting other tests via the global JOBS map.
        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// The job keeps the adapter's truncation flags.
    #[tokio::test]
    async fn sandbox_job_keeps_the_truncation_flags() {
        let fwd: Arc<dyn CallFwd> = Arc::new(ImmediateOk(Mutex::new(Some(json!({
            "stdout": "x",
            "stderr": "y",
            "exit_code": 0,
            "duration_ms": 5,
            "timed_out": false,
            "stdout_truncated": true,
            "stderr_truncated": true,
        })))));
        let resp =
            handle_sandbox_for_test(cfg_open(), fwd, Uuid::new_v4(), vec!["yes".into()], 5000)
                .await
                .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert!(r.record.stdout_truncated && r.record.stderr_truncated);

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn sandbox_job_null_exit_code_marks_failed() {
        // Signal death: the router reports `exit_code: null`. The job must be
        // Failed with no exit code, and the output captured so far kept.
        let cfg = cfg_open();
        let fwd: Arc<dyn CallFwd> = Arc::new(ImmediateOk(Mutex::new(Some(json!({
            "stdout": "partial\n",
            "stderr": "",
            "exit_code": null,
            "duration_ms": 5,
            "timed_out": false,
        })))));
        let resp =
            handle_sandbox_for_test(cfg.clone(), fwd, Uuid::new_v4(), vec!["sh".into()], 5000)
                .await
                .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.status, JobStatus::Failed);
        assert_eq!(r.record.exit_code, None);
        assert_eq!(r.record.stdout, "partial\n");

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn sandbox_job_timed_out_marks_killed() {
        let cfg = cfg_open();
        let fwd: Arc<dyn CallFwd> = Arc::new(ImmediateOk(Mutex::new(Some(json!({
            "stdout": "",
            "stderr": "",
            "exit_code": 0,
            "duration_ms": 30000,
            "timed_out": true,
        })))));
        let sandbox_id = Uuid::new_v4();
        let resp = handle_sandbox_for_test(
            cfg.clone(),
            fwd,
            sandbox_id,
            vec!["sleep".into(), "60".into()],
            30000,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.status, JobStatus::Killed);

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn sandbox_job_in_vm_timeout_error_marks_killed() {
        // The adapter's in-VM timeout, as the router forwards it.
        struct TimedOut;
        #[async_trait]
        impl CallFwd for TimedOut {
            async fn call(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
                Err(Error::Remote {
                    code: "invocation_failed".into(),
                    message: r#"handler error: {"code":"S200","message":"exec timed out after 1000 ms","retryable":false,"type":"execution"}"#.into(),
                    stacktrace: None,
                })
            }
        }
        let resp = handle_sandbox_for_test(
            cfg_open(),
            Arc::new(TimedOut),
            Uuid::new_v4(),
            vec!["sleep".into(), "30".into()],
            1000,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.status, JobStatus::Killed);
        assert!(r.record.stderr.contains("exec timed out"));

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn sandbox_job_call_error_marks_failed_with_message() {
        struct AlwaysErr;
        #[async_trait]
        impl CallFwd for AlwaysErr {
            async fn call(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
                Err(Error::Remote {
                    code: "S300".into(),
                    message: "VM boot failed".into(),
                    stacktrace: None,
                })
            }
        }
        let cfg = cfg_open();
        let resp = handle_sandbox_for_test(
            cfg.clone(),
            Arc::new(AlwaysErr),
            Uuid::new_v4(),
            vec!["echo".into()],
            5000,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        assert_eq!(r.record.status, JobStatus::Failed);
        assert!(r.record.stderr.contains("S300"));

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    #[tokio::test]
    async fn sandbox_job_late_completion_does_not_overwrite_killed() {
        // Race scenario: shell::kill marks the JobRecord Killed BEFORE the
        // sandbox::exec call returns. The late response must populate
        // stdout/stderr/exit_code (the data is still useful for debugging)
        // but must NOT overwrite the Killed status to Finished. This guards
        // the `already_killed` check in spawn_sandbox_job's tokio task.
        struct GatedFwd {
            release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
            response: Mutex<Option<Value>>,
        }
        #[async_trait]
        impl CallFwd for GatedFwd {
            async fn call(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
                // Wait until the test releases us via the oneshot channel.
                let rx = self.release.lock().await.take().unwrap();
                rx.await.unwrap();
                Ok(self.response.lock().unwrap().take().unwrap())
            }
        }

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let cfg = cfg_open();
        let fwd: Arc<dyn CallFwd> = Arc::new(GatedFwd {
            release: tokio::sync::Mutex::new(Some(rx)),
            response: Mutex::new(Some(json!({
                "stdout": "late\n",
                "stderr": "",
                "exit_code": 0,
                "duration_ms": 5,
                "timed_out": false,
            }))),
        });
        let sandbox_id = Uuid::new_v4();
        let resp = handle_sandbox_for_test(
            cfg.clone(),
            fwd,
            sandbox_id,
            vec!["echo".into(), "late".into()],
            5000,
        )
        .await
        .unwrap();

        // The background task is now parked inside `fwd.call`. While it's
        // blocked, simulate shell::kill mutating the record to Killed.
        let kill_time_ms = jobs::now_ms();
        {
            let h = jobs::get(&resp.job_id).await.expect("job exists");
            let mut r = h.lock().await;
            r.record.status = JobStatus::Killed;
            r.record.finished_at_ms = Some(kill_time_ms);
        }

        // Release the call; the late completion will run.
        let _ = tx.send(());
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let h = jobs::get(&resp.job_id).await.expect("job exists");
        let r = h.lock().await;
        // Status MUST remain Killed — the guard's whole point.
        assert_eq!(r.record.status, JobStatus::Killed);
        // Late response stdout was still captured as a courtesy.
        assert_eq!(r.record.stdout, "late\n");
        // finished_at_ms must reflect kill-time, not the late completion
        // time. Pollers reading this field expect the timestamp of the
        // user-visible cancellation, not when the in-VM process finally
        // finished — clobbering that value would mislead any UI showing
        // job duration or "ended at" times.
        assert_eq!(r.record.finished_at_ms, Some(kill_time_ms));

        drop(r);
        jobs::JOBS.map.lock().await.remove(&resp.job_id);
    }

    /// Test seam: the production `handle` accepts `iii_sdk::IIIClient` and
    /// constructs the backend internally. Tests inject the CallFwd
    /// directly to avoid spinning up an engine. The implementation
    /// factors out a `spawn_sandbox_job` helper that this shim calls.
    async fn handle_sandbox_for_test(
        cfg: Arc<ShellConfig>,
        fwd: Arc<dyn CallFwd>,
        sandbox_id: Uuid,
        argv: Vec<String>,
        timeout_ms: u64,
    ) -> Result<crate::functions::types::ExecBgResponse, String> {
        super::spawn_sandbox_job(None, None, cfg, fwd, sandbox_id, argv, timeout_ms).await
    }
}
