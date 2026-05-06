//! `shell::filesystem::grep` — spawn `grep` directly on the host filesystem.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;
use std::process::Stdio;

pub const ID: &str = "shell::filesystem::grep";
pub const DESCRIPTION: &str =
    "Recursive regex search on the host filesystem. Args: path, pattern, recursive?, ignore_case?, include_glob?, exclude_glob?, max_matches?, max_line_bytes?.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = required(args, "path")?;
    let pattern = required(args, "pattern")?;

    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let ignore_case = args
        .get("ignore_case")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_matches = args.get("max_matches").and_then(Value::as_u64);
    let include_glob = args
        .get("include_glob")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let exclude_glob = args
        .get("exclude_glob")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let max_line_bytes = args.get("max_line_bytes").and_then(Value::as_u64);

    let mut cmd = tokio::process::Command::new("grep");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    // Always use -n for line numbers; -E for extended regex.
    cmd.arg("-En");

    if recursive {
        cmd.arg("-r");
    }
    if ignore_case {
        cmd.arg("-i");
    }
    if let Some(n) = max_matches {
        cmd.arg(format!("-m{n}"));
    }
    if let Some(ref pat) = include_glob {
        cmd.arg(format!("--include={pat}"));
    }
    if let Some(ref pat) = exclude_glob {
        cmd.arg(format!("--exclude={pat}"));
    }
    cmd.arg("--").arg(&pattern).arg(&path);

    let output = cmd
        .output()
        .await
        .map_err(|e| IIIError::Handler(format!("grep failed: {e}")))?;

    // grep exits 1 when no matches, 2 on error — both are acceptable here.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut matches: Vec<Value> = Vec::new();

    for raw_line in stdout.lines() {
        // Format from grep -rn: path:line_no:rest
        // For non-recursive (single file), grep emits line_no:rest with no path prefix.
        let parts: Vec<&str> = raw_line.splitn(3, ':').collect();
        let (match_path, line_no, line) = if recursive || parts.len() == 3 {
            if parts.len() < 3 {
                continue;
            }
            let ln: u64 = parts[1].parse().unwrap_or(0);
            (parts[0].to_string(), ln, parts[2].to_string())
        } else {
            // Non-recursive: "line_no:text"
            if parts.len() < 2 {
                continue;
            }
            let ln: u64 = parts[0].parse().unwrap_or(0);
            (path.clone(), ln, parts[1].to_string())
        };

        let line = if let Some(max) = max_line_bytes {
            let max = max as usize;
            if line.len() > max {
                line[..max].to_string()
            } else {
                line
            }
        } else {
            line
        };

        matches.push(json!({ "path": match_path, "line_no": line_no, "line": line }));
    }

    let details = json!({ "matches": matches });
    Ok(json!({
        "content": [{ "type": "text", "text": render(&details) }],
        "details": details,
        "terminate": false,
    }))
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
