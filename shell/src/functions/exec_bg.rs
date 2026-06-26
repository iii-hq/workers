use std::sync::Arc;

use uuid::Uuid;

use crate::config::ShellConfig;
use crate::exec::host::{build_command, parse_argv};
use crate::exec::policy::{base_dir_for_target, build_overrides, ExecOverrides};
use crate::exec::sandbox::SandboxExecResponse;
use crate::functions::types::{ExecBgRequest, ExecBgResponse};
use crate::jobs::{self, JobHandle, JobRecord, JobStatus};
use crate::target::Target;
use crate::triggers::{IiiTriggerFwd, TriggerFwd};
use tokio::io::AsyncReadExt;

/// Grace for joining a host job's stdout/stderr drain tasks after the child has
/// exited. A process-group kill closes the pipes so the drains finish at once;
/// this only bounds the pathological case where the direct child exited but a
/// grandchild inherited and still holds the pipe — there the job finalizes
/// (freeing its slot) instead of wedging forever.
const DRAIN_GRACE_MS: u64 = 10_000;

/// Slack added to a sandbox job's in-VM `timeout_ms` to form the client-side RPC
/// deadline. The engine should answer within timeout_ms; the slack covers
/// transport + queueing before we declare it unresponsive and finalize the job.
const SANDBOX_RPC_SLACK_MS: u64 = 30_000;

pub async fn handle(
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::IIIClient,
    req: ExecBgRequest,
) -> Result<ExecBgResponse, String> {
    // Field-level type errors (wrong-type `command`, non-string `args[i]`,
    // bad `target.kind`) come from the per-field deserializers in
    // `functions::types`; the SDK forwards them as the trigger `Err` with
    // the actionable text the LLM needs to self-correct.
    // See `functions::exec` — `args.as_ref()` preserves the shell-words
    // tokenization contract when the caller omits `args`.
    let argv = parse_argv(&req.command, req.args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    // Gate the per-call cwd/env up front, BEFORE branching on target. Same
    // rules as shell::exec (jail-confined cwd, allowed_env + dangerous-key env
    // gating). exec_bg returns its spawn-time failures as plain strings (its
    // documented contract), so we stringify the S-code into the message — the
    // agent still sees the code (e.g. "S215") and the self-correcting text.
    // `base_dir` only scopes the host working directory: drop it for a sandbox
    // target so the harness-injected session dir does not surface as a host cwd
    // override and trip the host-only rejection below (see `base_dir_for_target`).
    let base_dir = base_dir_for_target(&req.target, req.base_dir.as_deref());
    let mut overrides = build_overrides(req.cwd.as_deref(), req.env.as_ref(), base_dir, &cfg)
        .map_err(|e| format!("{}: {}", e.code, e.message))?;
    // stdin needs no gating (opaque input bytes); host-only via is_empty().
    overrides.stdin = req.stdin;

    match req.target {
        Target::Host => spawn_host_job(cfg, argv, overrides).await,
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
            // the per-call timeout_ms but is no longer unbounded: spawn_host_job
            // bounds the detached wait by cfg.max_timeout_ms (the hard cap), so a
            // runaway host bg job is killed and finalized rather than leaking.
            let resolved = cfg.resolve_timeout(req.timeout_ms);
            let fwd: Arc<dyn TriggerFwd> = Arc::new(IiiTriggerFwd::new(iii));
            spawn_sandbox_job(cfg, fwd, sandbox_id, argv, resolved).await
        }
    }
}

