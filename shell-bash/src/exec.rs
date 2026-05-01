//! `shell::bash::exec` — wrap `sandbox::exec` as a bash invocation.
//!
//! This is the only execution path. There is NO host-shell fallback.
//! When the sandbox is unavailable, errors bubble up unchanged.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::bash::exec";
pub const DESCRIPTION: &str =
    "Run a bash command inside the sandbox. Args: command, timeout_ms?, env?, workdir?, stdin?.";
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const TRIGGER_TIMEOUT_MS: u64 = 35_000;
pub const MAX_OUTPUT_BYTES: usize = 30_000;

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: command".into()))?
        .to_string();
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    let mut payload = json!({
        "sandbox_id": sandbox_id,
        "cmd": "bash",
        "args": ["-lc", command],
        "timeout_ms": timeout_ms,
    });
    if let Some(env) = args.get("env") {
        payload["env"] = env.clone();
    }
    if let Some(workdir) = args.get("workdir") {
        payload["workdir"] = workdir.clone();
    }
    if let Some(stdin) = args.get("stdin") {
        payload["stdin"] = stdin.clone();
    }
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::exec".into(),
            payload,
            action: None,
            timeout_ms: Some(TRIGGER_TIMEOUT_MS),
        })
        .await;
    Ok(match resp {
        Ok(v) => render(&v),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::exec failed: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }),
    })
}

fn render(v: &Value) -> Value {
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
    let truncated: String = text.chars().take(MAX_OUTPUT_BYTES).collect();
    json!({
        "content": [{ "type": "text", "text": truncated }],
        "details": {
            "exit_code": exit,
            "via": "iii-sandbox",
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
        let v = render(&json!({ "stdout": "hi\n", "stderr": "", "exit_code": 0 }));
        let text = v["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.starts_with("exit=0\nhi"));
    }
    #[test]
    fn render_appends_stderr() {
        let v = render(&json!({ "stdout": "ok", "stderr": "warn", "exit_code": 1 }));
        let text = v["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("ok\nwarn"));
        assert!(text.starts_with("exit=1"));
    }
}
