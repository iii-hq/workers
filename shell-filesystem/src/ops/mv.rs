//! `shell::filesystem::mv` — rename/move a path directly on the host filesystem.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::mv";
pub const DESCRIPTION: &str =
    "Move/rename a path on the host filesystem. Args: src, dst, overwrite?.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let src = required(args, "src")?;
    let dst = required(args, "dst")?;
    let overwrite = args
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !overwrite && tokio::fs::metadata(&dst).await.is_ok() {
        return Ok(json!({
            "content": [{ "type": "text", "text": format!("mv {src} -> {dst}: destination exists") }],
            "details": { "error": "destination exists" },
            "terminate": false,
        }));
    }

    Ok(match tokio::fs::rename(&src, &dst).await {
        Ok(()) => json!({
            "content": [{ "type": "text", "text": format!("moved {} -> {}", src, dst) }],
            "details": { "src": src, "dst": dst },
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("mv {src} -> {dst}: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }),
    })
}

fn required(args: &Value, key: &str) -> Result<String, IIIError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required arg: {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id() {
        assert_eq!(ID, "shell::filesystem::mv");
    }
}
