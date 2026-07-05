//! `coder::apply-patch` — apply a whole patch in the V4A "apply_patch"
//! format codex-family models are trained on (`*** Begin Patch` …
//! `*** End Patch`). All-or-nothing: every hunk is resolved, read, and
//! computed BEFORE anything is written, so a bad context match or a jail
//! rejection fails the whole call (C210/C211/C215) with the filesystem
//! byte-identical. Writes then land per-file via sibling-temp + rename.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::functions::update_file::atomic_write;
use crate::code::patch::{self, Hunk};
use crate::code::path::PathResolver;

/// Lines echoed around the first changed region of a modified file.
const ECHO_CONTEXT: u64 = 2;
/// Cap on echoed lines per modified file.
const ECHO_MAX_LINES: usize = 20;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_apply_patch_input")]
pub struct ApplyPatchInput {
    /// The full patch text in the apply_patch format: starts with
    /// `*** Begin Patch`, ends with `*** End Patch`, with one hunk per
    /// file (`*** Add File: `, `*** Delete File: `, or `*** Update File: `
    /// with an optional `*** Move to: `). Paths are relative to the
    /// primary allowed root, or absolute inside any allowed root.
    pub patch: String,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_apply_patch_input() -> serde_json::Value {
    serde_json::json!({
        "patch": "*** Begin Patch\n*** Update File: src/calc.py\n@@ def add(a, b):\n-    return a - b\n+    return a + b\n*** End Patch"
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApplyPatchOutput {
    /// One entry per file hunk, in patch order. The whole patch applied —
    /// a failure anywhere returns an error with nothing written.
    pub results: Vec<PatchFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PatchFileResult {
    /// Canonical absolute path of the affected file (the destination for
    /// a moved file).
    pub path: String,
    /// What happened: "added" | "modified" | "deleted" | "moved".
    pub kind: String,
    /// Line count after applying (absent for deletions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line_count: Option<u64>,
    /// Bounded post-apply snapshot of the first changed region (modified
    /// files only) — verify from this instead of re-reading the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<PatchEcho>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PatchEcho {
    /// 1-based line number of the first echoed line, post-apply.
    pub from_line: u64,
    pub lines: Vec<String>,
}

/// One fully-planned write, computed before any filesystem mutation.
enum PlannedWrite {
    Add {
        abs: PathBuf,
        contents: String,
    },
    Delete {
        abs: PathBuf,
    },
    Update {
        abs: PathBuf,
        move_to: Option<PathBuf>,
        new_contents: String,
        first_change: Option<(u64, u64)>,
    },
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: ApplyPatchInput,
) -> Result<ApplyPatchOutput, String> {
    let scope_root = crate::fs::scope_root(req.fs_scope.as_ref()).map(str::to_string);
    tokio::task::spawn_blocking(move || {
        inner(&resolver, &cfg, scope_root.as_deref(), &req.patch).map_err(err_to_string)
    })
    .await
    .map_err(|e| format!("apply-patch task join failed: {e}"))?
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    scope_root: Option<&str>,
    patch_text: &str,
) -> Result<ApplyPatchOutput, CoderError> {
    let hunks = patch::parse_patch(patch_text)
        .map_err(|e| CoderError::BadInput(format!("invalid patch: {e}")))?;
    if hunks.is_empty() {
        return Err(CoderError::BadInput(
            "patch contains no hunks — nothing between '*** Begin Patch' and '*** End Patch'"
                .into(),
        ));
    }

    // Plan phase: resolve every path through the jail, read + compute every
    // update, and validate every precondition BEFORE any write.
    let mut planned: Vec<PlannedWrite> = Vec::with_capacity(hunks.len());
    for hunk in &hunks {
        match hunk {
            Hunk::AddFile { path, contents } => {
                let wire = path.display().to_string();
                let abs = resolver.require_writable_opt(scope_root, &wire)?;
                if abs.exists() {
                    return Err(CoderError::BadInput(format!(
                        "add file target already exists: {wire} — use an \
                         '*** Update File: ' hunk to modify it"
                    )));
                }
                check_write_size(cfg, &wire, contents.len())?;
                planned.push(PlannedWrite::Add {
                    abs,
                    contents: contents.clone(),
                });
            }
            Hunk::DeleteFile { path } => {
                let wire = path.display().to_string();
                let abs = resolver.require_writable_opt(scope_root, &wire)?;
                let md = std::fs::metadata(&abs).map_err(|e| CoderError::io_for_path(e, &wire))?;
                if !md.is_file() {
                    return Err(CoderError::BadInput(format!(
                        "delete file target is not a regular file: {wire}"
                    )));
                }
                planned.push(PlannedWrite::Delete { abs });
            }
            Hunk::UpdateFile {
                path,
                move_path,
                chunks,
            } => {
                let wire = path.display().to_string();
                let abs = resolver.require_writable_opt(scope_root, &wire)?;
                let bytes = std::fs::read(&abs).map_err(|e| CoderError::io_for_path(e, &wire))?;
                let original = String::from_utf8_lossy(&bytes);
                let applied = patch::derive_new_contents_from_chunks(&original, &wire, chunks)
                    .map_err(|e| CoderError::BadInput(e.0))?;
                check_write_size(cfg, &wire, applied.new_contents.len())?;
                let move_to = match move_path {
                    Some(dest) => {
                        let dest_wire = dest.display().to_string();
                        let dest_abs = resolver.require_writable_opt(scope_root, &dest_wire)?;
                        if dest_abs.exists() {
                            return Err(CoderError::BadInput(format!(
                                "move destination already exists: {dest_wire}"
                            )));
                        }
                        Some(dest_abs)
                    }
                    None => None,
                };
                planned.push(PlannedWrite::Update {
                    abs,
                    move_to,
                    new_contents: applied.new_contents,
                    first_change: applied.first_change,
                });
            }
        }
    }

    // Write phase: every plan entry validated — apply in patch order.
    let mut results = Vec::with_capacity(planned.len());
    for write in &planned {
        match write {
            PlannedWrite::Add { abs, contents } => {
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CoderError::io_for_path(e, &abs.display().to_string()))?;
                }
                atomic_write(abs, contents.as_bytes())?;
                results.push(PatchFileResult {
                    path: abs.display().to_string(),
                    kind: "added".into(),
                    new_line_count: Some(line_count(contents)),
                    echo: None,
                });
            }
            PlannedWrite::Delete { abs } => {
                std::fs::remove_file(abs)
                    .map_err(|e| CoderError::io_for_path(e, &abs.display().to_string()))?;
                results.push(PatchFileResult {
                    path: abs.display().to_string(),
                    kind: "deleted".into(),
                    new_line_count: None,
                    echo: None,
                });
            }
            PlannedWrite::Update {
                abs,
                move_to,
                new_contents,
                first_change,
            } => {
                let target: &Path = move_to.as_deref().unwrap_or(abs.as_path());
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| CoderError::io_for_path(e, &target.display().to_string()))?;
                }
                atomic_write(target, new_contents.as_bytes())?;
                if move_to.is_some() && target != abs {
                    std::fs::remove_file(abs)
                        .map_err(|e| CoderError::io_for_path(e, &abs.display().to_string()))?;
                }
                results.push(PatchFileResult {
                    path: target.display().to_string(),
                    kind: if move_to.is_some() {
                        "moved"
                    } else {
                        "modified"
                    }
                    .into(),
                    new_line_count: Some(line_count(new_contents)),
                    echo: build_echo(new_contents, *first_change),
                });
            }
        }
    }
    Ok(ApplyPatchOutput { results })
}

fn check_write_size(cfg: &CoderConfig, wire: &str, len: usize) -> Result<(), CoderError> {
    if (len as u64) > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "{wire} new contents are {len} bytes, which exceeds max_write_bytes \
             ({}). Split the patch or raise max_write_bytes in coder config.",
            cfg.max_write_bytes
        )));
    }
    Ok(())
}

fn line_count(contents: &str) -> u64 {
    contents.lines().count() as u64
}

/// Bounded snapshot of the first changed region ±2 context lines.
fn build_echo(new_contents: &str, first_change: Option<(u64, u64)>) -> Option<PatchEcho> {
    let (line, len) = first_change?;
    let lines: Vec<&str> = new_contents.lines().collect();
    let from = line.saturating_sub(1 + ECHO_CONTEXT) as usize; // 0-based
    let to = ((line - 1 + len + ECHO_CONTEXT) as usize).min(lines.len());
    let window: Vec<String> = lines
        .get(from..to)
        .unwrap_or(&[])
        .iter()
        .take(ECHO_MAX_LINES)
        .map(|s| s.to_string())
        .collect();
    Some(PatchEcho {
        from_line: from as u64 + 1,
        lines: window,
    })
}
