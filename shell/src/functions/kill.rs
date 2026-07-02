use crate::exec::error::ExecError;
use crate::functions::types::{KillRequest, KillResponse};
use crate::jobs::{self, JobStatus};

pub async fn handle(req: KillRequest) -> Result<KillResponse, ExecError> {
    // Return the TYPED ExecError (not its JSON string): main.rs's
    // `.map_err(Error::from)` lifts it to `Error::Remote`, so the S-code
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
    // task has not yet taken the Child). We still own the un-reaped Child, so
    // the pid cannot have been recycled — SIGKILL the whole process group
    // (process_group(0) at spawn) so a command that already forked cannot leave
    // descendants running after we report killed: true. The group SIGKILL is
    // the authoritative kill; start_kill() is best-effort on the leader (it can
    // race the group signal and find the leader already a zombie — that error
    // is not a kill failure, so don't surface it). Reaping is left to the drain
    // task's wait().
    if let Some(child) = h.child.as_mut() {
        crate::exec::host::kill_process_group(child.id());
        let _ = child.start_kill();
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
    // spawn). We do NOT signal a bare pid here: the drain task may be between its
    // `wait()` reaping the child and acquiring the lock to flip status, so the pid
    // could already be free for OS reuse — signalling it could hit an unrelated
    // process (and the worker may run as root). Instead, mark the job Killed and
    // notify its kill-signal channel; the drain task, which still owns the
    // un-reaped Child, kills the process group safely while the pid is guaranteed
    // live, and its finalize-once guard preserves this Killed status + timestamp.
    if h.host_pid.is_some() {
        h.record.status = JobStatus::Killed;
        h.record.finished_at_ms = Some(jobs::now_ms());
        let status = h.record.status.clone();
        let notify = jobs::kill_signal_for(&req.job_id);
        // Release the handle lock BEFORE notifying so the woken drain task can
        // acquire it immediately to perform the group-kill and finalize.
        drop(h);
        if let Some(n) = notify {
            n.notify_one();
        }
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: true,
            status,
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
    /// `Error::Remote { code: "S211", .. }`, which the engine SDK maps to
    /// the wire `code` verbatim — NOT the `invocation_failed`/Handler collapse.
    #[tokio::test]
    async fn killing_missing_job_lifts_to_remote_s211() {
        let err = handle(KillRequest {
            job_id: "job-does-not-exist".into(),
        })
        .await
        .expect_err("missing job must error");
        match iii_sdk::errors::Error::from(err) {
            iii_sdk::errors::Error::Remote { code, message, .. } => {
                assert_eq!(code, "S211");
                assert!(message.contains("no such job"));
            }
            other => panic!("expected Error::Remote, got {other:?}"),
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
            env: crate::config::EnvConfig {
                inherit: true,
                ..Default::default()
            },
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

    /// A RUNNING host bg job whose drain task already owns the `Child`
    /// (`child: None` in the handle) is terminated by `shell::kill` through the
    /// kill-signal channel: `kill::handle` marks it Killed and notifies; the REAL
    /// drain task (driven here via `spawn_host_job`) group-kills the live process
    /// while it still owns the un-reaped Child. We assert the OS process dies
    /// within a tight 3s deadline and the response carries no sandbox caveat.
    /// (The previous bare-pid SIGKILL was removed: the pid could be reused after
    /// the drain task's `wait()` reaped the child, risking an unrelated kill.)
    #[cfg(unix)]
    #[tokio::test]
    async fn killing_running_host_job_terminates_the_real_child() {
        use crate::jobs::{GAUGE_TEST_GUARD, HOST_SWEEP_TEST_GUARD};
        use std::time::{Duration, Instant};

        // spawn_host_job() reserves a running slot and bumps RUNNING_JOBS, so
        // take the gauge guard FIRST (same order as exec_bg.rs host-path tests)
        // to serialize with the +1/-1 gauge assertions in jobs.rs, then the
        // sweep guard to serialize on the global JOBS map.
        let _gauge_gate = GAUGE_TEST_GUARD.lock().await;
        let _guard = HOST_SWEEP_TEST_GUARD.lock().await;

        let mut cfg = open_cfg();
        cfg.max_timeout_ms = 60_000; // hard cap well past the test window
        cfg.max_concurrent_jobs = 64;
        // Drive the REAL spawn path so a real drain task is listening on the
        // kill-signal channel (a faked handle cannot be killed — by design).
        let resp = crate::functions::exec_bg::spawn_host_job(
            Arc::new(cfg),
            vec!["sleep".to_string(), "30".to_string()],
            ExecOverrides::default(),
        )
        .await
        .expect("spawn host bg job");
        let id = resp.job_id;

        // The drain task records the child's pid on the handle (host_pid).
        let pid = jobs::get(&id)
            .await
            .expect("job exists")
            .lock()
            .await
            .host_pid
            .expect("host bg job has a pid");
        assert!(pid_alive(pid), "child should be alive before kill");

        let resp = handle(KillRequest { job_id: id.clone() }).await.unwrap();
        assert!(resp.killed, "running host job must report killed");
        assert_eq!(resp.status, JobStatus::Killed);
        assert!(
            resp.reason.is_none(),
            "host kill must NOT return the sandbox caveat reason"
        );

        // TIGHT deadline: the notified drain task group-kills the process; a
        // SIGKILL'd `sleep 30` dies within milliseconds.
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

        // Record reflects the cancellation.
        {
            let h = jobs::get(&id).await.expect("job exists");
            let r = h.lock().await;
            assert_eq!(r.record.status, JobStatus::Killed);
            assert!(r.record.finished_at_ms.is_some());
        }

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
