use std::sync::Arc;

use serde_json::Value;

use crate::config::ShellConfig;
use crate::exec::{parse_argv, run_to_completion};
use crate::functions::types::ExecResponse;

pub async fn handle(cfg: Arc<ShellConfig>, payload: Value) -> Result<ExecResponse, String> {
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'command'".to_string())?;
    // Reject non-string elements rather than silently filtering: passing
    // `{"args": ["--count", 5]}` would otherwise drop the `5` and run with
    // partial arguments, a dangerous-to-debug deviation from caller intent.
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
    // `as_u64()` returns None for negative or float values, so an
    // unparseable `timeout_ms` silently falls back to the default. This
    // loose semantic is part of the wire contract (covered by e2e).
    let timeout_ms = payload.get("timeout_ms").and_then(|v| v.as_u64());

    let argv = parse_argv(command, args.as_ref()).map_err(|e| format!("argv: {}", e))?;

    cfg.is_command_allowed(&argv)?;

    let timeout = cfg.resolve_timeout(timeout_ms);

    let out = run_to_completion(&argv, &cfg, timeout)
        .await
        .map_err(|e| format!("exec: {}", e))?;

    Ok(ExecResponse {
        exit_code: out.exit_code,
        stdout: out.stdout,
        stderr: out.stderr,
        duration_ms: out.duration_ms,
        timed_out: out.timed_out,
        stdout_truncated: out.stdout_truncated,
        stderr_truncated: out.stderr_truncated,
    })
}
