use crate::exec::error::ExecError;
use crate::functions::types::{KillRequest, KillResponse};
use crate::jobs::{self, JobStatus};

pub async fn handle(req: KillRequest) -> Result<KillResponse, ExecError> {
    // Return the TYPED ExecError (not its JSON string): main.rs's
    // `.map_err(IIIError::from)` lifts it to `IIIError::Remote`, so the S-code
    // lands as the top-level wire `code` and an agent's single shell:: error
    // handler works here too. job-not-found maps to S211; operational kill
    // failures below use S216 (the exec/fs "other io" code).
    let handle = jobs::get(&req.job_id)
        .await
        .ok_or_else(|| ExecError::new("S211", format!("no such job: {}", req.job_id)))?;

    let mut h = handle.lock().await;
    if h.record.status != JobStatus::Running {
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: false,
            status: h.record.status.clone(),
            reason: Some("not running".into()),
        });
    }

    // Branch 1: the in-handle Child is still present (job spawned but its drain
    // task has not yet taken the Child). start_kill it directly and reap is left
    // to the drain task's own wait().
    if let Some(child) = h.child.as_mut() {
        child.start_kill().map_err(|e| {
            ExecError::new("S216", format!("failed to kill job {}: {}", req.job_id, e))
        })?;
        h.record.status = JobStatus::Killed;
        h.record.finished_at_ms = Some(jobs::now_ms());
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: true,
            status: h.record.status.clone(),
            reason: None,
        });
    }

    // Branch 2: a RUNNING host bg job whose drain task already took the Child out
    // of the handle (the common case — child is None almost immediately after
    // spawn). The live process is unreachable via the handle, but `host_pid` still
    // points at it, so signal SIGKILL directly. The detached drain task's
    // child.wait() reaps the process; its finalize-once guard keeps the Killed
    // status and this kill-time finished_at_ms from being clobbered.
    if let Some(pid) = h.host_pid {
        // SAFETY: SIGKILL to a pid we spawned. The record is still Running, so the
        // drain task has not finalized and PID reuse cannot have happened yet. An
        // already-exited child yields ESRCH, which we still report as killed (the
        // user's intent — terminate — is satisfied; nothing is left running).
        let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        if rc != 0 {
            let errno = std::io::Error::last_os_error();
            // ESRCH means the process already exited between the status check and
            // the signal — the job is effectively done. Any other errno is a real
            // failure to deliver the signal.
            if errno.raw_os_error() != Some(libc::ESRCH) {
                return Err(ExecError::new(
                    "S216",
                    format!(
                        "failed to signal host job {} (pid {}): {}",
                        req.job_id, pid, errno
                    ),
                ));
            }
        }
        h.record.status = JobStatus::Killed;
        h.record.finished_at_ms = Some(jobs::now_ms());
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: true,
            status: h.record.status.clone(),
            reason: None,
        });
    }

    // Branch 3: true sandbox-backed job (host_pid None) — no host child process at
    // all. The in-VM process is reachable only through `sandbox::exec`, which has
    // no cancel hook. Mark the record Killed so shell::status / shell::list reflect
    // the cancellation; the late trigger response captures stdout/stderr but won't
    // overwrite this status (see the `already_killed` guard in
    // functions::exec_bg::spawn_sandbox_job).
    h.record.status = JobStatus::Killed;
    h.record.finished_at_ms = Some(jobs::now_ms());
    Ok(KillResponse {
        job_id: req.job_id,
        killed: true,
        status: h.record.status.clone(),
        reason: Some(
            "sandbox::exec has no cancel hook; the in-VM process will run \
             until its timeout_ms expires"
                .into(),
        ),
    })
}

#[cfg(test)]
mod missing_job_tests {
    use super::*;
    use crate::functions::types::KillRequest;

    #[tokio::test]
    async fn killing_missing_job_returns_typed_s211() {
        let err = handle(KillRequest {
            job_id: "job-does-not-exist".into(),
        })
        .await
        .expect_err("missing job must error");
        // Assert the TYPED ExecError carries the S-code, not a JSON string.
        assert_eq!(err.code, "S211");
        assert!(err.message.contains("no such job"));
    }

