//! `coder::context` — one-call workspace snapshot for seeding a coding
//! session's environment block: effective allowed roots, platform, bounded
//! git state, and repo instruction files (AGENTS.md / CLAUDE.md). Read-only.
//! Git state comes from running the `git` binary with the primary root as
//! cwd; a missing binary, a timeout, or a non-repo root yields `git: null`
//! — never an error, so callers can always render whatever came back.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::path::PathResolver;

/// Maximum porcelain status entries reported before truncation.
const STATUS_MAX_ENTRIES: usize = 50;
/// Number of recent commits reported (`git log --oneline`).
const RECENT_COMMITS: usize = 5;
/// Per-file byte cap for instruction-file content.
const INSTRUCTION_MAX_BYTES: usize = 16 * 1024;
/// Instruction files probed at the primary root, in report order.
const INSTRUCTION_FILE_NAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
/// Wall-clock budget per git invocation.
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// No caller-visible arguments — `coder::context` is a pure discovery call.
// examples are wire-contract; goldens pin them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(example = "example_context_input")]
pub struct ContextInput {
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_context_input() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ContextOutput {
    /// The primary allowed root — relative paths in every `coder::*` call
    /// resolve against this directory.
    pub primary_root: String,
    /// Canonical absolute paths of all allowed roots, primary first.
    pub base_paths: Vec<String>,
    pub platform: Platform,
    /// Git state of the primary root; `null` when the root is not inside a
    /// git work tree or the `git` binary is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<GitContext>,
    /// Repo instruction files (AGENTS.md, CLAUDE.md) found at the primary
    /// root, in probe order. Content is capped per file; `truncated` marks
    /// a capped entry.
    pub instruction_files: Vec<InstructionFile>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Platform {
    /// Operating system (`std::env::consts::OS`, e.g. "macos", "linux").
    pub os: String,
    /// CPU architecture (`std::env::consts::ARCH`, e.g. "aarch64").
    pub arch: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GitContext {
    /// Current branch name; `"HEAD"` when detached.
    pub branch: String,
    /// `git status --porcelain` lines, capped at 50 entries.
    pub status: Vec<String>,
    /// True when the status list was capped.
    pub status_truncated: bool,
    /// Up to the last 5 commits, `git log --oneline` format.
    pub recent_commits: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InstructionFile {
    /// Root-relative file name (e.g. "AGENTS.md").
    pub path: String,
    /// File content, capped at 16 KiB (char-boundary safe).
    pub content: String,
    /// True when `content` was capped.
    pub truncated: bool,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    _cfg: Arc<CoderConfig>,
    _req: ContextInput,
) -> Result<ContextOutput, String> {
    let primary = resolver.base_root().to_path_buf();
    let git = git_context(&primary).await;
    let resolver_for_files = resolver.clone();
    let instruction_files =
        tokio::task::spawn_blocking(move || read_instruction_files(&resolver_for_files))
            .await
            .map_err(|e| format!("context task join failed: {e}"))?;

    Ok(ContextOutput {
        primary_root: primary.display().to_string(),
        base_paths: resolver
            .roots()
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        platform: Platform {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
        git,
        instruction_files,
    })
}

/// Run one bounded git command; `None` on spawn failure, timeout, or
/// non-zero exit (all equivalent to "no git state" for this snapshot).
async fn git_lines(root: &Path, args: &[&str]) -> Option<Vec<String>> {
    let fut = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output();
    let out = tokio::time::timeout(GIT_TIMEOUT, fut).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

async fn git_context(root: &Path) -> Option<GitContext> {
    // rev-parse doubles as the is-a-repo probe: any failure → not a repo
    // (or no usable git), and the whole block is omitted.
    let branch = git_lines(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await?
        .into_iter()
        .next()?;
    let mut status = git_lines(root, &["status", "--porcelain"])
        .await
        .unwrap_or_default();
    let status_truncated = status.len() > STATUS_MAX_ENTRIES;
    status.truncate(STATUS_MAX_ENTRIES);
    // Fails on a repo with no commits yet — an empty history, not an error.
    let recent_commits = git_lines(root, &["log", "--oneline", &format!("-{RECENT_COMMITS}")])
        .await
        .unwrap_or_default();
    Some(GitContext {
        branch,
        status,
        status_truncated,
        recent_commits,
    })
}

fn read_instruction_files(resolver: &PathResolver) -> Vec<InstructionFile> {
    let mut out = Vec::new();
    for name in INSTRUCTION_FILE_NAMES {
        // Resolve through the jail so non-accessible globs keep applying.
        let Ok(abs) = resolver.resolve(name) else {
            continue;
        };
        if resolver.is_non_accessible(&abs) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        let full = String::from_utf8_lossy(&bytes);
        let truncated = full.len() > INSTRUCTION_MAX_BYTES;
        let content = if truncated {
            let mut end = INSTRUCTION_MAX_BYTES;
            while !full.is_char_boundary(end) {
                end -= 1;
            }
            full[..end].to_string()
        } else {
            full.into_owned()
        };
        out.push(InstructionFile {
            path: name.to_string(),
            content,
            truncated,
        });
    }
    out
}
