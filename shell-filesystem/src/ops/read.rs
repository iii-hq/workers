//! `shell::filesystem::read` — read a host file directly.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::read";
pub const DESCRIPTION: &str =
    "Read a file from the host filesystem and return its UTF-8 contents (or a binary marker).";
pub const MAX_INLINE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadConfig {
    pub max_inline_bytes: usize,
}

impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            max_inline_bytes: MAX_INLINE_BYTES,
        }
    }
}

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    execute_with_config(iii, args, ReadConfig::default()).await
}

pub async fn execute_with_config(
    _iii: &III,
    args: &Value,
    config: ReadConfig,
) -> Result<Value, IIIError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: path".into()))?
        .to_string();

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return Ok(json!({
                "content": [{ "type": "text", "text": format!("read {path}: {e}") }],
                "details": { "error": e.to_string() },
                "terminate": false,
            }));
        }
    };

    let total = bytes.len();
    let truncated = total > config.max_inline_bytes;
    let body = if truncated {
        bytes[..config.max_inline_bytes].to_vec()
    } else {
        bytes
    };
    let text = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => format!("<binary {} bytes>", body.len()),
    };

    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "details": {
            "size": total,
            "truncated": truncated,
            "bytes_read": body.len(),
        },
        "terminate": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_namespace() {
        assert_eq!(ID, "shell::filesystem::read");
    }

    #[test]
    fn cap_is_explicit() {
        assert_eq!(MAX_INLINE_BYTES, 256 * 1024);
    }
}