pub(crate) async fn spawn_host_job(
    cfg: Arc<ShellConfig>,
    argv: Vec<String>,
    overrides: ExecOverrides,
) -> Result<ExecBgResponse, String> {
    let mut cmd = build_command(&argv, &cfg, &overrides)?;
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {}", e))?;
    crate::exec::host::pump_stdin(&mut child, &overrides.stdin);

    // Capture the OS pid NOW, before the detached drain task takes the `Child`
    // out of the handle. Once the task owns the Child, `JobHandle.child` is None
    // even while the process is alive, so this pid is the only way shell::kill and
    // the shutdown sweep can still signal the live host process.
    let host_pid = child.id();

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let id = format!("job-{}", Uuid::new_v4());
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
    // Atomic check-and-insert: prevents a TOCTOU where two concurrent
    // exec_bg calls both pass a separate running_count() check before
    // either insert lands. On rejection, kill the orphaned child.
    let running_guard = match jobs::try_reserve_and_insert(
        JobHandle {
            record,
            child: Some(child),
            host_pid,
        },
        cfg.max_concurrent_jobs,
    )
    .await
    {
        Ok((_, guard)) => guard,
        Err((running, mut handle)) => {
            if let Some(mut ch) = handle.child.take() {
                // The child was spawned with its own process group
                // (process_group(0)), and a long-running command may have
                // already forked descendants before we lost the slot race.
                // start_kill() only signals the direct leader, leaking the
                // group — so SIGKILL the whole group. Safe to signal by pid
                // here: we still own the un-reaped Child, so the pid cannot
                // have been recycled. THEN reap with wait() to release it.
                crate::exec::host::kill_process_group(ch.id());
                let _ = ch.start_kill();
                let _ = ch.wait().await;
            }
            return Err(format!(
                "max concurrent jobs ({}) reached, currently running: {}",
                cfg.max_concurrent_jobs, running
            ));
        }
    };

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

        // Drain stdout/stderr concurrently — sequential reads deadlock
        // once the child fills one pipe's ~64 KiB buffer before closing
        // the other.
        let stdout_task = stdout_pipe.map(|mut out| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut trunc = false;
                read_bounded(&mut out, limit, &mut buf, &mut trunc).await;
                (buf, trunc)
            })
        });
        let stderr_task = stderr_pipe.map(|mut err| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut trunc = false;
                read_bounded(&mut err, limit, &mut buf, &mut trunc).await;
                (buf, trunc)
            })
        });

        // Take ownership of the Child so we can wait()/kill it without holding
        // the handle lock across an await.
        let child = {
            let mut h = handle.lock().await;
            h.child.take()
        };

        // Race the child's exit against (a) the hard cap and (b) an external kill
        // request (shell::kill / shutdown sweep, delivered via `kill_notify`). On
        // either trigger we kill the child's WHOLE process group while we still
        // own the un-reaped Child — so the pid/pgid cannot have been reused and
        // grandchildren die too — then loop back to reap via wait(). The `if`
        // guards make the cap and kill arms fire at most once each.
        let mut timed_out = false;
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
            loop {
                tokio::select! {
                    r = ch.wait() => break r.ok(),
                    _ = &mut cap, if !timed_out && !killed_by_request => {
                        timed_out = true;
                        crate::exec::host::kill_process_group(pid);
                    }
                    _ = kill_notify.notified(), if !timed_out && !killed_by_request => {
                        killed_by_request = true;
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
        let (stdout_buf, stdout_trunc) = join_drain(stdout_task).await;
        let (stderr_buf, stderr_trunc) = join_drain(stderr_task).await;

        // Single finalize critical section: status, exit_code, stdout, stderr and
        // finished_at_ms are published together, so a poller never observes a
        // terminal status with empty output (or output with a stale status).
        let mut h = handle.lock().await;
        // Finalize-once: an external killer (shell::kill / shutdown sweep) may
        // have already flipped status to Killed and stamped finished_at_ms. Never
        // downgrade a terminal status, and never clobber that kill-time timestamp.
        if h.record.status == JobStatus::Running {
            h.record.status = if timed_out || killed_by_request {
                JobStatus::Killed
            } else {
                match &exit_status {
                    Some(s) if s.success() => JobStatus::Finished,
                    _ => JobStatus::Failed,
                }
            };
        }
        h.record.exit_code = exit_status.and_then(|s| s.code());
        h.record.stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
        h.record.stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
        if timed_out {
            // Append the cause without discarding any stderr the child emitted
            // before it was killed.
            let note =
                format!("background job exceeded the host hard cap of {cap_ms}ms and was killed");
            if h.record.stderr.is_empty() {
                h.record.stderr = note;
            } else {
                h.record.stderr.push('\n');
                h.record.stderr.push_str(&note);
            }
        }
        h.record.stdout_truncated = stdout_trunc;
        h.record.stderr_truncated = stderr_trunc;
        if h.record.finished_at_ms.is_none() {
            h.record.finished_at_ms = Some(jobs::now_ms());
        }
        drop(h);
        jobs::unregister_kill_signal(&id_clone);
    });

    Ok(ExecBgResponse { job_id: id, argv })
}

