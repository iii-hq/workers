use std::sync::Arc;

use uuid::Uuid;

use crate::config::ShellConfig;
use crate::exec::host::{build_command, parse_argv};
use crate::exec::sandbox::SandboxExecResponse;
use crate::functions::types::{ExecBgRequest, ExecBgResponse};
use crate::jobs::{self, JobHandle, JobRecord, JobStatus};
use crate::target::Target;
use crate::triggers::{IiiTriggerFwd, TriggerFwd};
use tokio::io::AsyncReadExt;

pub async fn handle(
    cfg: Arc<ShellConfig>,
    iii: iii_sdk::III,
    req: ExecBgRequest,
) -> Result<ExecBgResponse, String> {
    // Field-level type errors (wrong-type `command`, non-string `args[i]`,
    // bad `target.kind`) come from the per-field deserializers in
    // `functions::types`; the SDK forwards them as the trigger `Err` with
    // the actionable text the LLM needs to self-correct.
    let argv = parse_argv(&req.command, Some(&req.args)).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    match req.target {
        Target::Host => spawn_host_job(cfg, argv).await,
        Target::Sandbox { sandbox_id } => {
            // Resolve+clamp timeout for the sandbox path; host path
            // ignores timeout_ms (preserves today's unbounded host-bg
            // semantics — documented in README "Caveats").
            let resolved = cfg.resolve_timeout(req.timeout_ms);
            let fwd: Arc<dyn TriggerFwd> = Arc::new(IiiTriggerFwd::new(iii));
            spawn_sandbox_job(cfg, fwd, sandbox_id, argv, resolved).await
        }
    }
}

async fn spawn_host_job(
    cfg: Arc<ShellConfig>,
    argv: Vec<String>,
) -> Result<ExecBgResponse, String> {
    let mut cmd = build_command(&argv, &cfg)?;
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {}", e))?;

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
    match jobs::try_reserve_and_insert(
        JobHandle {
            record,
            child: Some(child),
        },
        cfg.max_concurrent_jobs,
    )
    .await
    {
        Ok(_) => {}
        Err((running, mut handle)) => {
            if let Some(mut ch) = handle.child.take() {
                let _ = ch.start_kill();
            }
            return Err(format!(
                "max concurrent jobs ({}) reached, currently running: {}",
                cfg.max_concurrent_jobs, running
            ));
        }
    }

    let id_clone = id.clone();
    let limit = cfg.max_output_bytes;
    tokio::spawn(async move {
        let handle = match jobs::get(&id_clone).await {
            Some(h) => h,
            None => return,
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

        let (stdout_buf, stdout_trunc) = match stdout_task {
            Some(t) => t.await.unwrap_or_else(|_| (Vec::new(), false)),
            None => (Vec::new(), false),
        };
        let (stderr_buf, stderr_trunc) = match stderr_task {
            Some(t) => t.await.unwrap_or_else(|_| (Vec::new(), false)),
            None => (Vec::new(), false),
        };

        {
            let mut h = handle.lock().await;
            if let Some(mut ch) = h.child.take() {
                drop(h);
                let wait_res = ch.wait().await;
                let mut h2 = handle.lock().await;
                match wait_res {
                    Ok(s) => {
                        h2.record.exit_code = s.code();
                        if h2.record.status == JobStatus::Running {
                            h2.record.status = if s.success() {
                                JobStatus::Finished
                            } else {
                                JobStatus::Failed
                            };
                        }
                    }
                    Err(_) => {
                        h2.record.status = JobStatus::Failed;
                    }
                }
            }
        }

        let mut h = handle.lock().await;
        h.record.stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
        h.record.stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
        h.record.stdout_truncated = stdout_trunc;
        h.record.stderr_truncated = stderr_trunc;
        h.record.finished_at_ms = Some(jobs::now_ms());
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
    match jobs::try_reserve_and_insert(
        JobHandle {
            record,
            child: None,
        },
        cfg.max_concurrent_jobs,
    )
    .await
    {
        Ok(_) => {}
        Err((running, _)) => {
            // No orphan child to kill — sandbox jobs don't own a local process.
            return Err(format!(
                "max concurrent jobs ({}) reached, currently running: {}",
                cfg.max_concurrent_jobs, running
            ));
        }
    }

    let id_clone = id.clone();
    let argv_for_payload = argv.clone();
    tokio::spawn(async move {
        let cmd = argv_for_payload[0].clone();
        let args: Vec<String> = argv_for_payload.iter().skip(1).cloned().collect();
        let payload = serde_json::json!({
            "sandbox_id": sandbox_id.to_string(),
            "cmd": cmd,
            "args": args,
            "timeout_ms": timeout_ms,
        });
        let res = fwd.trigger("sandbox::exec", payload).await;

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
mod sandbox_path_tests {
    use super::*;
    use crate::config::ShellConfig;
    use crate::jobs::{self, JobStatus};
    use crate::triggers::TriggerFwd;
    use async_trait::async_trait;
    use iii_sdk::IIIError;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    struct ImmediateOk(Mutex<Option<Value>>);

    #[async_trait]
    impl TriggerFwd for ImmediateOk {
        async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, IIIError> {
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
            async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, IIIError> {
                Err(IIIError::Remote {
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
            async fn trigger(&self, _fid: &str, _payload: Value) -> Result<Value, IIIError> {
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

    /// Test seam: the production `handle` accepts `iii_sdk::III` and
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
