//! `coder::delete-file` — remove one or more paths. Per-path errors are
//! reported in the result array rather than failing the whole batch.
//! Directories require `recursive: true`. Non-accessible paths return
//! `C211`. Trying to delete an allowed root itself is rejected.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::error::{err_to_string, CoderError, WireError};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_delete_file_input")]
pub struct DeleteFileInput {
    /// Paths to remove. Each entry is relative to the primary allowed root,
    /// or an absolute path inside any allowed root. Call `coder::info` to
    /// see the allowed roots. Paths outside every allowed root are rejected
    /// — use the shell worker's `shell::fs::*` for host paths outside the
    /// jail.
    pub paths: Vec<String>,
    /// Required for non-empty directories. Files and empty dirs ignore it.
    #[serde(default)]
    pub recursive: bool,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_delete_file_input() -> serde_json::Value {
    serde_json::json!({
        "paths": ["src/old_module.rs", "build/artifacts"],
        "recursive": true
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteFileOutput {
    pub results: Vec<DeleteFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteFileResult {
    /// Canonical absolute path (resolved through the jail); the caller's
    /// input verbatim when resolution failed.
    pub path: String,
    pub success: bool,
    pub removed: bool,
    /// Structured error for this entry. `code` is stable for programmatic
    /// branching (e.g. `"C211"` for not-found-or-denied; `"C210"` for
    /// refusing to delete an allowed root). `message` carries the
    /// corrective action an LLM agent needs to make a successful second call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<crate::code::config::CoderConfig>,
    req: DeleteFileInput,
) -> Result<DeleteFileOutput, String> {
    if req.paths.is_empty() {
        return Err(err_to_string(CoderError::BadInput(
            "`paths` must not be empty".into(),
        )));
    }
    let scope_root = crate::fs::scope_root(req.fs_scope.as_ref());
    let mut entries = Vec::with_capacity(req.paths.len());
    for p in req.paths {
        match resolver.require_writable_opt(scope_root, &p) {
            Ok(abs) => entries.push((p, Ok(abs))),
            Err(e) if is_jail_scope_error(&e) => return Err(err_to_string(e)),
            Err(e) => entries.push((p, Err(e))),
        }
    }
    let mut journal_entries = Vec::new();
    let results = entries
        .into_iter()
        .map(|(p, resolved)| {
            let (result, journal) = delete_one(&resolver, scope_root, &p, req.recursive, resolved);
            journal_entries.extend(journal);
            result
        })
        .collect();
    let root = resolver.effective_root(scope_root);
    crate::code::journal::record(
        &cfg,
        &root,
        req.fs_scope.as_ref(),
        "coder::delete-file",
        journal_entries,
    );
    Ok(DeleteFileOutput { results })
}

fn delete_one(
    resolver: &PathResolver,
    scope_root: Option<&str>,
    rel: &str,
    recursive: bool,
    resolved: Result<std::path::PathBuf, CoderError>,
) -> (DeleteFileResult, Option<crate::code::journal::EntryInput>) {
    // Resolve up front: deletion operates ONLY on the resolver-returned
    // path, and the result echoes that canonical absolute path. When
    // resolution fails there is no canonical path, so the caller's input
    // is echoed verbatim.
    let abs = match resolved {
        Ok(abs) => abs,
        Err(e) => {
            return (
                DeleteFileResult {
                    path: rel.to_string(),
                    success: false,
                    removed: false,
                    error: Some((&e).into()),
                },
                None,
            )
        }
    };
    let wire_path = abs.display().to_string();
    // Journal before-image: file contents pre-delete; a directory tree
    // cannot be snapshotted — journal it as a skipped (unrecoverable) gap.
    let journal_input = match std::fs::symlink_metadata(&abs) {
        Ok(md) if md.is_file() => Some(crate::code::journal::EntryInput {
            path: abs.clone(),
            before: std::fs::read(&abs).ok(),
            skipped: false,
        }),
        Ok(md) if md.is_dir() => Some(crate::code::journal::EntryInput {
            path: abs.clone(),
            before: None,
            skipped: true,
        }),
        _ => None,
    };
    match try_delete_one(resolver, scope_root, &abs, recursive) {
        Ok(removed) => (
            DeleteFileResult {
                path: wire_path,
                success: true,
                removed,
                error: None,
            },
            if removed { journal_input } else { None },
        ),
        Err(e) => (
            DeleteFileResult {
                path: wire_path,
                success: false,
                removed: false,
                error: Some((&e).into()),
            },
            None,
        ),
    }
}

fn is_jail_scope_error(e: &CoderError) -> bool {
    matches!(
        e,
        CoderError::OutsideBase(_) | CoderError::OutsideSession(_)
    )
}

fn try_delete_one(
    resolver: &PathResolver,
    scope_root: Option<&str>,
    abs: &Path,
    recursive: bool,
) -> Result<bool, CoderError> {
    if resolver.is_root(abs) {
        return Err(CoderError::BadInput(
            "refusing to delete an allowed root itself".into(),
        ));
    }
    // A session-scoped delete must not remove the session working directory
    // itself. With `scope_root` set, `paths: ["."]` (or any path that resolves
    // back to scope_root) canonicalizes to the session directory — which is a
    // SUBDIR of an allowed root, so the `is_root` guard above never catches it.
    // Without this, a recursive delete would wipe the active project directory.
    if let Some(base) = scope_root {
        if resolver.session_root(base).as_deref() == Some(abs) {
            return Err(CoderError::BadInput(
                "refusing to delete the session working directory itself".into(),
            ));
        }
    }
    let md = match std::fs::symlink_metadata(abs) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Idempotent: missing target counts as "not removed, no error".
            return Ok(false);
        }
        Err(e) => return Err(CoderError::from(e)),
    };
    if md.file_type().is_dir() {
        if recursive {
            remove_dir_all_safe(abs, resolver)?;
        } else {
            std::fs::remove_dir(abs).map_err(CoderError::from)?;
        }
    } else {
        std::fs::remove_file(abs).map_err(CoderError::from)?;
    }
    Ok(true)
}

/// `std::fs::remove_dir_all` plus a guard rail: refuse to descend through
/// non-accessible entries. The resolver canonicalised `abs` already so
/// it's known to be inside an allowed root.
fn remove_dir_all_safe(abs: &Path, resolver: &PathResolver) -> Result<(), CoderError> {
    for entry in walkdir::WalkDir::new(abs)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if resolver.is_non_accessible(entry.path()) {
            // REDACTION INVARIANT: do NOT name the discovered child path.
            // Naming it would allow callers to enumerate protected entries
            // by probing recursive deletes. The sanctioned constructor
            // references only the caller-supplied `abs`.
            return Err(CoderError::not_found_or_denied_subtree(
                &abs.display().to_string(),
            ));
        }
    }
    std::fs::remove_dir_all(abs).map_err(CoderError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (
        tempfile::TempDir,
        Arc<PathResolver>,
        Arc<crate::code::config::CoderConfig>,
    ) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(crate::code::config::CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            ..crate::code::config::CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn deletes_file() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["a.txt".into()],
                recursive: false,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert!(out.results[0].removed);
        assert!(!tmp.path().join("a.txt").exists());
    }

    #[tokio::test]
    async fn missing_path_is_idempotent_success() {
        let (_tmp, r, c) = setup();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["nope.txt".into()],
                recursive: false,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert!(!out.results[0].removed);
    }

    #[tokio::test]
    async fn refuses_non_accessible() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), "x").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec![".env".into()],
                recursive: false,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C211");
        assert!(tmp.path().join(".env").exists());
    }

