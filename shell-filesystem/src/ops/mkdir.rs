//! `shell::filesystem::mkdir` — create a directory on the host filesystem.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::mkdir";
pub const DESCRIPTION: &str = "Create a directory on the host filesystem. Args: path, parents?.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: path".into()))?
        .to_string();
    let parents = args.get("parents").and_then(Value::as_bool).unwrap_or(true);

    let result = if parents {
        tokio::fs::create_dir_all(&path).await
    } else {
        tokio::fs::create_dir(&path).await
    };

    Ok(match result {
        Ok(()) => json!({
            "content": [{ "type": "text", "text": format!("created {}", path) }],
            "details": {},
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("mkdir {path}: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_uses_double_colon_namespace() {
        assert_eq!(ID, "shell::filesystem::mkdir");
    }
}
