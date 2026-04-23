use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::config::ShellConfig;
use crate::exec::{parse_argv, run_to_completion};

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
    // Validate strictly: if `args` is present, it must be an array of strings.
    // Silently dropping non-strings with filter_map meant a caller sending
    // `{"args": ["--count", 5]}` would have `5` quietly removed and the shell
    // would then run with partial arguments — a subtle, dangerous-to-debug
    // deviation from what the caller asked for.
    let args: Option<Vec<String>> = match payload.get("args") {
        None | Some(Value::Null) => None,
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                match v.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(IIIError::Handler(format!(
                            "'args[{}]' must be a string (got {})",
                            i, v
                        )));
                    }
                }
            }
            Some(out)
        }
        Some(other) => {
            return Err(IIIError::Handler(format!(
                "'args' must be an array of strings (got {})",
                other
            )));
        }
    };
    let timeout_ms = payload.get("timeout_ms").and_then(|v| v.as_u64());

    let argv = parse_argv(command, args.as_ref())
        .map_err(|e| IIIError::Handler(format!("argv: {}", e)))?;

    cfg.is_command_allowed(&argv).map_err(IIIError::Handler)?;

    let timeout = cfg.resolve_timeout(timeout_ms);

    let out = run_to_completion(&argv, &cfg, timeout)
        .await
        .map_err(|e| IIIError::Handler(format!("exec: {}", e)))?;

    Ok(json!({
        "exit_code": out.exit_code,
        "stdout": out.stdout,
        "stderr": out.stderr,
        "duration_ms": out.duration_ms,
        "timed_out": out.timed_out,
        "stdout_truncated": out.stdout_truncated,
        "stderr_truncated": out.stderr_truncated,
    }))
}
