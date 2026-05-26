//! `coder::delete-file` — remove one or more paths. Per-path errors are
//! reported in the result array rather than failing the whole batch.
//! Directories require `recursive: true`. Non-accessible paths return
//! `C211`. Trying to delete `base_path` itself is rejected.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{err_to_string, CoderError};
use crate::path::PathResolver;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteFileInput {
    pub paths: Vec<String>,
    /// Required for non-empty directories. Files and empty dirs ignore it.
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteFileOutput {
    pub results: Vec<DeleteFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteFileResult {
    pub path: String,
    pub success: bool,
    pub removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    req: DeleteFileInput,
) -> Result<DeleteFileOutput, String> {
    if req.paths.is_empty() {
        return Err(err_to_string(CoderError::BadInput(
            "`paths` must not be empty".into(),
        )));
    }
    let mut results = Vec::with_capacity(req.paths.len());
    for p in req.paths {
        results.push(delete_one(&resolver, &p, req.recursive));
    }
    Ok(DeleteFileOutput { results })
}

fn delete_one(resolver: &PathResolver, rel: &str, recursive: bool) -> DeleteFileResult {
    match try_delete_one(resolver, rel, recursive) {
        Ok(removed) => DeleteFileResult {
            path: rel.to_string(),
            success: true,
            removed,
            error: None,
        },
        Err(e) => DeleteFileResult {
            path: rel.to_string(),
            success: false,
            removed: false,
            error: Some(e.to_wire_string()),
        },
    }
}

fn try_delete_one(resolver: &PathResolver, rel: &str, recursive: bool) -> Result<bool, CoderError> {
    let abs = resolver.require_writable(rel)?;
    if abs == resolver.base_root() {
        return Err(CoderError::BadInput(
            "refusing to delete base_path itself".into(),
        ));
    }
    let md = match std::fs::symlink_metadata(&abs) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Idempotent: missing target counts as "not removed, no error".
            return Ok(false);
        }
        Err(e) => return Err(CoderError::from(e)),
    };
    if md.file_type().is_dir() {
        if recursive {
            remove_dir_all_safe(&abs, resolver)?;
        } else {
            std::fs::remove_dir(&abs).map_err(CoderError::from)?;
        }
    } else {
        std::fs::remove_file(&abs).map_err(CoderError::from)?;
    }
    Ok(true)
}

/// `std::fs::remove_dir_all` plus a guard rail: refuse to descend through
/// non-accessible entries. The resolver canonicalised `abs` already so
/// it's known to be inside `base_root`.
fn remove_dir_all_safe(abs: &Path, resolver: &PathResolver) -> Result<(), CoderError> {
    for entry in walkdir::WalkDir::new(abs)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if resolver.is_non_accessible(entry.path()) {
            return Err(CoderError::NotFoundOrDenied(format!(
                "recursive delete blocked: {} contains non-accessible {}",
                abs.display(),
                entry.path().display()
            )));
        }
    }
    std::fs::remove_dir_all(abs).map_err(CoderError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(crate::config::CoderConfig {
            base_path: tmp.path().to_path_buf(),
            non_accessible_globs: vec!["**/.env".to_string()],
            ..crate::config::CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver)
    }

    #[tokio::test]
    async fn deletes_file() {
        let (tmp, r) = setup();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec!["a.txt".into()],
                recursive: false,
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
        let (_tmp, r) = setup();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec!["nope.txt".into()],
                recursive: false,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert!(!out.results[0].removed);
    }

    #[tokio::test]
    async fn refuses_non_accessible() {
        let (tmp, r) = setup();
        std::fs::write(tmp.path().join(".env"), "x").unwrap();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec![".env".into()],
                recursive: false,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert!(out.results[0].error.as_deref().unwrap().contains("C211"));
        assert!(tmp.path().join(".env").exists());
    }

    #[tokio::test]
    async fn directory_without_recursive_rejected() {
        let (tmp, r) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/x"), "x").unwrap();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec!["d".into()],
                recursive: false,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
    }

    #[tokio::test]
    async fn directory_with_recursive_succeeds() {
        let (tmp, r) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/x"), "x").unwrap();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec!["d".into()],
                recursive: true,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert!(!tmp.path().join("d").exists());
    }

    #[tokio::test]
    async fn recursive_refuses_when_subtree_has_non_accessible() {
        let (tmp, r) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        std::fs::write(tmp.path().join("d/.env"), "secret").unwrap();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec!["d".into()],
                recursive: true,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert!(out.results[0].error.as_deref().unwrap().contains("C211"));
        assert!(tmp.path().join("d/.env").exists());
    }

    #[tokio::test]
    async fn refuses_to_delete_base_root() {
        let (_tmp, r) = setup();
        let out = handle(
            r,
            DeleteFileInput {
                paths: vec![".".into()],
                recursive: true,
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert!(out.results[0].error.as_deref().unwrap().contains("C210"));
    }
}
