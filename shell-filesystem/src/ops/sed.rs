//! `shell::filesystem::sed` — wrap `sandbox::fs::sed`.

use iii_sdk::{IIIError, TriggerRequest, Value, III};
use serde_json::json;

use sandbox_helpers::resolve_sandbox_id;

pub const ID: &str = "shell::filesystem::sed";
pub const DESCRIPTION: &str =
    "Find and replace inside the sandbox. Args: pattern, replacement, files OR path, plus filters.";

pub async fn execute(iii: &III, args: &Value) -> Result<Value, IIIError> {
    let sandbox_id = match resolve_sandbox_id(iii, args).await {
        Ok(id) => id,
        Err(e) => {
            return Ok(serde_json::to_value(e.to_tool_result())
                .expect("ToolResult is always serializable"))
        }
    };
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: pattern".into()))?
        .to_string();
    let replacement = args
        .get("replacement")
        .and_then(Value::as_str)
        .ok_or_else(|| IIIError::Handler("missing required arg: replacement".into()))?
        .to_string();

    let mut payload = json!({
        "sandbox_id": sandbox_id,
        "pattern": pattern,
        "replacement": replacement,
    });
    let has_files = args.get("files").is_some();
    let has_path = args.get("path").is_some();
    if has_files == has_path {
        return Ok(json!({
            "content": [{ "type": "text", "text": "must pass exactly one of `files` or `path`" }],
            "details": { "error": "S210" },
            "terminate": false,
        }));
    }
    for key in [
        "files",
        "path",
        "recursive",
        "include_glob",
        "exclude_glob",
        "regex",
        "first_only",
        "ignore_case",
    ] {
        if let Some(v) = args.get(key) {
            payload[key] = v.clone();
        }
    }
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::fs::sed".into(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await;
    Ok(match resp {
        Ok(v) => {
            let total = v
                .get("total_replacements")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            json!({
                "content": [{ "type": "text", "text": format!("{} replacements", total) }],
                "details": v,
                "terminate": false,
            })
        }
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("sandbox::fs::sed failed: {e}") }],
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
        assert_eq!(ID, "shell::filesystem::sed");
    }
}
