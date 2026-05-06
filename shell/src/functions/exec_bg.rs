use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::config::ShellConfig;
use crate::exec::{build_command, parse_argv};
use crate::functions::types::ExecBgResponse;
use crate::jobs::{self, JobHandle, JobRecord, JobStatus};
use tokio::io::AsyncReadExt;

pub async fn handle(cfg: Arc<ShellConfig>, payload: Value) -> Result<ExecBgResponse, String> {
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'command'".to_string())?;
    // See exec.rs — silently dropping non-string args is especially
    // dangerous for a backgrounded long-lived job.
    let args: Option<Vec<String>> = match payload.get("args") {
        None | Some(Value::Null) => None,
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                match v.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(format!("'args[{}]' must be a string (got {})", i, v));
                    }
                }
            }
            Some(out)
        }
        Some(other) => {
            return Err(format!(
                "'args' must be an array of strings (got {})",
                other
            ));
        }
    };

    let argv = parse_argv(command, args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

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
