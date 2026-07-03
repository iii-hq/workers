//! `coder::move` — rename or move one or more paths inside the jail.
//!
//! Same-root moves use `std::fs::rename` (atomic, works for files and
//! directories). Cross-root moves are supported for FILES only: copy → verify
//! (length match) → delete source, with rollback when the source delete fails.
//! Cross-root directory moves are rejected (`C210`). Per-entry errors are
//! reported in the result array rather than failing the whole batch.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::error::{err_to_string, CoderError, WireError};
use crate::code::path::PathResolver;

// ---------------------------------------------------------------------------
// Input / output types
// ---------------------------------------------------------------------------

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_move_file_input")]
pub struct MoveFileInput {
    /// Entries to move. Each entry is processed independently so a single
    /// failure never aborts the rest.
    pub files: Vec<MoveFileSpec>,
    /// Internal harness-scoped working directory; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub base_dir: Option<String>,
    /// Internal harness-granted roots; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub extra_roots: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveFileSpec {
    /// Source path: relative to the primary allowed root, or an absolute path
    /// inside any allowed root. Call `coder::info` to see the allowed roots.
    /// Paths outside every allowed root are rejected — use the shell worker's
    /// `shell::fs::*` for host paths outside the jail.
    pub from: String,

    /// Destination path: relative to the primary allowed root, or an absolute
    /// path inside any allowed root. Call `coder::info` to see the allowed
    /// roots. Paths outside every allowed root are rejected — use the shell
    /// worker's `shell::fs::*` for host paths outside the jail.
    pub to: String,

    /// When false (the default), refuse to overwrite an existing destination.
    /// Pass `overwrite: true` to replace an existing file at `to`.
    #[serde(default)]
    pub overwrite: bool,

    /// Create missing parent directories of the destination. Defaults to true.
    #[serde(default = "default_true")]
    pub parents: bool,
}

fn default_true() -> bool {
    true
}

