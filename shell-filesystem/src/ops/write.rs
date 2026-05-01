//! `shell::filesystem::write` — wrap `sandbox::fs::write` and fill its
//! content channel from a UTF-8 or base64 payload.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::{fill_ref, resolve_sandbox_id};

pub const ID: &str = "shell::filesystem::write";
pub const DESCRIPTION: &str =
    "Write a file inside the sandbox. Args: path, content (utf-8) or content_b64, mode?, parents?.";

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
    let bytes = decode_content(args)?;
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("0644");
    let parents = args
        .get("parents")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let channel = match iii.create_channel(None).await {
        Ok(ch) => ch,
        Err(e) => {
            return Ok(json!({
                "content": [{ "type": "text", "text": format!("create_channel failed: {e}") }],
                "details": { "error": e.to_string() },
                "terminate": false,
            }));
        }
    };

    let trigger_payload = json!({
        "sandbox_id": sandbox_id,
        "path": path,
        "mode": mode,
        "parents": parents,
        "content": channel.reader_ref,
    });

    let trigger_fut = iii.trigger(TriggerRequest {
        function_id: "sandbox::fs::write".into(),
        payload: trigger_payload,
        action: None,
        timeout_ms: None,
    });
    let fill_fut = fill_ref(iii, &channel.writer_ref, &bytes);

    let (trigger_res, fill_res) = tokio::join!(trigger_fut, fill_fut);

    if let Err(e) = fill_res {
        return Ok(json!({
            "content": [{ "type": "text", "text": format!("channel fill failed: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }));
    }

    Ok(match trigger_res {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": format!("wrote {} bytes to {}", bytes.len(), path) }],
            "details": v,
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::write failed: {e}") }],
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
