//! `shell::bash::exec` — spawn bash directly on the host.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const ID: &str = "shell::bash::exec";
pub const DESCRIPTION: &str =
    "Run a bash command on the host. Args: command, timeout_ms?, env?, workdir?, stdin?.";
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const TRIGGER_TIMEOUT_MS: u64 = 35_000;
pub const MAX_OUTPUT_BYTES: usize = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecConfig {
    pub default_timeout_ms: u64,
    pub trigger_timeout_ms: u64,
    pub max_output_bytes: usize,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            trigger_timeout_ms: TRIGGER_TIMEOUT_MS,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    execute_with_config(_iii, args, ExecConfig::default()).await
}

pub async fn execute_with_config(
    _iii: &III,
    args: &Value,
    config: ExecConfig,
) -> Result<Value, IIIError> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: command".into()))?
        .to_string();
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(config.default_timeout_ms);

    let stdin_data = args
        .get("stdin")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-lc").arg(&command);

    // Apply environment variables.
    if let Some(env_map) = args.get("env").and_then(Value::as_object) {
        for (k, v) in env_map {
            if v.is_null() {
                cmd.env_remove(k);
            } else if let Some(s) = v.as_str() {
                cmd.env(k, s);
            }
        }
    }

    // Apply working directory.
    if let Some(wd) = args.get("workdir").and_then(Value::as_str) {
        cmd.current_dir(wd);
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let start = Instant::now();

    if let Some(ref data) = stdin_data {
        use tokio::io::AsyncWriteExt;
        cmd.stdin(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| IIIError::Handler(format!("spawn bash: {e}")))?;
        if let Some(mut stdin_handle) = child.stdin.take() {
            let _ = stdin_handle.write_all(data.as_bytes()).await;
        }
        let result =
            tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await;
        let duration_ms = start.elapsed().as_millis() as u64;
        let v = match result {
            Ok(Ok(output)) => json!({
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "exit_code": output.status.code().unwrap_or(-1),
                "timed_out": false,
                "duration_ms": duration_ms,
            }),
            Ok(Err(e)) => {
                return Err(IIIError::Handler(format!("wait bash: {e}")));
            }
            Err(_) => json!({
                "stdout": "",
                "stderr": format!("timeout after {}ms", timeout_ms),
                "exit_code": -1,
                "timed_out": true,
                "duration_ms": duration_ms,
            }),
        };
        return Ok(render_with_config(&v, &config));
    }

    cmd.stdin(Stdio::null());
    let child = cmd
        .spawn()
        .map_err(|e| IIIError::Handler(format!("spawn bash: {e}")))?;
    let result =
        tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let v = match result {
        Ok(Ok(output)) => json!({
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "exit_code": output.status.code().unwrap_or(-1),
            "timed_out": false,
            "duration_ms": duration_ms,
        }),
        Ok(Err(e)) => {
            return Err(IIIError::Handler(format!("wait bash: {e}")));
        }
        Err(_) => json!({
            "stdout": "",
            "stderr": format!("timeout after {}ms", timeout_ms),
            "exit_code": -1,
            "timed_out": true,
            "duration_ms": duration_ms,
        }),
    };
    Ok(render_with_config(&v, &config))
}

fn render_with_config(v: &Value, config: &ExecConfig) -> Value {
    let stdout = v.get("stdout").and_then(Value::as_str).unwrap_or("");
    let stderr = v.get("stderr").and_then(Value::as_str).unwrap_or("");
    let exit = v.get("exit_code").and_then(Value::as_i64).unwrap_or(-1);
    let mut text = format!("exit={exit}\n");
    text.push_str(stdout);
    if !stderr.is_empty() {
        if !stdout.is_empty() && !stdout.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(stderr);
    }
    let truncated: String = text.chars().take(config.max_output_bytes).collect();
    json!({
        "content": [{ "type": "text", "text": truncated }],
        "details": {
            "exit_code": exit,
            "via": "host",
            "duration_ms": v.get("duration_ms").cloned().unwrap_or(Value::Null),
            "timed_out": v.get("timed_out").cloned().unwrap_or(Value::Null),
        },
        "terminate": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id() {
        assert_eq!(ID, "shell::bash::exec");
    }
    #[test]
    fn render_includes_exit_and_stdout() {
        let v = render_with_config(
            &json!({ "stdout": "hi\n", "stderr": "", "exit_code": 0 }),
            &ExecConfig::default(),
        );
        let text = v["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.starts_with("exit=0\nhi"));
    }
    #[test]
    fn render_appends_stderr() {
        let v = render_with_config(
            &json!({ "stdout": "ok", "stderr": "warn", "exit_code": 1 }),
            &ExecConfig::default(),
        );
        let text = v["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("ok\nwarn"));
        assert!(text.starts_with("exit=1"));
    }
}