// examples are wire-contract; goldens pin them.
fn example_move_file_input() -> serde_json::Value {
    serde_json::json!({
        "files": [
            { "from": "src/old_name.rs", "to": "src/new_name.rs" },
            { "from": "build/output.bin",
              "to": "/tmp/coder-cache/output.bin",
              "overwrite": true }
        ]
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MoveFileOutput {
    pub results: Vec<MoveFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MoveFileResult {
    /// Canonical absolute path of the source (resolved through the jail);
    /// the caller's input verbatim when resolution failed.
    pub from: String,

    /// Canonical absolute path of the destination (resolved through the jail);
    /// the caller's input verbatim when resolution failed.
    pub to: String,

    pub success: bool,

    /// True only when the move fully completed; false for a no-op
    /// self-move (`from` and `to` resolve to the same file).
    pub moved: bool,

    /// Structured error for this entry. `code` is stable for programmatic
    /// branching (e.g. `"C217"` means destination exists; pass `overwrite=true`
    /// to replace; `"C210"` for disallowed operations such as cross-root
    /// directory moves, moving a root itself, or a destination that is a
    /// directory — the message then names the corrected target path).
    /// `message` carries the corrective action an LLM agent needs to make
    /// a successful second call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(
    resolver: Arc<PathResolver>,
    req: MoveFileInput,
) -> Result<MoveFileOutput, String> {
    if req.files.is_empty() {
        return Err(err_to_string(CoderError::BadInput(
            "`files` must not be empty".into(),
        )));
    }
    let base_dir = req.base_dir.as_deref();
    for spec in &req.files {
        for path in [&spec.from, &spec.to] {
            if let Err(e) = resolver.require_writable_opt(base_dir, path) {
                if is_jail_scope_error(&e) {
                    return Err(err_to_string(e));
                }
            }
        }
    }
    let mut results = Vec::with_capacity(req.files.len());
    for spec in req.files {
        results.push(move_one(&resolver, base_dir, spec));
    }
    Ok(MoveFileOutput { results })
}

fn is_jail_scope_error(e: &CoderError) -> bool {
    matches!(
        e,
        CoderError::OutsideBase(_) | CoderError::OutsideSession(_)
    )
}

// ---------------------------------------------------------------------------
// Per-entry logic
// ---------------------------------------------------------------------------

fn move_one(resolver: &PathResolver, base_dir: Option<&str>, spec: MoveFileSpec) -> MoveFileResult {
    // Resolve source.
    let abs_from = match resolver.require_writable_opt(base_dir, &spec.from) {
        Ok(p) => p,
        Err(e) => {
            return MoveFileResult {
                from: spec.from,
                to: spec.to,
                success: false,
                moved: false,
                error: Some((&e).into()),
            }
        }
    };

    // Resolve destination (may not exist yet — resolution via fallback is fine).
    let abs_to = match resolver.require_writable_opt(base_dir, &spec.to) {
        Ok(p) => p,
        Err(e) => {
            return MoveFileResult {
                from: abs_from.display().to_string(),
                to: spec.to,
                success: false,
                moved: false,
                error: Some((&e).into()),
            }
        }
    };

    // SELF-MOVE GUARD — before ALL other checks. Allowed roots may NEST
    // (containing_root is first-match-wins), so `from` and `to` can name
    // the SAME file through different wire forms; if such a pair ever
    // reached the cross-root copy+delete branch, fs::copy(src, dst) with
    // src==dst would TRUNCATE the file before reading it and the
    // follow-up delete would remove it entirely — data loss. A self-move
    // is a no-op success: moved=false, no error.
    if abs_from == abs_to {
        let wire = abs_from.display().to_string();
        return MoveFileResult {
            from: wire.clone(),
            to: wire,
            success: true,
            moved: false,
            error: None,
        };
    }

    let wire_from = abs_from.display().to_string();
    let wire_to = abs_to.display().to_string();

    match try_move_one(
        resolver,
        &abs_from,
        &abs_to,
        &spec.from,
        &spec.to,
        spec.overwrite,
        spec.parents,
    ) {
        Ok(()) => MoveFileResult {
            from: wire_from,
            to: wire_to,
            success: true,
            moved: true,
            error: None,
        },
        Err(e) => MoveFileResult {
            from: wire_from,
            to: wire_to,
            success: false,
            moved: false,
            error: Some((&e).into()),
        },
    }
}

fn try_move_one(
    resolver: &PathResolver,
    abs_from: &Path,
    abs_to: &Path,
    wire_from: &str,
    wire_to: &str,
    overwrite: bool,
    parents: bool,
) -> Result<(), CoderError> {
    // ------------------------------------------------------------------
    // Categorical rejections FIRST — everything in this block is
    // read-only. A rejected entry must leave ZERO side effects, so no
    // filesystem mutation (including parent-dir creation) may run before
    // every rejection below has been ruled out.
    // ------------------------------------------------------------------

    // Guard: refuse to move an allowed root itself (mirrors delete's root guard).
    if resolver.is_root(abs_from) {
        return Err(CoderError::BadInput(
            "refusing to move an allowed root itself".into(),
        ));
    }

    // Source must exist.
    let src_meta =
        std::fs::symlink_metadata(abs_from).map_err(|e| CoderError::io_for_path(e, wire_from))?;

    let from_root = resolver.containing_root(abs_from);
    let to_root = resolver.containing_root(abs_to);
    let same_root = match (from_root, to_root) {
        (Some(fr), Some(tr)) => fr == tr,
        _ => false,
    };

    // Cross-root directory moves are categorically unsupported.
    if !same_root && src_meta.is_dir() {
        return Err(CoderError::BadInput(
            "cross-root directory moves are unsupported; move files individually".into(),
        ));
    }

    // Destination conflict checks.
    if let Ok(dst_meta) = std::fs::symlink_metadata(abs_to) {
        if dst_meta.is_dir() && !src_meta.is_dir() {
            // overwrite=true can't fix this (a file cannot replace a
            // directory via rename), so don't send the caller down the
            // C217 dead end — tell them the actual corrective call.
            let fname = abs_from
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<file>".to_string());
            return Err(CoderError::BadInput(format!(
                "{wire_to}: destination is a directory; name the target \
                 file inside it (e.g. {wire_to}/{fname})"
            )));
        }
        if !overwrite {
            return Err(CoderError::AlreadyExists(format!(
                "{wire_to} already exists; pass overwrite=true to replace"
            )));
        }
    }

    // ------------------------------------------------------------------
    // MUTATIONS BEGIN — nothing above this line may touch the filesystem
    // (pinned by rejected_*_leaves_dst_tree_absent test).
    // ------------------------------------------------------------------

    // Create destination parent directories if requested.
    if parents {
        if let Some(parent) = abs_to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoderError::io_for_path(e, wire_to))?;
        }
    }

    if same_root {
        // Same containing root — attempt atomic rename.
        // For files: on cross-device errors (rename across mount points
        // inside the same logical root, unusual but possible with
        // bind-mounts), fall back to copy+delete.
        // For directories: rename is the only safe option; surface the error.
        if src_meta.is_dir() {
            std::fs::rename(abs_from, abs_to).map_err(|e| CoderError::io_for_path(e, wire_from))?;
        } else {
            match std::fs::rename(abs_from, abs_to) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                    copy_and_delete(abs_from, abs_to, wire_from, wire_to)?;
                }
                Err(e) => return Err(CoderError::io_for_path(e, wire_from)),
            }
        }
    } else {
        // Cross-root FILE move (directories were rejected above):
        // copy → verify → delete source, with rollback on failure.
        copy_and_delete(abs_from, abs_to, wire_from, wire_to)?;
    }

    Ok(())
}

