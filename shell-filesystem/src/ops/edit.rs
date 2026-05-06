//! `shell::filesystem::edit` — composite: read, replace once, write.
//!
//! Mirrors the legacy `edit` tool: fails when `old_string` matches zero or
//! more than one time so the model is forced to disambiguate.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::edit";
pub const DESCRIPTION: &str =
    "Replace the unique occurrence of `old_string` with `new_string` in a file.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = required(args, "path")?;
    let old = required(args, "old_string")?;
    let new = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| IIIError::Handler("missing required arg: new_string".into()))?
        .to_string();

    // Read.
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| IIIError::Handler(format!("read {path}: {e}")))?;

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
    match tokio::fs::write(&path, updated.as_bytes()).await {
        Ok(()) => Ok(text_result(&format!("edited {}", path), json!({ "matches": 1 }))),
        Err(e) => Ok(text_result(
            &format!("write {path}: {e}"),
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
