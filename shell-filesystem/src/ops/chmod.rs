//! `shell::filesystem::chmod` — change permissions directly on the host filesystem.
//!
//! `uid`/`gid` ownership changes are not supported (require `nix`/`libc::chown`
//! which are not in scope); those args are silently ignored.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

pub const ID: &str = "shell::filesystem::chmod";
pub const DESCRIPTION: &str =
    "Change permissions on the host filesystem. Args: path, mode (octal string), recursive?. uid/gid are accepted but ignored.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
    let path = required_str(args, "path")?;
    let mode_str = required_str(args, "mode")?;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Parse mode as octal (e.g. "0644", "644", "755").
    let mode = u32::from_str_radix(mode_str.trim_start_matches('0'), 8)
        .or_else(|_| u32::from_str_radix(&mode_str, 8))
        .map_err(|_| IIIError::Handler(format!("invalid mode: {mode_str}")))?;

    if recursive {
        // Walk directory tree and apply to every entry plus the root.
        let mut stack: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(&path)];
        let mut errors: Vec<String> = Vec::new();

        while let Some(p) = stack.pop() {
            match apply_mode(&p, mode).await {
                Ok(()) => {}
                Err(e) => errors.push(e),
            }
            // If it's a directory, enqueue children.
            if let Ok(mut rd) = tokio::fs::read_dir(&p).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    stack.push(entry.path());
                }
            }
        }

        if errors.is_empty() {
            Ok(json!({
                "content": [{ "type": "text", "text": format!("chmod {} ok", path) }],
                "details": { "path": path, "mode": mode_str },
                "terminate": false,
            }))
        } else {
            Ok(json!({
                "content": [{ "type": "text", "text": format!("chmod {} partial: {}", path, errors.join("; ")) }],
                "details": { "errors": errors },
                "terminate": false,
            }))
        }
    } else {
        match apply_mode(std::path::Path::new(&path), mode).await {
            Ok(()) => Ok(json!({
                "content": [{ "type": "text", "text": format!("chmod {} ok", path) }],
                "details": { "path": path, "mode": mode_str },
                "terminate": false,
            })),
            Err(e) => Ok(json!({
                "content": [{ "type": "text", "text": format!("chmod {path}: {e}") }],
                "details": { "error": e },
                "terminate": false,
            })),
        }
    }
}

async fn apply_mode(path: &std::path::Path, mode: u32) -> Result<(), String> {
    let perms = std::fs::Permissions::from_mode(mode);
    tokio::fs::set_permissions(path, perms)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn required_str(args: &Value, key: &str) -> Result<String, IIIError> {
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
        assert_eq!(ID, "shell::filesystem::chmod");
    }
}
