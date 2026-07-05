//! Post-write checks: report-only diagnostics run after coder writes
//! (`post_write_checks` in `CoderConfig`). Each configured check whose
//! glob matches a written file runs ONCE per call (deduplicated by
//! command) with the effective root as cwd; output is bounded and
//! attached to the write response. A check can fail, time out, or be
//! unrunnable — none of that fails the edit that triggered it.

use std::path::Path;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;

use crate::code::config::{CoderConfig, PostWriteCheck};
use crate::code::path::PathResolver;

/// Byte cap on a check's captured output (stdout + stderr merged).
const CHECK_OUTPUT_MAX_BYTES: usize = 4 * 1024;

/// One executed check's outcome, attached to the write response.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CheckOutcome {
    /// The configured command, verbatim.
    pub command: String,
    /// Process exit code; absent when the check timed out or failed to
    /// spawn (see `error`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Merged stdout + stderr, capped at 4 KiB (char-boundary safe).
    pub output: String,
    /// True when `output` was capped.
    pub truncated: bool,
    /// Timeout / spawn failure note; the edit itself already succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Run every configured check matching one of `written` (canonical
/// absolute paths of files this call wrote/deleted). Never errors.
pub async fn run_post_write_checks(
    cfg: &CoderConfig,
    resolver: &PathResolver,
    root: &Path,
    written: &[String],
) -> Vec<CheckOutcome> {
    if cfg.post_write_checks.is_empty() || written.is_empty() {
        return Vec::new();
    }
    let rels: Vec<String> = written
        .iter()
        .filter_map(|p| resolver.relative(Path::new(p)))
        .collect();

    let mut seen_commands: Vec<&str> = Vec::new();
    let mut outcomes = Vec::new();
    for check in &cfg.post_write_checks {
        if seen_commands.contains(&check.command.as_str()) {
            continue;
        }
        if !matches_any(&check.match_glob, &rels) {
            continue;
        }
        seen_commands.push(&check.command);
        outcomes.push(run_one(check, root).await);
    }
    outcomes
}

fn matches_any(glob: &str, rels: &[String]) -> bool {
    let Ok(compiled) = globset::Glob::new(glob).map(|g| g.compile_matcher()) else {
        tracing::warn!(glob = %glob, "ignoring invalid post_write_checks glob");
        return false;
    };
    rels.iter().any(|r| compiled.is_match(r))
}

async fn run_one(check: &PostWriteCheck, root: &Path) -> CheckOutcome {
    let fut = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&check.command)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .output();
    match tokio::time::timeout(Duration::from_millis(check.timeout_ms), fut).await {
        Err(_) => CheckOutcome {
            command: check.command.clone(),
            exit_code: None,
            output: String::new(),
            truncated: false,
            error: Some(format!("check timed out after {}ms", check.timeout_ms)),
        },
        Ok(Err(e)) => CheckOutcome {
            command: check.command.clone(),
            exit_code: None,
            output: String::new(),
            truncated: false,
            error: Some(format!("check failed to run: {e}")),
        },
        Ok(Ok(out)) => {
            let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.trim().is_empty() {
                if !merged.is_empty() {
                    merged.push('\n');
                }
                merged.push_str(&stderr);
            }
            let truncated = merged.len() > CHECK_OUTPUT_MAX_BYTES;
            if truncated {
                let mut end = CHECK_OUTPUT_MAX_BYTES;
                while !merged.is_char_boundary(end) {
                    end -= 1;
                }
                merged.truncate(end);
            }
            CheckOutcome {
                command: check.command.clone(),
                exit_code: out.status.code(),
                output: merged,
                truncated,
                error: None,
            }
        }
    }
}