pub(crate) async fn spawn_sandbox_job(
    cfg: Arc<ShellConfig>,
    fwd: Arc<dyn TriggerFwd>,
    sandbox_id: Uuid,
    argv: Vec<String>,
    timeout_ms: u64,
) -> Result<ExecBgResponse, String> {
    let id = format!("job-{}", Uuid::new_v4());
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
    let running_guard = match jobs::try_reserve_and_insert(
        JobHandle {
            record,
            child: None,
            // Sandbox jobs own no local OS process — leave host_pid None so the
            // kill sweep and shell::kill correctly route them to the sandbox path.
            host_pid: None,
        },
        cfg.max_concurrent_jobs,
    )
    .await
    {
        Ok((_, guard)) => guard,
        Err((running, _)) => {
            // No orphan child to kill — sandbox jobs don't own a local process.
            return Err(format!(
                "max concurrent jobs ({}) reached, currently running: {}",
                cfg.max_concurrent_jobs, running
            ));
        }
    };

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
        let res =
            match tokio::time::timeout(rpc_timeout, fwd.trigger("sandbox::exec", payload)).await {
                Ok(r) => r,
                Err(_) => {
                    // RPC deadline blown: finalize the job (unless an external kill
                    // already did) so its slot/gauge are released, then stop.
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
                    }
                    return;
                }
            };

        let handle = match jobs::get(&id_clone).await {
            Some(h) => h,
            None => return,
        };
        let mut h = handle.lock().await;

        // If shell::kill marked this Killed before the trigger returned, do not
        // overwrite the status — but capture stdout/stderr for completeness.
        let already_killed = h.record.status == JobStatus::Killed;

        match res {
            Ok(value) => {
                let parsed: Result<SandboxExecResponse, _> = serde_json::from_value(value);
                match parsed {
                    Ok(p) => {
                        h.record.stdout = p.stdout;
                        h.record.stderr = p.stderr;
                        h.record.exit_code = Some(p.exit_code);
                        if !already_killed {
                            h.record.status = if p.timed_out {
                                JobStatus::Killed
                            } else if p.exit_code == 0 {
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
                    h.record.status = JobStatus::Failed;
                }
                h.record.stderr = format!("engine error: {err:?}");
            }
        }
        // Same guard as the status writes: if shell::kill already
        // recorded a finished_at_ms when it flipped status to Killed,
        // a late trigger response must NOT clobber that timestamp —
        // pollers reading finished_at_ms expect the kill-time, not
        // the in-VM process's eventual completion time.
        if !already_killed {
            h.record.finished_at_ms = Some(jobs::now_ms());
        }
    });

    Ok(ExecBgResponse { job_id: id, argv })
}

/// Join one drain task under [`DRAIN_GRACE_MS`]. Returns `(bytes, truncated)`.
/// On grace expiry (a grandchild is holding the pipe open after the child
/// exited) the reader task is aborted to release its file descriptor and the
/// stream is reported truncated, so the owning job can still finalize.
async fn join_drain(task: Option<tokio::task::JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
    match task {
        None => (Vec::new(), false),
        Some(mut t) => {
            match tokio::time::timeout(std::time::Duration::from_millis(DRAIN_GRACE_MS), &mut t)
                .await
            {
                Ok(Ok(v)) => v,
                // Reader task panicked or was cancelled: no recoverable output.
                Ok(Err(_)) => (Vec::new(), false),
                // Grace expired: abort the reader to free the fd, report truncated.
                Err(_) => {
                    t.abort();
                    (Vec::new(), true)
                }
            }
        }
    }
}

