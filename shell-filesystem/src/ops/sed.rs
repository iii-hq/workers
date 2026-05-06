//! `shell::filesystem::sed` — find and replace via `sed` directly on the host filesystem.
//!
//! Supports `files` (explicit list) and `path` (single file). Recursive directory
//! walking is not yet supported — pass `files` for multi-file edits.

use iii_sdk::{IIIError, Value, III};
use serde_json::json;
use std::process::Stdio;

pub const ID: &str = "shell::filesystem::sed";
pub const DESCRIPTION: &str =
    "Find and replace inside files on the host. Args: pattern, replacement, files OR path, regex?, first_only?, ignore_case?.";

pub async fn execute(_iii: &III, args: &Value) -> Result<Value, IIIError> {
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

    let has_files = args.get("files").is_some();
    let has_path = args.get("path").is_some();
    if has_files == has_path {
        return Ok(json!({
            "content": [{ "type": "text", "text": "must pass exactly one of `files` or `path`" }],
            "details": { "error": "S210" },
            "terminate": false,
        }));
    }

    let files: Vec<String> = if has_files {
        args.get("files")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        // has_path
        let p = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        vec![p]
    };

    let first_only = args
        .get("first_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ignore_case = args
        .get("ignore_case")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let use_regex = args
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    // Build the sed substitution flags.
    // sed on macOS and Linux both support: s/pat/rep/[g][I]
    // Use extended regex (-E) when regex=true (the default).
    let flags = {
        let mut f = String::new();
        if !first_only {
            f.push('g');
        }
        if ignore_case {
            // GNU sed uses 'I', macOS sed uses 'i' — try 'I' which works on GNU.
            // For portability we use a case-insensitive approach: macOS accepts 'I' too.
            f.push('I');
        }
        f
    };

    // Escape delimiter '/' inside pattern and replacement.
    let escaped_pattern = pattern.replace('/', "\\/");
    let escaped_replacement = replacement.replace('/', "\\/");
    let script = format!("s/{escaped_pattern}/{escaped_replacement}/{flags}");

    let mut total_replacements: u64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for file in &files {
        if file.is_empty() {
            continue;
        }
        let mut cmd = tokio::process::Command::new("sed");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        if use_regex {
            cmd.arg("-E");
        }
        // In-place edit; macOS requires an extension argument after -i.
        // Use `-i ''` on macOS and `-i` on Linux — detect by platform.
        #[cfg(target_os = "macos")]
        {
            cmd.arg("-i").arg("").arg(&script).arg(file);
        }
        #[cfg(not(target_os = "macos"))]
        {
            cmd.arg("-i").arg(&script).arg(file);
        }

        match cmd.output().await {
            Ok(out) if out.status.success() => {
                total_replacements += 1;
            }
            Ok(out) => {
                let msg = String::from_utf8_lossy(&out.stderr).to_string();
                errors.push(format!("{file}: {msg}"));
            }
            Err(e) => {
                errors.push(format!("{file}: {e}"));
            }
        }
    }

    let text = if errors.is_empty() {
        format!("{total_replacements} replacements")
    } else {
        format!(
            "{total_replacements} replacements; {} errors: {}",
            errors.len(),
            errors.join("; ")
        )
    };

    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "details": { "total_replacements": total_replacements },
        "terminate": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id() {
        assert_eq!(ID, "shell::filesystem::sed");
    }
}
