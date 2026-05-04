//! `shell::filesystem::rm` — wrap `sandbox::fs::rm`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::rm";
pub const DESCRIPTION: &str = "Remove a path inside the sandbox. Args: path, recursive?.";

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
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::rm".into(),
            payload: json!({
                "sandbox_id": sandbox_id,
                "path": path,
                "recursive": recursive,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
    Ok(match resp {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": format!("removed {}", path) }],
            "details": v,
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::rm failed: {e}") }],
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
