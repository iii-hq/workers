//! `shell::filesystem::stat` — stat a path on the host filesystem.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;

pub const ID: &str = "shell::filesystem::stat";
pub const DESCRIPTION: &str = "Stat a file or directory on the host filesystem.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| IIIError::Handler("missing required arg: path".into()))?
        .to_string();

    match tokio::fs::metadata(&path).await {
        Ok(m) => {
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            let v = json!({
                "name": name,
                "is_dir": m.is_dir(),
                "size": m.len(),
            });
            Ok(json!({
                "content": [{ "type": "text", "text": render_stat(&v) }],
                "details": v,
                "terminate": false,
            }))
        }
        Err(e) => Ok(json!({
            "content": [{ "type": "text", "text": format!("stat {path}: {e}") }],
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