    /// Pin the wire contract: the handler's `Err` lifts to
    /// `IIIError::Remote { code: "S211", .. }`, which the engine SDK maps to
    /// the wire `code` verbatim — NOT the `invocation_failed`/Handler collapse.
    #[tokio::test]
    async fn killing_missing_job_lifts_to_remote_s211() {
        let err = handle(KillRequest {
            job_id: "job-does-not-exist".into(),
        })
        .await
        .expect_err("missing job must error");
        match iii_sdk::IIIError::from(err) {
            iii_sdk::IIIError::Remote { code, message, .. } => {
                assert_eq!(code, "S211");
                assert!(message.contains("no such job"));
            }
            other => panic!("expected IIIError::Remote, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod sandbox_kill_tests {
    use super::*;
    use crate::functions::types::KillRequest;
    use crate::jobs::{self, JobHandle, JobRecord};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn killing_sandbox_job_returns_caveat_reason() {
        // Insert a Running sandbox-backed job (child=None) directly into JOBS.
        let id = format!("job-{}", uuid::Uuid::new_v4());
        let record = JobRecord {
            id: id.clone(),
            argv: vec!["echo".into(), "x".into()],
            started_at_ms: jobs::now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };
        jobs::JOBS.map.lock().await.insert(
            id.clone(),
            Arc::new(Mutex::new(JobHandle {
                record,
                child: None,
                host_pid: None,
            })),
        );

        let resp = handle(KillRequest { job_id: id.clone() }).await.unwrap();
        assert!(resp.killed);
        assert_eq!(resp.status, JobStatus::Killed);
        let reason = resp
            .reason
            .expect("sandbox kill must include caveat reason");
        assert!(reason.contains("sandbox::exec"));
        assert!(reason.contains("timeout_ms"));

        // Cleanup.
        jobs::JOBS.map.lock().await.remove(&id);
    }
}

#[cfg(test)]
mod host_kill_tests {
    use super::*;
    use crate::config::ShellConfig;
    use crate::exec::host::build_command;
    use crate::exec::policy::ExecOverrides;
    use crate::functions::types::KillRequest;
    use crate::jobs::{self, JobHandle, JobRecord};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn open_cfg() -> ShellConfig {
        let mut c = ShellConfig {
            inherit_env: true,
            max_output_bytes: 4096,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        c
    }

    fn pid_alive(pid: u32) -> bool {
        // SAFETY: signal 0 only performs permission/existence checking.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    /// The real fix: a RUNNING host bg job whose drain task already took the
    /// `Child` out of the handle (`child: None`) must still be terminated — by
    /// `host_pid`, not the absent in-handle child. Before the fix, kill::handle
    /// fell through to the sandbox else-branch and never signalled the live
    /// process, so a `sleep 30` would keep running for 30s. We assert the pid is
    /// dead within a TIGHT 3s deadline, so reverting the pid-kill (leaving only
    /// the sandbox caveat) makes this test fail. We also assert the response
    /// carries NO sandbox caveat — this is a real host kill, not the in-VM no-op.
    #[cfg(unix)]
    #[tokio::test]
    async fn killing_running_host_job_terminates_the_real_child() {
        use std::time::{Duration, Instant};

        let cfg = open_cfg();
        let argv = vec!["sleep".to_string(), "30".to_string()];
        let mut command =
            build_command(&argv, &cfg, &ExecOverrides::default()).expect("build_command");
        let mut child = command.spawn().expect("spawn sleep");
        let pid = child.id().expect("child has pid");

        // Mirror spawn_host_job's steady state: the detached drain task owns the
        // Child (here a local waiter task), so the handle holds `child: None` and
        // only `host_pid` can reach the live process. Drain the pipes off first.
        let _ = child.stdout.take();
        let _ = child.stderr.take();
        let waiter = tokio::spawn(async move {
            let _ = child.wait().await;
        });

        assert!(pid_alive(pid), "child should be alive before kill");

        let id = format!("job-{}", uuid::Uuid::new_v4());
        let record = JobRecord {
            id: id.clone(),
            argv,
            started_at_ms: jobs::now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };
        jobs::JOBS.map.lock().await.insert(
            id.clone(),
            Arc::new(Mutex::new(JobHandle {
                record,
                // Child taken by the drain task; reachable only via host_pid.
                child: None,
                host_pid: Some(pid),
            })),
        );

        let resp = handle(KillRequest { job_id: id.clone() }).await.unwrap();
        assert!(resp.killed, "running host job must report killed");
        assert_eq!(resp.status, JobStatus::Killed);
        assert!(
            resp.reason.is_none(),
            "host pid-kill must NOT return the sandbox caveat reason"
        );

        // Record reflects the cancellation.
        {
            let h = jobs::get(&id).await.expect("job exists");
            let r = h.lock().await;
            assert_eq!(r.record.status, JobStatus::Killed);
            assert!(r.record.finished_at_ms.is_some());
        }

        // TIGHT deadline: a SIGKILL'd `sleep 30` dies within milliseconds. If the
        // pid-kill were reverted (kill falls to the sandbox no-op), the process
        // would survive the full 30s and blow past this 3s window.
        let start = Instant::now();
        loop {
            if !pid_alive(pid) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "killed host child (pid {pid}) must die within 3s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let _ = waiter.await;
        jobs::JOBS.map.lock().await.remove(&id);
    }

    /// Coverage for branch 1: a Running host job whose `Child` is still in the
    /// handle (drain task has not yet taken it) is terminated via `start_kill`.
    #[cfg(unix)]
    #[tokio::test]
    async fn killing_host_job_with_in_handle_child_uses_start_kill() {
        let cfg = open_cfg();
        let argv = vec!["sleep".to_string(), "30".to_string()];
        let mut command =
            build_command(&argv, &cfg, &ExecOverrides::default()).expect("build_command");
        let mut child = command.spawn().expect("spawn sleep");
        let pid = child.id().expect("child has pid");

        let _ = child.stdout.take();
        let _ = child.stderr.take();

        assert!(pid_alive(pid), "child should be alive before kill");

        let id = format!("job-{}", uuid::Uuid::new_v4());
        let record = JobRecord {
            id: id.clone(),
            argv,
            started_at_ms: jobs::now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };
        jobs::JOBS.map.lock().await.insert(
            id.clone(),
            Arc::new(Mutex::new(JobHandle {
                record,
                child: Some(child),
                host_pid: Some(pid),
            })),
        );

        let resp = handle(KillRequest { job_id: id.clone() }).await.unwrap();
        assert!(resp.killed, "running host job must report killed");
        assert_eq!(resp.status, JobStatus::Killed);
        assert!(resp.reason.is_none(), "host kill has no caveat reason");

        // Reap the killed child so the kernel releases the zombie, then prove the
        // pid is gone.
        if let Some(h) = jobs::get(&id).await {
            let mut g = h.lock().await;
            if let Some(child) = g.child.as_mut() {
                let _ = child.wait().await;
            }
        }
        assert!(!pid_alive(pid), "child (pid {pid}) must be dead after kill");

        jobs::JOBS.map.lock().await.remove(&id);
    }
}
