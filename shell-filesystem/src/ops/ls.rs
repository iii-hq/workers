//! `shell::filesystem::ls` — wrap `sandbox::fs::ls`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::ls";
pub const DESCRIPTION: &str = "List directory entries inside the sandbox.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let path = required_str(args, "path")?;
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::ls".into(),
            payload: json!({ "sandbox_id": sandbox_id, "path": path }),
            action: None,
            timeout_ms: None,
        })
        .await;
    match resp {
        Ok(v) => Ok(text_result(format_entries(&v), v)),
        Err(e) => Ok(text_result(
            format!("sandbox::fs::ls failed: {e}"),
            json!({ "error": e.to_string() }),
        )),
    }
}

fn required_str(args: &Value, key: &str) -> Result<String, IIIError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required arg: {key}")))
}

fn format_entries(value: &Value) -> String {
    let entries = value.get("entries").and_then(Value::as_array);
    let Some(arr) = entries else {
        return String::new();
    };
    let mut names: Vec<String> = arr
        .iter()
        .filter_map(|e| {
            e.get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect();
    names.sort();
    names.join("\n")
}

fn text_result(text: String, details: Value) -> Value {
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
    fn format_entries_returns_sorted_names() {
        let v = json!({ "entries": [
            { "name": "b" }, { "name": "a" }, { "name": "c" }
        ]});
        assert_eq!(format_entries(&v), "a\nb\nc");
    }

    #[test]
    fn format_entries_handles_missing_field() {
        assert_eq!(format_entries(&json!({})), "");
        assert_eq!(format_entries(&json!({ "entries": [] })), "");
    }
}
