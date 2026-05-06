//! `shell::filesystem::write` — write a file directly to the host filesystem.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::write";
pub const DESCRIPTION: &str =
    "Write a file on the host filesystem. Args: path, content (utf-8) or content_b64, parents?.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: path".into()))?
        .to_string();
    let bytes = decode_content(args)?;
    let parents = args.get("parents").and_then(Value::as_bool).unwrap_or(true);

    if parents {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Ok(json!({
                        "content": [{ "type": "text", "text": format!("mkdir parents for {path}: {e}") }],
                        "details": { "error": e.to_string() },
                        "terminate": false,
                    }));
                }
            }
        }
    }

    Ok(match tokio::fs::write(&path, &bytes).await {
        Ok(()) => json!({
            "content": [{ "type": "text", "text": format!("wrote {} bytes to {}", bytes.len(), path) }],
            "details": { "bytes": bytes.len() },
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("write {path}: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }),
    })
}

fn decode_content(args: &Value) -> Result<Vec<u8>, IIIError> {
    if let Some(s) = args.get("content").and_then(Value::as_str) {
        return Ok(s.as_bytes().to_vec());
    }
    if let Some(s) = args.get("content_b64").and_then(Value::as_str) {
        return B64
            .decode(s)
            .map_err(|e| IIIError::Handler(format!("invalid content_b64: {e}")));
    }
    Err(IIIError::Handler(
        "missing required arg: content or content_b64".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_content_prefers_utf8() {
        let v = json!({ "content": "hi" });
        assert_eq!(decode_content(&v).unwrap(), b"hi");
    }

    #[test]
    fn decode_content_falls_back_to_b64() {
        let v = json!({ "content_b64": B64.encode("zz") });
        assert_eq!(decode_content(&v).unwrap(), b"zz");
    }

    #[test]
    fn decode_content_errors_when_missing() {
        let v = json!({});
        assert!(decode_content(&v).is_err());
    }
}
