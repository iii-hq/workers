//! `shell::filesystem::read` — wrap `sandbox::fs::read` and drain the
//! returned `StreamChannelRef`.

use iii_sdk::{IIIError, StreamChannelRef, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::{drain_ref, resolve_sandbox_id};

pub const ID: &str = "shell::filesystem::read";
pub const DESCRIPTION: &str =
    "Read a file inside the sandbox and return its UTF-8 contents (or base64 fallback).";
pub const MAX_INLINE_BYTES: usize = 256 * 1024;

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: path".into()))?
        .to_string();

    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::read".into(),
            payload: json!({ "sandbox_id": sandbox_id, "path": path }),
            action: None,
            timeout_ms: None,
        })
        .await;

    let resp = match resp {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({
                "content": [{ "type": "text", "text": format!("sandbox::fs::read failed: {e}") }],
                "details": { "error": e.to_string() },
                "terminate": false,
            }));
        }
    };

    let channel_ref = match resp.get("content").cloned() {
        Some(v) => match serde_json::from_value::<StreamChannelRef>(v) {
            Ok(r) => r,
            Err(e) => {
                return Ok(json!({
                    "content": [{ "type": "text", "text": format!("invalid channel ref: {e}") }],
                    "details": { "error": e.to_string() },
                    "terminate": false,
                }));
            }
        },
        None => {
            return Ok(json!({
                "content": [{ "type": "text", "text": "sandbox::fs::read returned no content channel" }],
                "details": resp,
                "terminate": false,
            }));
        }
    };

    let bytes = match drain_ref(iii, &channel_ref).await {
        Ok(b) => b,
        Err(e) => {
            return Ok(json!({
                "content": [{ "type": "text", "text": format!("channel drain failed: {e}") }],
                "details": { "error": e.to_string() },
                "terminate": false,
            }));
        }
    };

    let truncated = bytes.len() > MAX_INLINE_BYTES;
    let body = if truncated {
        bytes[..MAX_INLINE_BYTES].to_vec()
    } else {
        bytes.clone()
    };
    let text = match String::from_utf8(body.clone()) {
        Ok(s) => s,
        Err(_) => format!("<binary {} bytes>", body.len()),
    };
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "details": {
            "size": resp.get("size").cloned().unwrap_or(Value::Null),
            "mode": resp.get("mode").cloned().unwrap_or(Value::Null),
            "mtime": resp.get("mtime").cloned().unwrap_or(Value::Null),
            "truncated": truncated,
            "bytes_read": bytes.len(),
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
