//! `shell::filesystem::mv` — wrap `sandbox::fs::mv`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::mv";
pub const DESCRIPTION: &str = "Move/rename a path inside the sandbox. Args: src, dst, overwrite?.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let src = required(args, "src")?;
    let dst = required(args, "dst")?;
    let overwrite = args
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::mv".into(),
            payload: json!({
                "sandbox_id": sandbox_id,
                "src": src,
                "dst": dst,
                "overwrite": overwrite,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
    Ok(match resp {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": format!("moved {} -> {}", src, dst) }],
            "details": v,
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::mv failed: {e}") }],
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
