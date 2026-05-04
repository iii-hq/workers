//! `shell::filesystem::chmod` — wrap `sandbox::fs::chmod`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::chmod";
pub const DESCRIPTION: &str =
    "Change permissions inside the sandbox. Args: path, mode, uid?, gid?, recursive?.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let path = required_str(args, "path")?;
    let mode = required_str(args, "mode")?;
    let mut payload = json!({
        "sandbox_id": sandbox_id,
        "path": path,
        "mode": mode,
        "recursive": args.get("recursive").and_then(Value::as_bool).unwrap_or(false),
    });
    if let Some(uid) = args.get("uid").and_then(Value::as_u64) {
        payload["uid"] = json!(uid);
    }
    if let Some(gid) = args.get("gid").and_then(Value::as_u64) {
        payload["gid"] = json!(gid);
    }
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::chmod".into(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await;
    Ok(match resp {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": format!("chmod {} ok", path) }],
            "details": v,
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::chmod failed: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        }),
    })
}

fn required_str(args: &Value, key: &str) -> Result<String, IIIError> {
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
        assert_eq!(ID, "shell::filesystem::chmod");
    }
}