/// Copy `src` to `dst`, verify byte count matches, then delete `src`.
/// On source-delete failure: attempt to remove the copied `dst` (rollback).
/// Structured error messages describe the resulting state for both
/// rollback-succeeded and double-fault cases.
fn copy_and_delete(
    src: &Path,
    dst: &Path,
    wire_from: &str,
    wire_to: &str,
) -> Result<(), CoderError> {
    // Copy.
    let copied_bytes = std::fs::copy(src, dst).map_err(|e| CoderError::io_for_path(e, wire_to))?;

    // Verify: compare source size against bytes written.
    // TOCTOU caveat: lengths may spuriously mismatch if src is modified
    // between copy() and metadata(); acceptable in a jailed single-agent
    // context.
    let src_len = match std::fs::metadata(src) {
        Ok(m) => m.len(),
        Err(e) => {
            // Metadata read failed after copy — unusual; attempt rollback.
            let _ = std::fs::remove_file(dst);
            return Err(CoderError::io_for_path(e, wire_from));
        }
    };

    if copied_bytes != src_len {
        // Size mismatch — copy incomplete; rollback.
        let _ = std::fs::remove_file(dst);
        return Err(CoderError::Io(format!(
            "copy incomplete: wrote {copied_bytes} bytes but source is {src_len} bytes; \
             copy removed, source unchanged"
        )));
    }

    // Delete source.
    if let Err(del_err) = std::fs::remove_file(src) {
        // Source delete failed — attempt rollback by removing the copy.
        match std::fs::remove_file(dst) {
            Ok(()) => {
                // Rollback succeeded: report the delete failure; state is clean.
                return Err(CoderError::Io(format!(
                    "move rolled back; source unchanged: failed to delete source \
                     after copy ({del_err}); the copy at {wire_to} was removed"
                )));
            }
            Err(rb_err) => {
                // Double fault: both delete-src and rollback failed.
                return Err(CoderError::Io(format!(
                    "copy exists at {wire_to}; source remains at {wire_from}; \
                     manual cleanup needed: failed to delete source ({del_err}) and \
                     rollback also failed ({rb_err})"
                )));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn setup_single() -> (tempfile::TempDir, Arc<PathResolver>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(crate::code::config::CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            ..crate::code::config::CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver)
    }

    fn setup_two_roots() -> (tempfile::TempDir, tempfile::TempDir, Arc<PathResolver>) {
        let tmp0 = tempdir().unwrap();
        let tmp1 = tempdir().unwrap();
        let cfg = Arc::new(crate::code::config::CoderConfig {
            base_paths: vec![tmp0.path().to_path_buf(), tmp1.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            ..crate::code::config::CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp0, tmp1, resolver)
    }

    // ------------------------------------------------------------------
    // Same-root: rename a file
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn same_root_rename_file() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "a.txt".into(),
                    to: "b.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(out.results[0].moved);
        assert!(!tmp.path().join("a.txt").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("b.txt")).unwrap(),
            "hello"
        );
        // Echo: canonical absolute paths for both ends.
        let canon = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(
            out.results[0].from,
            canon.join("a.txt").display().to_string()
        );
        assert_eq!(out.results[0].to, canon.join("b.txt").display().to_string());
    }

    // ------------------------------------------------------------------
    // Same-root: rename a directory
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn same_root_rename_directory() {
        let (tmp, r) = setup_single();
        std::fs::create_dir(tmp.path().join("src_dir")).unwrap();
        std::fs::write(tmp.path().join("src_dir/file.txt"), "content").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "src_dir".into(),
                    to: "dst_dir".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(!tmp.path().join("src_dir").exists());
        assert!(tmp.path().join("dst_dir/file.txt").exists());
    }

    // ------------------------------------------------------------------
    // SELF-MOVE GUARD: from == to is a no-op success (moved=false) and
    // the file is untouched. Without the guard, a same-file pair routed
    // to copy+delete would truncate-then-delete the file (data loss).
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn self_move_is_noop_success_with_content_intact() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("a.txt"), "precious").unwrap();
        for overwrite in [false, true] {
            let out = handle(
                r.clone(),
                MoveFileInput {
                    files: vec![MoveFileSpec {
                        from: "a.txt".into(),
                        to: "a.txt".into(),
                        overwrite,
                        parents: true,
                    }],
                    base_dir: None,
                    extra_roots: None,
                },
            )
            .await
            .unwrap();
            assert!(out.results[0].success, "overwrite={overwrite}");
            assert!(!out.results[0].moved, "self-move must report moved=false");
            assert!(out.results[0].error.is_none());
            // Both echoes are the same canonical path.
            assert_eq!(out.results[0].from, out.results[0].to);
            // File content INTACT.
            assert_eq!(
                std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
                "precious",
                "overwrite={overwrite}: self-move must not touch the file"
            );
        }
    }

    /// NESTED ROOTS regression: root B lives INSIDE root A, so the same
    /// file is reachable via A's relative form and via an absolute path
    /// inside B. Both canonicalize to one path; the self-move guard must
    /// catch the pair before any branch (the cross-root branch would
    /// otherwise fs::copy(src, src) → truncation → delete → data loss).
    #[tokio::test]
    async fn self_move_through_nested_roots_is_noop_and_content_intact() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("sub");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("file.txt"), "do not lose me").unwrap();
        let cfg = Arc::new(crate::code::config::CoderConfig {
            // Root B (= A/sub) nests inside root A; A is primary.
            base_paths: vec![tmp.path().to_path_buf(), nested.clone()],
            non_accessible_globs: vec![],
            ..crate::code::config::CoderConfig::default()
        });
        let r = Arc::new(PathResolver::new(&cfg).unwrap());
        assert_eq!(r.roots().len(), 2, "both nested roots must be active");

        // from: relative through primary root A; to: absolute inside B.
        let to_abs = std::fs::canonicalize(&nested).unwrap().join("file.txt");
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "sub/file.txt".into(),
                    to: to_abs.display().to_string(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(!out.results[0].moved, "same-file pair must be a no-op");
        assert!(out.results[0].error.is_none());
        // The file survives with its content intact.
        assert_eq!(
            std::fs::read_to_string(nested.join("file.txt")).unwrap(),
            "do not lose me"
        );
    }

    // ------------------------------------------------------------------
    // Cross-root: file copy+delete (verify content + src gone)
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn cross_root_file_copy_and_delete() {
        let (tmp0, tmp1, r) = setup_two_roots();
        std::fs::write(tmp0.path().join("src.txt"), "cross-root content").unwrap();
        let dst_abs = tmp1.path().join("dst.txt");
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "src.txt".into(),
                    to: dst_abs.display().to_string(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(out.results[0].moved);
        // Source gone.
        assert!(!tmp0.path().join("src.txt").exists());
        // Destination has correct content.
        assert_eq!(
            std::fs::read_to_string(&dst_abs).unwrap(),
            "cross-root content"
        );
    }

    // ------------------------------------------------------------------
    // Rollback on src-delete failure (via read-only parent post-copy)
    // ------------------------------------------------------------------
    #[cfg(unix)]
    #[tokio::test]
    async fn rollback_on_src_delete_failure() {
        // Skip when running as root — chmod is ineffective for root.
        // Use `id -u` via the shell rather than libc to avoid adding a dependency.
        let euid: u32 = std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if euid == 0 {
            eprintln!("SKIP: rollback_on_src_delete_failure skipped when euid==0 (CI-as-root makes chmod ineffective)");
            return;
        }

        let (tmp0, tmp1, r) = setup_two_roots();
        // Write source file under a parent directory we can make read-only.
        std::fs::create_dir(tmp0.path().join("locked")).unwrap();
        std::fs::write(tmp0.path().join("locked/src.txt"), "rollback test").unwrap();

        // Make parent read-only so remove_file(src) will fail after copy.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            tmp0.path().join("locked"),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();

        let dst_abs = tmp1.path().join("dst.txt");
        let src_abs = tmp0.path().join("locked/src.txt");

        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: src_abs.display().to_string(),
                    to: dst_abs.display().to_string(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();

        // Restore permissions for cleanup.
        let _ = std::fs::set_permissions(
            tmp0.path().join("locked"),
            std::fs::Permissions::from_mode(0o755),
        );

        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        assert_eq!(err.code, "C216", "expected C216, got: {:?}", err.code);
        // Error must mention rollback.
        assert!(
            err.message.contains("rolled back") || err.message.contains("rollback"),
            "error should mention rollback: {}",
            err.message
        );
        // Source must still exist (rollback succeeded).
        assert!(src_abs.exists(), "source must remain after rollback");
        // Copy must have been removed (rollback cleaned up dst).
        assert!(!dst_abs.exists(), "dst copy must be removed after rollback");
    }

    // ------------------------------------------------------------------
    // overwrite=false → C217
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn overwrite_false_dst_exists_c217() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("src.txt"), "new").unwrap();
        std::fs::write(tmp.path().join("dst.txt"), "old").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "src.txt".into(),
                    to: "dst.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        assert_eq!(err.code, "C217");
        // House C217 shape — identical format to create-file's (no colon).
        assert_eq!(
            err.message,
            "dst.txt already exists; pass overwrite=true to replace"
        );
        // Both files must remain untouched.
        assert!(tmp.path().join("src.txt").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("dst.txt")).unwrap(),
            "old"
        );
    }

    // ------------------------------------------------------------------
    // overwrite=true replaces existing destination
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn overwrite_true_replaces_existing() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("src.txt"), "new-content").unwrap();
        std::fs::write(tmp.path().join("dst.txt"), "old-content").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "src.txt".into(),
                    to: "dst.txt".into(),
                    overwrite: true,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(!tmp.path().join("src.txt").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("dst.txt")).unwrap(),
            "new-content"
        );
    }

    // ------------------------------------------------------------------
    // Missing source → C211 (standard redaction wording)
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn missing_src_c211() {
        let (_tmp, r) = setup_single();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "no-such-file.txt".into(),
                    to: "dst.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        assert_eq!(err.code, "C211");
        // Redaction: must use the standard C211 suffix; must NOT leak OS text.
        assert!(
            err.message.contains("not found or not accessible"),
            "C211 must use standard wording: {}",
            err.message
        );
        assert!(
            !err.message.contains("os error"),
            "C211 must not leak OS error text: {}",
            err.message
        );
    }

    // ------------------------------------------------------------------
    // Glob-denied source (redaction: same C211 suffix as missing)
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn glob_denied_src_c211_identical_suffix_to_missing() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join(".env"), "secret").unwrap();
        // Both resolve + deny and stat failure yield C211; suffix must match.
        let out_denied = handle(
            r.clone(),
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: ".env".into(),
                    to: "dst.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        let out_missing = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "definitely-missing.txt".into(),
                    to: "dst2.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out_denied.results[0].success);
        assert!(!out_missing.results[0].success);
        let denied_err = out_denied.results[0].error.as_ref().unwrap();
        let missing_err = out_missing.results[0].error.as_ref().unwrap();
        assert_eq!(denied_err.code, "C211");
        assert_eq!(missing_err.code, "C211");
        // Suffix after "path: " must be byte-identical (REDACTION INVARIANT).
        let denied_suffix = denied_err
            .message
            .strip_prefix(".env: ")
            .expect("denied message must start with the supplied path");
        let missing_suffix = missing_err
            .message
            .strip_prefix("definitely-missing.txt: ")
            .expect("missing message must start with the supplied path");
        assert_eq!(
            denied_suffix, missing_suffix,
            "C211 missing vs glob-denied suffixes must be byte-identical"
        );
        // Source must NOT have been moved.
        assert!(tmp.path().join(".env").exists());
    }

    // ------------------------------------------------------------------
    // Glob-denied destination
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn glob_denied_dst_c211() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("src.txt"), "x").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "src.txt".into(),
                    to: ".env".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C211");
        // Source must remain.
        assert!(tmp.path().join("src.txt").exists());
    }

    // ------------------------------------------------------------------
    // Move an allowed root itself → C210
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn move_root_rejected_c210() {
        let (_tmp, r) = setup_single();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: ".".into(),
                    to: "other".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C210");
    }

    // ------------------------------------------------------------------
    // Cross-root directory move → C210
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn cross_root_dir_rejected_c210() {
        let (tmp0, tmp1, r) = setup_two_roots();
        std::fs::create_dir(tmp0.path().join("mydir")).unwrap();
        std::fs::write(tmp0.path().join("mydir/f.txt"), "x").unwrap();
        let dst_abs = tmp1.path().join("mydir");
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "mydir".into(),
                    to: dst_abs.display().to_string(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        assert_eq!(err.code, "C210");
        assert!(
            err.message.contains("cross-root"),
            "error must mention cross-root: {}",
            err.message
        );
    }

    // ------------------------------------------------------------------
    // ZERO SIDE EFFECTS: a rejected entry must mutate nothing — in
    // particular, the parents=true dir creation must NOT run before the
    // categorical rejections (probe regression: empty parent dirs were
    // created at dst for a rejected cross-root dir move).
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn rejected_cross_root_dir_move_leaves_dst_tree_absent() {
        let (tmp0, tmp1, r) = setup_two_roots();
        std::fs::create_dir(tmp0.path().join("mydir")).unwrap();
        std::fs::write(tmp0.path().join("mydir/f.txt"), "x").unwrap();
        // Destination nested under parents that don't exist yet.
        let dst_abs = tmp1.path().join("a/b/mydir");
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "mydir".into(),
                    to: dst_abs.display().to_string(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C210");
        // Source untouched.
        assert!(tmp0.path().join("mydir/f.txt").exists());
        // No part of the dst tree may have been created.
        assert!(
            !tmp1.path().join("a").exists(),
            "rejected entry must not create dst parent directories"
        );
    }

    // ------------------------------------------------------------------
    // dst is a directory + src is a file → prescriptive C210 (not the
    // C217 "pass overwrite=true" dead end — overwrite can't fix it).
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn dst_is_directory_src_is_file_prescriptive_c210() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("src.txt"), "x").unwrap();
        std::fs::create_dir(tmp.path().join("destdir")).unwrap();
        // Both overwrite=false AND overwrite=true must get the same
        // guidance: overwrite cannot make a file replace a directory.
        for overwrite in [false, true] {
            let out = handle(
                r.clone(),
                MoveFileInput {
                    files: vec![MoveFileSpec {
                        from: "src.txt".into(),
                        to: "destdir".into(),
                        overwrite,
                        parents: true,
                    }],
                    base_dir: None,
                    extra_roots: None,
                },
            )
            .await
            .unwrap();
            assert!(!out.results[0].success, "overwrite={overwrite}");
            let err = out.results[0].error.as_ref().unwrap();
            assert_eq!(err.code, "C210", "overwrite={overwrite}: {}", err.message);
            assert!(
                err.message.contains("destination is a directory"),
                "must explain dst is a dir: {}",
                err.message
            );
            // The e.g. hint must name the corrected target path.
            assert!(
                err.message.contains("destdir/src.txt"),
                "must suggest the target file inside the dir: {}",
                err.message
            );
            // Source and destination untouched.
            assert!(tmp.path().join("src.txt").exists());
            assert!(tmp.path().join("destdir").is_dir());
        }
    }

    // ------------------------------------------------------------------
    // parents=false + missing parent directory → error
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn parents_false_missing_parent_errors() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("src.txt"), "x").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "src.txt".into(),
                    to: "missing_parent/dst.txt".into(),
                    overwrite: false,
                    parents: false,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        // rename itself may fail or succeed depending on whether the parent
        // directory exists; since it doesn't, the move should fail.
        // On Linux/macOS std::fs::rename returns an error when dst parent is absent.
        assert!(!out.results[0].success);
        // Source must still be there.
        assert!(tmp.path().join("src.txt").exists());
    }

    // ------------------------------------------------------------------
    // Order preservation in a batch
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn batch_order_preserved() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "B").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![
                    MoveFileSpec {
                        from: "a.txt".into(),
                        to: "a2.txt".into(),
                        overwrite: false,
                        parents: true,
                    },
                    MoveFileSpec {
                        from: "b.txt".into(),
                        to: "b2.txt".into(),
                        overwrite: false,
                        parents: true,
                    },
                ],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(out.results[0].success);
        assert!(out.results[1].success);
        // Results are in request order.
        assert!(out.results[0].from.contains("a.txt"));
        assert!(out.results[1].from.contains("b.txt"));
    }

    // ------------------------------------------------------------------
    // Echo rules: canonical when resolved, verbatim when not
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn jail_escape_aborts_batch_with_top_level_c215() {
        let (_tmp, r) = setup_single();
        // A path that escapes the jail will fail resolution (C215).
        let err = handle(
            r,
            MoveFileInput {
                files: vec![MoveFileSpec {
                    from: "/etc/passwd".into(),
                    to: "dst.txt".into(),
                    overwrite: false,
                    parents: true,
                }],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap_err();
        // Jail-scope errors must be visible to the hook as whole-call errors.
        assert!(err.contains("\"code\":\"C215\""));
        assert!(err.contains("/etc/passwd"));
        assert!(err.contains("grant_hint="));
    }

    // ------------------------------------------------------------------
    // Batch continues after a per-entry failure
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn batch_continues_after_failure() {
        let (tmp, r) = setup_single();
        std::fs::write(tmp.path().join("good.txt"), "ok").unwrap();
        let out = handle(
            r,
            MoveFileInput {
                files: vec![
                    // First entry: missing source → C211.
                    MoveFileSpec {
                        from: "missing.txt".into(),
                        to: "out1.txt".into(),
                        overwrite: false,
                        parents: true,
                    },
                    // Second entry: valid move → should succeed.
                    MoveFileSpec {
                        from: "good.txt".into(),
                        to: "moved.txt".into(),
                        overwrite: false,
                        parents: true,
                    },
                ],
                base_dir: None,
                extra_roots: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(!out.results[0].success);
        assert!(out.results[1].success);
        assert!(!tmp.path().join("good.txt").exists());
        assert!(tmp.path().join("moved.txt").exists());
    }
}
