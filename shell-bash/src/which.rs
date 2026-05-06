//! `shell::bash::which` — runs `command -v <name>` inside the sandbox.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::bash::which";
pub const DESCRIPTION: &str = "Resolve a CLI name to its absolute path inside the sandbox.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: name".into()))?
        .to_string();
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::exec".into(),
            payload: json!({
                "sandbox_id": sandbox_id,
                "cmd": "bash",
                "args": ["-lc", format!("command -v {}", shell_escape(&name))],
                "timeout_ms": 5_000,
            }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|e| IIIError::Handler(format!("sandbox::exec failed: {e}")))?;

    let exit = resp.get("exit_code").and_then(Value::as_i64).unwrap_or(-1);
    let stdout = resp.get("stdout").and_then(Value::as_str).unwrap_or("");
    let path = stdout.trim();
    Ok(json!({
        "content": [{ "type": "text", "text": path }],
        "details": { "found": exit == 0, "exit_code": exit, "name": name },
        "terminate": false,
    }))
}

fn shell_escape(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        name.into()
    } else {
        format!("'{}'", name.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_escape_simple_passthrough() {
        assert_eq!(shell_escape("python3"), "python3");
        assert_eq!(shell_escape("/usr/bin/env"), "/usr/bin/env");
    }
    #[test]
    fn shell_escape_quotes_special_chars() {
        assert_eq!(shell_escape("a b"), "'a b'");
        assert_eq!(shell_escape("a'b"), "'a'\\''b'");
    }
}
