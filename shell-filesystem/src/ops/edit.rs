//! `shell::filesystem::edit` — composite: read, replace once, write.
//!
//! Mirrors the legacy `edit` tool: fails when `old_string` matches zero or
//! more than one time so the model is forced to disambiguate.

use iii_sdk::{IIIError, StreamChannelRef, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::{drain_ref, fill_ref, resolve_sandbox_id};

pub const ID: &str = "shell::filesystem::edit";
pub const DESCRIPTION: &str =
    "Replace the unique occurrence of `old_string` with `new_string` in a sandboxed file.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let path = required(args, "path")?;
    let old = required(args, "old_string")?;
    let new = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| IIIError::Handler("missing required arg: new_string".into()))?
        .to_string();

    // Read.
    let read = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::read".into(),
            payload: json!({ "sandbox_id": sandbox_id, "path": path }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(format!("sandbox::fs::read failed: {e}")))?;
    let channel_ref: StreamChannelRef = serde_json::from_value(
        read.get("content")
            .cloned()
            .ok_or_else(|| IIIError::Handler("read returned no channel".into()))?,
    )
    .map_err(|e| IIIError::Handler(e.to_string()))?;
    let bytes = drain_ref(iii, &channel_ref)
        .await
        .map_err(|e| IIIError::Handler(format!("drain failed: {e}")))?;
    let text =
        String::from_utf8(bytes).map_err(|_| IIIError::Handler("file is not utf-8".into()))?;

    let count = text.matches(&old).count();
    if count == 0 {
        return Ok(text_result("old_string not found", json!({ "matches": 0 })));
    }
    if count > 1 {
        return Ok(text_result(
            &format!("old_string matched {count} times; provide more context"),
            json!({ "matches": count }),
        ));
    }
    let updated = text.replacen(&old, &new, 1);

    // Write.
    let channel = iii
        .create_channel(None)
        .await
        .map_err(|e| IIIError::Handler(format!("create_channel failed: {e}")))?;
    let trigger_fut = iii.trigger(TriggerRequest {
        function_id: "sandbox::fs::write".into(),
        payload: json!({
            "sandbox_id": sandbox_id,
            "path": path,
            "mode": "0644",
            "parents": false,
            "content": channel.reader_ref,
        }),
        action: None,
        timeout_ms: None,
    });
    let fill_fut = fill_ref(iii, &channel.writer_ref, updated.as_bytes());
    let (trigger_res, fill_res) = tokio::join!(trigger_fut, fill_fut);
    if let Err(e) = fill_res {
        return Ok(text_result(
            &format!("channel fill failed: {e}"),
            json!({ "error": e.to_string() }),
        ));
    }
    match trigger_res {
        Ok(v) => Ok(text_result(&format!("edited {}", path), v)),
        Err(e) => Ok(text_result(
            &format!("sandbox::fs::write failed: {e}"),
            json!({ "error": e.to_string() }),
        )),
    }
}

fn required(args: &Value, key: &str) -> Result<String, IIIError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required arg: {key}")))
}

fn text_result(text: &str, details: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "details": details,
        "terminate": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id() {
        assert_eq!(ID, "shell::filesystem::edit");
    }
}
