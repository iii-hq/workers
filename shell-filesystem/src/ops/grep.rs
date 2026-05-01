//! `shell::filesystem::grep` — wrap `sandbox::fs::grep`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::grep";
pub const DESCRIPTION: &str =
    "Recursive regex search inside the sandbox. Args: path, pattern, recursive?, ignore_case?, include_glob?, exclude_glob?, max_matches?, max_line_bytes?.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let path = required(args, "path")?;
    let pattern = required(args, "pattern")?;
    let mut payload = json!({
        "sandbox_id": sandbox_id,
        "path": path,
        "pattern": pattern,
    });
    for key in [
        "recursive",
        "ignore_case",
        "include_glob",
        "exclude_glob",
        "max_matches",
        "max_line_bytes",
    ] {
        if let Some(v) = args.get(key) {
            payload[key] = v.clone();
        }
    }
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::grep".into(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await;
    Ok(match resp {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": render(&v) }],
            "details": v,
            "terminate": false,
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::grep failed: {e}") }],
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

fn render(v: &Value) -> String {
    let Some(arr) = v.get("matches").and_then(Value::as_array) else {
        return String::new();
    };
    let mut lines = Vec::with_capacity(arr.len());
    for m in arr {
        let path = m.get("path").and_then(Value::as_str).unwrap_or("?");
        let line_no = m.get("line_no").and_then(Value::as_u64).unwrap_or(0);
        let line = m.get("line").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("{path}:{line_no}:{line}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_formats_path_line_text() {
        let v = json!({ "matches": [
            { "path": "/a", "line_no": 3, "line": "hi" },
            { "path": "/b", "line_no": 7, "line": "yo" },
        ]});
        assert_eq!(render(&v), "/a:3:hi\n/b:7:yo");
    }
}