    #[tokio::test]
    async fn directory_without_recursive_rejected() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/x"), "x").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["d".into()],
                recursive: false,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
    }

    #[tokio::test]
    async fn directory_with_recursive_succeeds() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/x"), "x").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["d".into()],
                recursive: true,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert!(!tmp.path().join("d").exists());
    }

    #[tokio::test]
    async fn recursive_refuses_when_subtree_has_non_accessible() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/.env"), "secret").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["d".into()],
                recursive: true,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C211");
        assert!(tmp.path().join("d/.env").exists());
    }

    // REDACTION INVARIANT: the error message for a recursive-delete blocked
    // by a non-accessible child MUST NOT contain the child's filename. The
    // caller supplied "d", so only "d" (its canonical absolute form) may
    // appear — the child ".env" must be invisible to the caller.
    #[tokio::test]
    async fn recursive_blocked_error_does_not_leak_child_path() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("secrets")).unwrap();
        std::fs::write(tmp.path().join("secrets/.env"), "API_KEY=secret").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["secrets".into()],
                recursive: true,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        // Code must be C211.
        assert_eq!(err.code, "C211", "expected C211, got: {:?}", err.code);
        // The discovered child name must NOT appear in the error.
        assert!(
            !err.message.contains(".env"),
            "REDACTION INVARIANT violated: error leaks child '.env': {}",
            err.message
        );
        // The caller-supplied directory name IS allowed to appear (it was
        // the input they gave us).
        assert!(
            err.message.contains("secrets"),
            "expected caller path 'secrets' in error, got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn refuses_to_delete_base_root() {
        let (_tmp, r, c) = setup();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec![".".into()],
                recursive: true,
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C210");
    }

    // The session working directory is a SUBDIR of the allowed root, so the
    // is_root guard does not cover it. A scoped `delete path="."` must still be
    // refused (C210) — otherwise a recursive delete wipes the active project.
    #[tokio::test]
    async fn refuses_to_delete_session_dir_via_dot() {
        let (tmp, r, c) = setup();
        let session = tmp.path().join("project");
        std::fs::create_dir(&session).unwrap();
        std::fs::write(session.join("keep.txt"), "x").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec![".".into()],
                recursive: true,
                fs_scope: Some(crate::fs::FsScope {
                    root: session.to_string_lossy().into_owned(),
                    grants: Vec::new(),
                    session_id: None,
                    turn_id: None,
                }),
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C210");
        assert!(session.exists(), "session dir must survive the delete");
        assert!(session.join("keep.txt").exists());
    }

    // The same protection must hold when the session dir is named by an
    // absolute path rather than ".".
    #[tokio::test]
    async fn refuses_to_delete_session_dir_via_absolute_path() {
        let (tmp, r, c) = setup();
        let session = tmp.path().join("project");
        std::fs::create_dir(&session).unwrap();
        let abs = session.to_string_lossy().into_owned();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec![abs.clone()],
                recursive: true,
                fs_scope: Some(crate::fs::FsScope {
                    root: abs,
                    grants: Vec::new(),
                    session_id: None,
                    turn_id: None,
                }),
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C210");
        assert!(session.exists(), "session dir must survive the delete");
    }

    // The guard must not be over-broad: a real file INSIDE the session dir
    // still deletes normally.
    #[tokio::test]
    async fn deletes_file_inside_session_dir() {
        let (tmp, r, c) = setup();
        let session = tmp.path().join("project");
        std::fs::create_dir(&session).unwrap();
        std::fs::write(session.join("a.txt"), "x").unwrap();
        let out = handle(
            r,
            c.clone(),
            DeleteFileInput {
                paths: vec!["a.txt".into()],
                recursive: false,
                fs_scope: Some(crate::fs::FsScope {
                    root: session.to_string_lossy().into_owned(),
                    grants: Vec::new(),
                    session_id: None,
                    turn_id: None,
                }),
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert!(out.results[0].removed);
        assert!(!session.join("a.txt").exists());
        assert!(session.exists(), "session dir itself is untouched");
    }
}
