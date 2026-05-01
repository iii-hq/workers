//! `shell::filesystem::stat` — wrap `sandbox::fs::stat`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::stat";
pub const DESCRIPTION: &str = "Stat a path inside the sandbox.";

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
            function_id: "sandbox::fs::stat".into(),
            payload: json!({ "sandbox_id": sandbox_id, "path": path }),
            action: None,
            timeout_ms: None,
        })
        .await;
    match resp {
        Ok(v) => Ok(json!({
            "content": [{ "type": "text", "text": render_stat(&v) }],
            "details": v,
            "terminate": false,
        })),
        Err(e) => Ok(json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::stat failed: {e}") }],
            "details": { "error": e.to_string() },
            "terminate": false,
        })),
    }
}

fn render_stat(v: &Value) -> String {
    let name = v.get("name").and_then(Value::as_str).unwrap_or("?");
    let is_dir = v.get("is_dir").and_then(Value::as_bool).unwrap_or(false);
    let size = v.get("size").and_then(Value::as_u64).unwrap_or(0);
    format!("{name} dir={is_dir} size={size}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_stat_includes_dir_and_size() {
        let s = render_stat(&json!({ "name": "a.txt", "is_dir": false, "size": 12 }));
        assert!(s.contains("a.txt"));
        assert!(s.contains("dir=false"));
        assert!(s.contains("size=12"));
    }
}