async fn read_bounded<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    limit: usize,
    buf: &mut Vec<u8>,
    truncated: &mut bool,
) {
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if !*truncated {
                    if buf.len() + n > limit {
                        let take = limit.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..take]);
                        *truncated = true;
                        // Keep draining to avoid SIGPIPE on the child.
                    } else {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
            }
            Err(_) => break,
        }
    }
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
            inherit_env: true,
            max_output_bytes: 4096,
            max_timeout_ms: bg_cap_ms,
            max_bg_timeout_ms: bg_cap_ms,
            max_concurrent_jobs,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        Arc::new(c)
    }

    /// Poll a job until it is fully finalized (`finished_at_ms` set — the last
    /// write the detached task makes) or the deadline expires, then return its
    /// status. Waiting on `finished_at_ms` rather than just a non-Running
    /// status avoids racing the gap between the status flip and the trailing
    /// stdout/stderr/finished_at_ms writes. Tighter than a fixed sleep:
    /// returns the instant the task finalizes, so the test stays fast.
    async fn await_terminal(job_id: &str, deadline_ms: u64) -> JobStatus {
        let start = std::time::Instant::now();
        loop {
            if let Some(h) = jobs::get(job_id).await {
                let r = h.lock().await;
                if r.record.finished_at_ms.is_some() {
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
        let resp = spawn_host_job(cfg, vec!["true".into()], ExecOverrides::default())
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
        // Drive the full public `spawn_host_job` reject path with cap=0 (always
        // over the budget — deterministic regardless of the global JOBS map
        // shared across the concurrent test binary, which a cap=1+count test
        // would race). The spawn must be REJECTED with the cap message; the
        // "rejected job never enters JOBS, and its child is killed" guarantees
        // are proved structurally in jobs.rs (`try_reserve_with_zero_cap_*`,
        // which asserts the handle is returned and never inserted) and at the
        // OS pid level in `rejected_child_pid_is_dead` below.
        let cfg = cfg(30_000, 0);
        let err = spawn_host_job(
            cfg.clone(),
            vec!["sleep".into(), "30".into()],
            ExecOverrides::default(),
        )
        .await
        .expect_err("cap=0 must reject the spawn");
        assert!(
            err.contains("max concurrent jobs"),
            "rejection message: {err}"
        );
    }

    /// Direct, PID-level proof that the reject arm's child is actually dead
    /// (not leaked). Mirrors exactly what `spawn_host_job` does on rejection:
    /// build + spawn a child, capture its PID, then `try_reserve_and_insert`
    /// with cap=0 (forced rejection) and `start_kill` the returned handle —
    /// the same two lines as the production reject arm. We then reap the child
    /// and assert via `kill -0 <pid>` that the OS no longer knows the process.
    #[cfg(unix)]
    #[tokio::test]
    async fn rejected_child_pid_is_dead() {
        use crate::exec::host::build_command;
        use crate::exec::policy::ExecOverrides;
        use crate::jobs::{JobHandle, JobRecord};

        let cfg = cfg(30_000, 16);
        let argv = vec!["sleep".to_string(), "30".to_string()];
        let mut command =
            build_command(&argv, &cfg, &ExecOverrides::default()).expect("build_command");
        let mut child = command.spawn().expect("spawn");
        let pid = child.id().expect("child has a pid");

        // Sanity: the process is alive right now (kill -0 succeeds).
        assert!(
            pid_is_alive(pid),
            "freshly spawned child should be alive (pid {pid})"
        );

        let record = JobRecord {
            id: format!("job-{}", uuid::Uuid::new_v4()),
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
        // child.stdout/stderr are still owned by `child`; that's fine — we move
        // the whole child into the handle and never read its pipes here.
        // Drain the pipes off the handle the way spawn_host_job does, so the
        // handle's child is the sole owner of the process.
        let _ = child.stdout.take();
        let _ = child.stderr.take();

        // cap=0 forces rejection, returning the handle with its child intact —
        // exactly the path spawn_host_job hits when over the cap.
        let (_, mut handle) = jobs::try_reserve_and_insert(
            JobHandle {
                record,
                child: Some(child),
                host_pid: Some(pid),
            },
            0,
        )
        .await
        .expect_err("cap=0 must reject and return the handle");

        // The production reject arm does precisely this:
        let mut killed_child = handle.child.take().expect("rejected handle owns child");
        killed_child
            .start_kill()
            .expect("start_kill on rejected child");
        // Reap so the kernel releases the zombie before we probe kill -0.
        let _ = killed_child.wait().await;

        // The OS must no longer have this pid as a live (non-zombie) process.
        assert!(
            !pid_is_alive(pid),
            "rejected child (pid {pid}) must be dead, not leaked"
        );
    }

    /// `kill(pid, 0)` probes for the existence of a *live* process without
    /// sending a signal. After we wait() the child above, the zombie is reaped,
    /// so this returns false for a dead process.
    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        // SAFETY: signal 0 performs error checking only; it never delivers a
        // signal or touches process memory.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
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
    use crate::config::ShellConfig;
    use crate::jobs::{self, JobStatus};
    use crate::triggers::TriggerFwd;
    use async_trait::async_trait;
    use iii_sdk::errors::Error;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    struct ImmediateOk(Mutex<Option<Value>>);

    #[async_trait]
    impl TriggerFwd for ImmediateOk {
        async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
            Ok(self.0.lock().unwrap().take().unwrap())
        }
    }

    fn cfg_open() -> Arc<ShellConfig> {
        let mut c = ShellConfig {
            inherit_env: true,
            max_output_bytes: 4096,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        Arc::new(c)
    }

    #[tokio::test]
    async fn sandbox_job_transitions_to_finished() {
        let cfg = cfg_open();
        let fwd: Arc<dyn TriggerFwd> = Arc::new(ImmediateOk(Mutex::new(Some(json!({
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

    #[tokio::test]
    async fn sandbox_job_timed_out_marks_killed() {
        let cfg = cfg_open();
        let fwd: Arc<dyn TriggerFwd> = Arc::new(ImmediateOk(Mutex::new(Some(json!({
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
    async fn sandbox_job_trigger_error_marks_failed_with_message() {
        struct AlwaysErr;
        #[async_trait]
        impl TriggerFwd for AlwaysErr {
            async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
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
        // sandbox::exec trigger returns. The late response must populate
        // stdout/stderr/exit_code (the data is still useful for debugging)
        // but must NOT overwrite the Killed status to Finished. This guards
        // the `already_killed` check in spawn_sandbox_job's tokio task.
        struct GatedFwd {
            release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
            response: Mutex<Option<Value>>,
        }
        #[async_trait]
        impl TriggerFwd for GatedFwd {
            async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, Error> {
                // Wait until the test releases us via the oneshot channel.
                let rx = self.release.lock().await.take().unwrap();
                rx.await.unwrap();
                Ok(self.response.lock().unwrap().take().unwrap())
            }
        }

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let cfg = cfg_open();
        let fwd: Arc<dyn TriggerFwd> = Arc::new(GatedFwd {
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

        // The background task is now parked inside `fwd.trigger`. While it's
        // blocked, simulate shell::kill mutating the record to Killed.
        let kill_time_ms = jobs::now_ms();
        {
            let h = jobs::get(&resp.job_id).await.expect("job exists");
            let mut r = h.lock().await;
            r.record.status = JobStatus::Killed;
            r.record.finished_at_ms = Some(kill_time_ms);
        }

        // Release the trigger; the late completion will run.
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
    /// constructs the backend internally. Tests inject the TriggerFwd
    /// directly to avoid spinning up an engine. The implementation
    /// factors out a `spawn_sandbox_job` helper that this shim calls.
    async fn handle_sandbox_for_test(
        cfg: Arc<ShellConfig>,
        fwd: Arc<dyn TriggerFwd>,
        sandbox_id: Uuid,
        argv: Vec<String>,
        timeout_ms: u64,
    ) -> Result<crate::functions::types::ExecBgResponse, String> {
        super::spawn_sandbox_job(cfg, fwd, sandbox_id, argv, timeout_ms).await
    }
}
