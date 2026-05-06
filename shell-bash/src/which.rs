//! `shell::bash::which` — runs `command -v <name>` directly on the host.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;

pub const ID: &str = "shell::bash::which";
pub const DESCRIPTION: &str = "Resolve a CLI name to its absolute path on the host.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: name".into()))?
        .to_string();
    let script = format!("command -v {}", shell_escape(&name));
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
        .map_err(|_| IIIError::Handler("which timed out".into()))?
        .map_err(|e| IIIError::Handler(format!("wait bash: {e}")))?;
    let exit = result.status.code().unwrap_or(-1) as i64;
    let stdout = String::from_utf8_lossy(&result.stdout);
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
