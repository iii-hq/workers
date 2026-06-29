//! `coder::create-file` — write one or more new files. Each entry is
//! treated independently so a single bad input never aborts the rest.
//! Non-accessible paths and oversized payloads are rejected.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError, WireError};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_create_file_input")]
pub struct CreateFileInput {
    pub files: Vec<CreateFileSpec>,
    /// Optional per-call session working directory. When set, relative
    /// `path`s anchor here instead of the primary allowed root, and every
    /// resolved path must stay inside it. `base_dir` itself must canonicalize
    /// inside an allowed root (`coder::info` lists them). Omit to resolve
    /// against the primary allowed root exactly as before.
    #[serde(default)]
    pub base_dir: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateFileSpec {
    /// Path relative to the primary allowed root, or an absolute path inside
    /// any allowed root. Call `coder::info` to see the allowed roots. Paths
    /// outside every allowed root are rejected — use the shell worker's
    /// `shell::fs::*` for host paths outside the jail.
    pub path: String,
    pub content: String,
    /// Octal permission bits as a string, e.g. "0644". Defaults to "0644".
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Create missing parent directories. Defaults to true so a single
    /// `coder::create-file` call can scaffold a fresh subtree.
    #[serde(default = "default_true")]
    pub parents: bool,
    /// When false (the default), refuse to write if `path` already exists.
    #[serde(default)]
    pub overwrite: bool,
}

fn default_mode() -> String {
    "0644".to_string()
}
fn default_true() -> bool {
    true
}

// examples are wire-contract; goldens pin them.
fn example_create_file_input() -> serde_json::Value {
    serde_json::json!({
        "files": [
            {
                "path": "src/lib.rs",
                "content": "pub mod utils;\n",
                "overwrite": false
            },
            {
                "path": "/tmp/scratch/notes.md",
                "content": "# scratch notes\n",
                "overwrite": true
            }
        ]
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateFileOutput {
    pub results: Vec<CreateFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateFileResult {
    /// Canonical absolute path (resolved through the jail); the caller's
    /// input verbatim when resolution failed.
    pub path: String,
    pub success: bool,
    pub bytes_written: u64,
    /// Structured error for this entry. `code` is stable for programmatic
    /// branching (e.g. `"C217"` means already-exists; pass `overwrite=true`
    /// to replace). `message` carries the corrective action an LLM agent
    /// needs to make a successful second call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: CreateFileInput,
) -> Result<CreateFileOutput, String> {
    if req.files.is_empty() {
        return Err(err_to_string(CoderError::BadInput(
            "`files` must not be empty".into(),
        )));
    }
    let base_dir = req.base_dir.as_deref();
    let mut results = Vec::with_capacity(req.files.len());
    for spec in req.files {
        results.push(create_one(&resolver, &cfg, base_dir, spec));
    }
    Ok(CreateFileOutput { results })
}

fn create_one(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    base_dir: Option<&str>,
    spec: CreateFileSpec,
) -> CreateFileResult {
    // Resolve up front: from here on every filesystem operation uses ONLY
    // the resolver-returned path (never re-derived from the raw request),
    // and the result echoes that canonical absolute path. When resolution
    // fails there is no canonical path, so the input is echoed verbatim.
    let abs = match resolver.require_writable_opt(base_dir, &spec.path) {
        Ok(abs) => abs,
        Err(e) => {
            return CreateFileResult {
                path: spec.path,
                success: false,
                bytes_written: 0,
                error: Some((&e).into()),
            }
        }
    };
    let wire_path = abs.display().to_string();
    match try_create_one(cfg, &abs, spec) {
        Ok(bytes) => CreateFileResult {
            path: wire_path,
            success: true,
            bytes_written: bytes,
            error: None,
        },
        Err(e) => CreateFileResult {
            path: wire_path,
            success: false,
            bytes_written: 0,
            error: Some((&e).into()),
        },
    }
}

fn try_create_one(cfg: &CoderConfig, abs: &Path, spec: CreateFileSpec) -> Result<u64, CoderError> {
    let bytes = spec.content.as_bytes();
    if (bytes.len() as u64) > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "{} is {} bytes, which exceeds max_write_bytes ({}). \
             Split the content into smaller files or raise \
             max_write_bytes in coder config.",
            spec.path,
            bytes.len(),
            cfg.max_write_bytes
        )));
    }
    if abs.exists() && !spec.overwrite {
        return Err(CoderError::AlreadyExists(format!(
            "{} already exists; pass overwrite=true to replace",
            spec.path
        )));
    }
    if spec.parents {
        if let Some(parent) = abs.parent() {
            // io_for_path names spec.path (caller-supplied, redaction-safe)
            // rather than the derived parent directory.
            std::fs::create_dir_all(parent).map_err(|e| CoderError::io_for_path(e, &spec.path))?;
        }
    }
    std::fs::write(abs, bytes).map_err(|e| CoderError::io_for_path(e, &spec.path))?;
    apply_mode(abs, &spec.mode)?;
    Ok(bytes.len() as u64)
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode_str: &str) -> Result<(), CoderError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = u32::from_str_radix(mode_str.trim_start_matches('0'), 8)
        .map_err(|e| CoderError::BadInput(format!("bad mode {mode_str:?}: {e}")))?;
    let perms = std::fs::Permissions::from_mode(mode & 0o777);
    std::fs::set_permissions(path, perms).map_err(CoderError::from)
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode_str: &str) -> Result<(), CoderError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            max_write_bytes: 1024 * 1024,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn creates_simple_file() {
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "hello".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(out.results[0].bytes_written, 5);
        // Successful entries echo the canonical absolute path.
        assert_eq!(
            out.results[0].path,
            std::fs::canonicalize(tmp.path())
                .unwrap()
                .join("a.txt")
                .display()
                .to_string()
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn creates_with_parents() {
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a/b/c.txt".into(),
                    content: "hi".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "{:?}", out.results[0].error);
        assert!(tmp.path().join("a/b/c.txt").exists());
    }

    #[tokio::test]
    async fn rejects_existing_without_overwrite() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "new".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        let err = out.results[0].error.as_ref().unwrap();
        assert_eq!(err.code, "C217");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "old"
        );
    }

    #[tokio::test]
    async fn overwrite_replaces_existing() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "a.txt".into(),
                    content: "new".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "new"
        );
    }

    #[tokio::test]
    async fn refuses_non_accessible() {
        let (_tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: ".env".into(),
                    content: "secret".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: true,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C211");
    }

    #[tokio::test]
    async fn jail_escape_reports_c215_per_item_and_batch_continues() {
        // A jail escape on one entry must surface as a per-item C215 (not a
        // top-level failure) and must NOT abort the rest of the batch — the
        // write-side per-item jail contract that the dropped path-security
        // BDD scenarios asserted.
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![
                    CreateFileSpec {
                        path: "../escape.txt".into(),
                        content: "x".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                    },
                    CreateFileSpec {
                        path: "ok.txt".into(),
                        content: "y".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                    },
                ],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success, "escape entry must fail");
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C215");
        assert!(
            out.results[1].success,
            "the in-jail entry must still be written"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("ok.txt")).unwrap(),
            "y"
        );
        assert!(
            !tmp.path().join("../escape.txt").exists(),
            "the escaping path must never be created"
        );
    }

    #[tokio::test]
    async fn refuses_oversize() {
        let (_tmp, r, _c) = setup();
        let small_cfg = Arc::new(CoderConfig {
            base_paths: vec![_tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            max_write_bytes: 4,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            small_cfg,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: "big.txt".into(),
                    content: "abcdefg".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C213");
    }

    #[tokio::test]
    async fn multi_file_partial_success() {
        let (tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![
                    CreateFileSpec {
                        path: ".env".into(),
                        content: "x".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                    },
                    CreateFileSpec {
                        path: "ok.txt".into(),
                        content: "y".into(),
                        mode: "0644".into(),
                        parents: true,
                        overwrite: false,
                    },
                ],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(!out.results[0].success);
        assert!(out.results[1].success);
        assert!(tmp.path().join("ok.txt").exists());
    }

    /// The per-entry `error` field must serialize as a raw JSON object —
    /// NOT a JSON string containing escaped JSON. An LLM agent reading
    /// `"code":"C2` directly as an object key requires no mental
    /// unescaping; the old wire shape `\"code\":\"C2` was a double-encode.
    #[tokio::test]
    async fn error_field_serializes_as_structured_object_not_escaped_string() {
        let (_tmp, r, c) = setup();
        let out = handle(
            r,
            c,
            CreateFileInput {
                files: vec![CreateFileSpec {
                    path: ".env".into(),
                    content: "x".into(),
                    mode: "0644".into(),
                    parents: true,
                    overwrite: false,
                }],
                base_dir: None,
            },
        )
        .await
        .unwrap();
        let serialized = serde_json::to_string(&out.results[0]).unwrap();
        // Structured object key must appear raw.
        assert!(
            serialized.contains(r#""code":"C2"#),
            "expected raw object key; got: {serialized}"
        );
        // Double-encoded form must NOT appear.
        assert!(
            !serialized.contains(r#"\"code\""#),
            "double-encoded JSON detected; got: {serialized}"
        );
    }
}
