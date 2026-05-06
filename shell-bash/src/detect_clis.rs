//! `shell::bash::detect_clis` — probe a fixed CLI set on the host.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;

pub const ID: &str = "shell::bash::detect_clis";
pub const DESCRIPTION: &str =
    "Probe a fixed set of CLIs on the host and report which are installed.";

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

pub async fn execute(_iii: &III, _args: &Value) -> Result<Value, IIIError> {
    let mut script = String::with_capacity(PROBED.len() * 24);
    for name in PROBED {
        script.push_str(&format!("command -v {name} >/dev/null && echo {name}\n"));
    }
    let child = tokio::process::Command::new("bash")
        .arg("-lc")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| IIIError::Handler(format!("spawn bash: {e}")))?;
    let result = tokio::time::timeout(Duration::from_millis(5_000), child.wait_with_output())
        .await
        .map_err(|_| IIIError::Handler("detect_clis timed out".into()))?
        .map_err(|e| IIIError::Handler(format!("wait bash: {e}")))?;
    let stdout = String::from_utf8_lossy(&result.stdout);
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
