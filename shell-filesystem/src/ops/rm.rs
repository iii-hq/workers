//! `shell::filesystem::rm` — remove a path directly on the host filesystem.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::rm";
pub const DESCRIPTION: &str = "Remove a path on the host filesystem. Args: path, recursive?.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: path".into()))?
        .to_string();
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let result = if recursive {
        tokio::fs::remove_dir_all(&path).await
    } else {
        // Stat to decide between file and directory removal.
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.is_dir() => tokio::fs::remove_dir(&path).await,
            Ok(_) => tokio::fs::remove_file(&path).await,
            Err(e) => Err(e),
        }
    };

    Ok(match result {
        Ok(()) => json!({
            "content": [{ "type": "text", "text": format!("removed {}", path) }],
            "details": { "path": path },
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("rm {path}: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id() {
        assert_eq!(ID, "shell::filesystem::rm");
    }
}
