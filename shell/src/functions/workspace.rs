use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::ShellConfig;
use crate::fs::error::FsError;
use crate::path::canonicalize_with_fallback;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkspaceRootsRequest {}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceRootsResponse {
    pub roots: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkspaceValidateRequest {
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceValidateResponse {
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkspaceListRequest {
    pub path: String,
    #[serde(default)]
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceEntryKind {
    Dir,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceEntryKind,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceListResponse {
    pub path: String,
    pub entries: Vec<WorkspaceEntry>,
}

pub fn workspace_roots(cfg: &ShellConfig) -> WorkspaceRootsResponse {
    let mut candidates = cfg.fs.roots();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home));
    }
    candidates.push(PathBuf::from("/"));

    let mut roots = BTreeSet::new();
    for candidate in candidates {
        let Ok(canon) = std::fs::canonicalize(&candidate) else {
            continue;
        };
        if !canon.is_dir() || is_denylisted(&canon, cfg) {
            continue;
        }
        roots.insert(path_string(&canon));
    }
    WorkspaceRootsResponse {
        roots: roots.into_iter().collect(),
    }
}

pub fn validate_workspace_path(
    req: WorkspaceValidateRequest,
    cfg: &ShellConfig,
) -> Result<WorkspaceValidateResponse, FsError> {
    let path = canonical_workspace_dir(&req.path, cfg)?;
    Ok(WorkspaceValidateResponse {
        path: path_string(&path),
    })
}

pub fn list_workspace_dirs(
    req: WorkspaceListRequest,
    cfg: &ShellConfig,
) -> Result<WorkspaceListResponse, FsError> {
    let root = canonical_workspace_dir(&req.path, cfg)?;
    let page_size = req.page_size.unwrap_or(200).clamp(1, 1_000);
    let rd = std::fs::read_dir(&root).map_err(|e| FsError::from_io(&req.path, e))?;
    let mut entries = Vec::new();
    for ent in rd {
        let Ok(ent) = ent else {
            continue;
        };
        let name = ent.file_name().to_string_lossy().into_owned();
        let path = ent.path();
        let Ok(md) = std::fs::metadata(&path) else {
            continue;
        };
        if !md.is_dir() {
            continue;
        }
        let Ok(canon) = std::fs::canonicalize(&path) else {
            continue;
        };
        if is_denylisted(&canon, cfg) {
            continue;
        }
        entries.push(WorkspaceEntry {
            name,
            path: path_string(&canon),
            kind: WorkspaceEntryKind::Dir,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries.truncate(page_size);
    Ok(WorkspaceListResponse {
        path: path_string(&root),
        entries,
    })
}

fn canonical_workspace_dir(path: &str, cfg: &ShellConfig) -> Result<PathBuf, FsError> {
    if path.is_empty() {
        return Err(FsError::new("S210", "path must not be empty"));
    }
    let canon = std::fs::canonicalize(path).map_err(|e| FsError::from_io(path, e))?;
    if !canon.is_dir() {
        return Err(FsError::new("S212", format!("not a directory: {path}")));
    }
    if is_denylisted(&canon, cfg) {
        return Err(FsError::new("S215", format!("path is denylisted: {path}")));
    }
    Ok(canon)
}

fn is_denylisted(path: &Path, cfg: &ShellConfig) -> bool {
    cfg.fs.denylist_paths.iter().any(|deny| {
        canonicalize_with_fallback(deny)
            .map(|canon| path.starts_with(canon))
            .unwrap_or(false)
    })
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn cfg_with_root(root: PathBuf) -> ShellConfig {
        let mut cfg = ShellConfig::default();
        cfg.fs.host_roots = vec![root];
        cfg
    }

    fn canon(p: &Path) -> String {
        std::fs::canonicalize(p)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn roots_include_configured_host_root_and_operator_anchors() {
        let jail = tempdir().unwrap();
        let cfg = cfg_with_root(jail.path().to_path_buf());

        let roots = workspace_roots(&cfg).roots;

        assert!(roots.contains(&canon(jail.path())));
        assert!(roots.iter().any(|r| r == "/"));
    }

    #[test]
    fn validate_accepts_existing_directory_outside_configured_roots() {
        let jail = tempdir().unwrap();
        let selected = tempdir().unwrap();
        let cfg = cfg_with_root(jail.path().to_path_buf());

        let resp = validate_workspace_path(
            WorkspaceValidateRequest {
                path: selected.path().to_string_lossy().into_owned(),
            },
            &cfg,
        )
        .unwrap();

        assert_eq!(resp.path, canon(selected.path()));
    }

    #[test]
    fn validate_rejects_denylisted_directory() {
        let jail = tempdir().unwrap();
        let denied = tempdir().unwrap();
        let mut cfg = cfg_with_root(jail.path().to_path_buf());
        cfg.fs.denylist_paths = vec![denied.path().to_path_buf()];

        let err = validate_workspace_path(
            WorkspaceValidateRequest {
                path: denied.path().to_string_lossy().into_owned(),
            },
            &cfg,
        )
        .unwrap_err();

        assert_eq!(err.code, "S215");
    }

    #[test]
    fn list_returns_directories_only_sorted_with_canonical_paths() {
        let jail = tempdir().unwrap();
        let selected = tempdir().unwrap();
        std::fs::create_dir(selected.path().join("b")).unwrap();
        std::fs::create_dir(selected.path().join("a")).unwrap();
        std::fs::write(selected.path().join("file.txt"), "x").unwrap();
        let cfg = cfg_with_root(jail.path().to_path_buf());

        let resp = list_workspace_dirs(
            WorkspaceListRequest {
                path: selected.path().to_string_lossy().into_owned(),
                page_size: Some(20),
            },
            &cfg,
        )
        .unwrap();

        assert_eq!(resp.path, canon(selected.path()));
        assert_eq!(
            resp.entries,
            vec![
                WorkspaceEntry {
                    name: "a".into(),
                    path: canon(&selected.path().join("a")),
                    kind: WorkspaceEntryKind::Dir,
                },
                WorkspaceEntry {
                    name: "b".into(),
                    path: canon(&selected.path().join("b")),
                    kind: WorkspaceEntryKind::Dir,
                },
            ]
        );
    }
}
