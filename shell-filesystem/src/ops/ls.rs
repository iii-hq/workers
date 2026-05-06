//! `shell::filesystem::ls` — list a host directory directly.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::ls";
pub const DESCRIPTION: &str = "List entries in a directory on the host filesystem.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = required_str(args, "path")?;
    match read_entries(&path).await {
        Ok(entries) => {
            let names = format_entries_from(&entries);
            Ok(text_result(names, json!({ "entries": entries })))
        }
        Err(e) => Ok(text_result(
            format!("ls {path}: {e}"),
            json!({ "error": e.to_string() }),
        )),
    }
}

async fn read_entries(path: &str) -> std::io::Result<Vec<Value>> {
    let mut rd = tokio::fs::read_dir(path).await?;
    let mut out: Vec<Value> = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry
            .file_type()
            .await
            .map(|t| t.is_dir())
            .unwrap_or(false);
        let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
        out.push(json!({ "name": name, "is_dir": is_dir, "size": size }));
    }
    Ok(out)
}

fn required_str(args: &Value, key: &str) -> Result<String, IIIError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required arg: {key}")))
}

fn format_entries_from(entries: &[Value]) -> String {
    let mut names: Vec<String> = entries
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
        let entries = vec![
            json!({ "name": "b" }),
            json!({ "name": "a" }),
            json!({ "name": "c" }),
        ];
        assert_eq!(format_entries_from(&entries), "a\nb\nc");
    }

    #[test]
    fn format_entries_handles_empty() {
        assert_eq!(format_entries_from(&[]), "");
    }
}
