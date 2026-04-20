use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::ShellConfig;
use crate::exec::{build_command, parse_argv};
use crate::jobs::{self, JobHandle, JobRecord, JobStatus};
use tokio::io::AsyncReadExt;

pub fn build_handler(
    config: Arc<ShellConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let cfg = config.clone();
        Box::pin(async move { handle(cfg, payload).await })
    }
}

async fn handle(cfg: Arc<ShellConfig>, payload: Value) -> Result<Value, IIIError> {
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing 'command'".to_string()))?;
    let args: Option<Vec<String>> = payload
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect());

    let argv = parse_argv(command, args.as_ref())
        .map_err(|e| IIIError::Handler(format!("argv: {}", e)))?;

    cfg.is_command_allowed(&argv)
        .map_err(IIIError::Handler)?;

    let running = jobs::running_count().await;
    if running >= cfg.max_concurrent_jobs {
        return Err(IIIError::Handler(format!(
            "max concurrent jobs ({}) reached",
            cfg.max_concurrent_jobs
        )));
    }

    let mut cmd = build_command(&argv, &cfg).map_err(IIIError::Handler)?;
    let mut child = cmd
        .spawn()
        .map_err(|e| IIIError::Handler(format!("spawn: {}", e)))?;

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
    jobs::insert(JobHandle {
        record,
        child: Some(child),
    })
    .await;

    let id_clone = id.clone();
    let limit = cfg.max_output_bytes;
    tokio::spawn(async move {
        let handle = match jobs::get(&id_clone).await {
            Some(h) => h,
            None => return,
        };
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_trunc = false;
        let mut stderr_trunc = false;

        if let Some(mut out) = stdout_pipe {
            read_bounded(&mut out, limit, &mut stdout_buf, &mut stdout_trunc).await;
        }
        if let Some(mut err) = stderr_pipe {
            read_bounded(&mut err, limit, &mut stderr_buf, &mut stderr_trunc).await;
        }

        let status = {
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
                h2.record.status.clone()
            } else {
                h.record.status.clone()
            }
        };

        let mut h = handle.lock().await;
        h.record.stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
        h.record.stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
        h.record.stdout_truncated = stdout_trunc;
        h.record.stderr_truncated = stderr_trunc;
        h.record.finished_at_ms = Some(jobs::now_ms());
        let _ = status;
    });

    Ok(json!({
        "job_id": id,
        "argv": argv,
    }))
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
                if buf.len() + n > limit {
                    let take = limit.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                    *truncated = true;
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(_) => break,
        }
    }
}
