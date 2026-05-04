//! `shell::bash::detect_clis` — probe a fixed CLI set inside the sandbox.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::bash::detect_clis";
pub const DESCRIPTION: &str =
    "Probe a fixed set of CLIs inside the sandbox and report which are installed.";

pub const PROBED: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "openclaw",
    "hermes",
    "pi",
    "gemini",
    "cursor-agent",
];

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let mut script = String::with_capacity(PROBED.len() * 24);
    for name in PROBED {
        script.push_str(&format!("command -v {name} >/dev/null && echo {name}\n"));
    }
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::exec".into(),
            payload: json!({
                "sandbox_id": sandbox_id,
                "cmd": "bash",
                "args": ["-lc", script],
                "timeout_ms": 5_000,
            }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|e| IIIError::Handler(format!("sandbox::exec failed: {e}")))?;
    let stdout = resp.get("stdout").and_then(Value::as_str).unwrap_or("");
    let installed: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(ToString::to_string)
        .collect();
    Ok(json!({
        "content": [{ "type": "text", "text": installed.join("\n") }],
        "details": {
            "installed": installed,
            "probed": PROBED,
        },
        "terminate": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_list_is_canonical() {
        assert!(PROBED.contains(&"claude"));
        assert!(PROBED.contains(&"gemini"));
        assert_eq!(PROBED.len(), 8);
    }
}
