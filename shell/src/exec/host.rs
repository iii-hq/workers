//! Host-process exec backend. Wraps `tokio::process::Command` with the
//! existing env-scrubbing, bounded-output, and timeout-watchdog logic
//! that lived in the flat `src/exec.rs` before the trait split.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::ShellConfig;
use crate::exec::{backend::ExecBackend, error::ExecError, ExecCallResult, ExecOutcome};

pub fn parse_argv(command: &str, args: Option<&Vec<String>>) -> Result<Vec<String>, String> {
    if let Some(args) = args {
        let mut v = vec![command.to_string()];
        v.extend(args.iter().cloned());
        Ok(v)
    } else {
        shell_words::split(command).map_err(|e| format!("parse command: {}", e))
    }
}

pub fn build_command(argv: &[String], cfg: &ShellConfig) -> Result<Command, String> {
    let program = argv.first().ok_or_else(|| "empty command".to_string())?;
    let mut cmd = Command::new(program);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    if !cfg.inherit_env {
        cmd.env_clear();
        for k in &cfg.allowed_env {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
    }
    if let Some(dir) = &cfg.working_dir {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd)
}

pub async fn run_to_completion(
    argv: &[String],
    cfg: &ShellConfig,
    timeout_ms: u64,
) -> Result<ExecOutcome, String> {
    let started = std::time::Instant::now();
    let mut cmd = build_command(argv, cfg)?;
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {}", e))?;

    let mut stdout_reader = child.stdout.take().ok_or("no stdout pipe")?;
    let mut stderr_reader = child.stderr.take().ok_or("no stderr pipe")?;

    let limit = cfg.max_output_bytes;
    let timeout = Duration::from_millis(timeout_ms);

    let stdout_task = tokio::spawn(async move { read_bounded(&mut stdout_reader, limit).await });
    let stderr_task = tokio::spawn(async move { read_bounded(&mut stderr_reader, limit).await });

    let wait_res = tokio::time::timeout(timeout, child.wait()).await;

    let (exit_code, timed_out) = match wait_res {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("wait: {}", e)),
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            (None, true)
        }
    };

    let (stdout_bytes, stdout_truncated) =
        stdout_task.await.map_err(|e| format!("stdout: {}", e))?;
    let (stderr_bytes, stderr_truncated) =
        stderr_task.await.map_err(|e| format!("stderr: {}", e))?;

    Ok(ExecOutcome {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        timed_out,
        stdout_truncated,
        stderr_truncated,
    })
}

async fn read_bounded<R: AsyncReadExt + Unpin>(reader: &mut R, limit: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if !truncated {
                    if buf.len() + n > limit {
                        let take = limit.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..take]);
                        truncated = true;
                        // Fall through and continue draining; do NOT break.
                        // Dropping the reader here closes the pipe, and the
                        // child's next write() raises SIGPIPE — making
                        // ExitStatus::code() report None on what should have
                        // been a normal-exit-with-truncated-output run.
                    } else {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
                // truncated == true: discard everything past `limit`.
            }
            Err(_) => break,
        }
    }
    (buf, truncated)
}

pub struct HostExecBackend {
    cfg: Arc<ShellConfig>,
}

impl HostExecBackend {
    pub fn new(cfg: Arc<ShellConfig>) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl ExecBackend for HostExecBackend {
    async fn run(&self, argv: &[String], timeout_ms: u64) -> ExecCallResult<ExecOutcome> {
        // S216 is the generic "shell-internal error" code in the
        // S2xx range. We deliberately do NOT use S300 here even
        // though it visually parallels SandboxExecBackend's S300
        // path: in iii's S-code taxonomy, S300 is reserved for "VM
        // boot failed" (no virt host). Wrapping host-side spawn or
        // wait failures as S300 would make a "command not found" on
        // the host indistinguishable from a missing /dev/kvm to
        // callers that branch on the code.
        run_to_completion(argv, &self.cfg, timeout_ms)
            .await
            .map_err(|e| ExecError::new("S216", format!("host exec: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> ShellConfig {
        let mut c = ShellConfig {
            inherit_env: true,
            max_output_bytes: 4096,
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        c
    }

    #[test]
    fn test_parse_argv_with_args_field() {
        let got = parse_argv("echo", Some(&vec!["hello".into(), "world".into()])).unwrap();
        assert_eq!(got, vec!["echo", "hello", "world"]);
    }

    #[test]
    fn test_parse_argv_from_shell_words() {
        let got = parse_argv(r#"echo "hello world""#, None).unwrap();
        assert_eq!(got, vec!["echo", "hello world"]);
    }

    #[test]
    fn test_parse_argv_bad_quoting() {
        assert!(parse_argv(r#"echo "unterminated"#, None).is_err());
    }

    #[tokio::test]
    async fn test_run_echo() {
        let cfg = test_cfg();
        let out = run_to_completion(&["echo".into(), "hi".into()], &cfg, 5000)
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "hi");
        assert!(!out.timed_out);
    }

    #[tokio::test]
    async fn test_run_nonexistent_command() {
        let cfg = test_cfg();
        let err = run_to_completion(&["_nope_no_exist_".into()], &cfg, 1000).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_timeout_kills() {
        let cfg = test_cfg();
        let out = run_to_completion(&["sleep".into(), "5".into()], &cfg, 200)
            .await
            .unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, None);
    }

    #[tokio::test]
    async fn test_output_truncation() {
        let mut cfg = test_cfg();
        cfg.max_output_bytes = 16;
        let out = run_to_completion(
            &[
                "sh".into(),
                "-c".into(),
                "printf 'x%.0s' $(seq 1 100)".into(),
            ],
            &cfg,
            3000,
        )
        .await
        .unwrap();
        assert!(out.stdout_truncated);
        assert_eq!(out.stdout.len(), 16);
    }

    #[tokio::test]
    async fn host_backend_runs_via_trait() {
        let cfg = Arc::new(test_cfg());
        let backend = HostExecBackend::new(cfg);
        let out = backend
            .run(&["echo".into(), "via-trait".into()], 5000)
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "via-trait");
    }
}
