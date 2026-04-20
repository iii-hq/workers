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
    let args: Option<Vec<String>> = payload
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect());
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(|v| v.as_u64());

    let argv = parse_argv(command, args.as_ref())
        .map_err(|e| IIIError::Handler(format!("argv: {}", e)))?;

    cfg.is_command_allowed(&argv)
        .map_err(|e| IIIError::Handler(e))?;

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
