//! `coder::tree` — recursive directory snapshot, bounded by `max_depth`
//! and a `per_folder_limit`. Folders that hit the limit are tagged with
//! a `truncated` block pointing the caller at `coder::list-folder` for
//! pagination — matching the user's "if folder contains thousands of
//! files, it should show an indication it loaded only 50" requirement.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::CoderConfig;
use crate::error::{err_to_string, CoderError};
use crate::path::PathResolver;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TreeInput {
    /// Base folder relative to `base_path`. Defaults to `.`.
    #[serde(default = "default_path")]
    pub path: String,
    /// Maximum depth to descend; the root node is depth 0.
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// Maximum children listed per folder. When more exist, the folder is
    /// flagged `truncated` and callers should switch to `coder::list-folder`.
    #[serde(default)]
    pub per_folder_limit: Option<u32>,
}

fn default_path() -> String {
    ".".to_string()
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeOutput {
    pub root: TreeNode,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    pub size: u64,
    pub mtime: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub non_accessible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
    /// Set on directories whose `children` was capped at `per_folder_limit`
    /// or whose subtree was cut off by `max_depth`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<TruncationInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TruncationInfo {
    /// Reason this folder was truncated: hit `per_folder_limit` or
    /// `max_depth`.
    pub reason: String,
    /// Number of children actually returned.
    pub shown: u32,
    /// Total number of children in the folder (only populated when
    /// `reason == "per_folder_limit"`; for depth truncation we don't
    /// peek into the folder).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    pub hint: String,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: TreeInput,
) -> Result<TreeOutput, String> {
    inner(&resolver, &cfg, req).map_err(err_to_string)
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: TreeInput,
) -> Result<TreeOutput, CoderError> {
    let abs = resolver.resolve(&req.path)?;
    let md = std::fs::metadata(&abs)?;
    if !md.is_dir() {
        return Err(CoderError::BadInput(format!(
            "not a directory: {}",
            req.path
        )));
    }
    let max_depth = req.max_depth.unwrap_or(cfg.tree_default_depth);
    let per_folder_limit = req
        .per_folder_limit
        .unwrap_or(cfg.tree_per_folder_limit)
        .max(1);

    let root_rel = resolver.relative(&abs).unwrap_or_default();
    let root = walk_dir(resolver, &abs, root_rel, 0, max_depth, per_folder_limit)?;
    Ok(TreeOutput { root })
}

fn walk_dir(
    resolver: &PathResolver,
    abs: &Path,
    rel: String,
    depth: u32,
    max_depth: u32,
    per_folder_limit: u32,
) -> Result<TreeNode, CoderError> {
    let md = std::fs::metadata(abs)?;
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());

    let mut node = TreeNode {
        name,
        path: rel.clone(),
        kind: NodeKind::Dir,
        size: md.len(),
        mtime: unix_mtime(&md),
        non_accessible: resolver.is_non_accessible(abs),
        children: None,
        truncated: None,
    };

    if depth >= max_depth {
        node.truncated = Some(TruncationInfo {
            reason: "max_depth".to_string(),
            shown: 0,
            total: None,
            hint: "raise max_depth or call coder::tree with this path as the new root".into(),
        });
        return Ok(node);
    }

    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(abs) {
        Ok(it) => it.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            // Surface as a node with no children rather than failing the whole tree.
            node.truncated = Some(TruncationInfo {
                reason: "io_error".to_string(),
                shown: 0,
                total: None,
                hint: format!("read_dir failed: {e}"),
            });
            return Ok(node);
        }
    };
    entries.sort_by_key(|a| a.file_name());

    let total = entries.len() as u32;
    let cap = per_folder_limit as usize;
    let truncated_here = total as usize > cap;
    let visible = if truncated_here {
        &entries[..cap]
    } else {
        &entries[..]
    };

    let mut children = Vec::with_capacity(visible.len());
    for e in visible {
        let child_abs = e.path();
        let child_rel = if rel.is_empty() {
            e.file_name().to_string_lossy().into_owned()
        } else {
            format!("{}/{}", rel, e.file_name().to_string_lossy())
        };
        let ft = e.file_type().ok();
        if ft.as_ref().is_some_and(|t| t.is_dir()) {
            let sub = walk_dir(
                resolver,
                &child_abs,
                child_rel,
                depth + 1,
                max_depth,
                per_folder_limit,
            )?;
            children.push(sub);
        } else {
            let cmd = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            children.push(TreeNode {
                name: e.file_name().to_string_lossy().into_owned(),
                path: child_rel,
                kind: classify(&cmd),
                size: cmd.len(),
                mtime: unix_mtime(&cmd),
                non_accessible: resolver.is_non_accessible(&child_abs),
                children: None,
                truncated: None,
            });
        }
    }
    node.children = Some(children);
    if truncated_here {
        node.truncated = Some(TruncationInfo {
            reason: "per_folder_limit".to_string(),
            shown: cap as u32,
            total: Some(total),
            hint: "use coder::list-folder for paginated access to all entries".into(),
        });
    }
    Ok(node)
}

fn classify(md: &std::fs::Metadata) -> NodeKind {
    let ft = md.file_type();
    if ft.is_symlink() {
        NodeKind::Symlink
    } else if ft.is_dir() {
        NodeKind::Dir
    } else if ft.is_file() {
        NodeKind::File
    } else {
        NodeKind::Other
    }
}

fn unix_mtime(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_path: tmp.path().to_path_buf(),
            non_accessible_globs: vec!["**/.env".to_string()],
            tree_default_depth: 4,
            tree_per_folder_limit: 50,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn tree_with_nested_dirs() {
        let (tmp, r, c) = setup();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/c.txt"), "hi").unwrap();
        std::fs::write(tmp.path().join("z.txt"), "x").unwrap();

        let out = handle(
            r,
            c,
            TreeInput {
                path: ".".into(),
                max_depth: None,
                per_folder_limit: None,
            },
        )
        .await
        .unwrap();
        let root = out.root;
        assert!(matches!(root.kind, NodeKind::Dir));
        let children = root.children.unwrap();
        let names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "z.txt"]);
        let a = &children[0];
        let a_children = a.children.as_ref().unwrap();
        assert_eq!(a_children[0].name, "b");
        let b_children = a_children[0].children.as_ref().unwrap();
        assert_eq!(b_children[0].name, "c.txt");
    }

    #[tokio::test]
    async fn max_depth_truncates_subtree() {
        let (tmp, r, _c) = setup();
        std::fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        std::fs::write(tmp.path().join("a/b/c/x.txt"), "x").unwrap();
        let cfg = Arc::new(CoderConfig {
            base_path: tmp.path().to_path_buf(),
            tree_default_depth: 1,
            tree_per_folder_limit: 50,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            cfg,
            TreeInput {
                path: ".".into(),
                max_depth: None,
                per_folder_limit: None,
            },
        )
        .await
        .unwrap();
        let a = &out.root.children.unwrap()[0];
        // a is depth 1, which equals max_depth → should be truncated, no children loaded.
        assert!(a.children.is_none());
        let trunc = a.truncated.as_ref().unwrap();
        assert_eq!(trunc.reason, "max_depth");
    }

    #[tokio::test]
    async fn per_folder_limit_truncates_with_total() {
        let (tmp, r, _c) = setup();
        for i in 0..10 {
            std::fs::write(tmp.path().join(format!("f{i:02}.txt")), "x").unwrap();
        }
        let cfg = Arc::new(CoderConfig {
            base_path: tmp.path().to_path_buf(),
            tree_default_depth: 4,
            tree_per_folder_limit: 3,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            cfg,
            TreeInput {
                path: ".".into(),
                max_depth: None,
                per_folder_limit: None,
            },
        )
        .await
        .unwrap();
        let kids = out.root.children.as_ref().unwrap();
        assert_eq!(kids.len(), 3);
        let trunc = out.root.truncated.as_ref().unwrap();
        assert_eq!(trunc.reason, "per_folder_limit");
        assert_eq!(trunc.shown, 3);
        assert_eq!(trunc.total, Some(10));
        assert!(trunc.hint.contains("coder::list-folder"));
    }

    #[tokio::test]
    async fn non_accessible_flag_set_on_matching_entries() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), "x").unwrap();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let out = handle(
            r,
            c,
            TreeInput {
                path: ".".into(),
                max_depth: None,
                per_folder_limit: None,
            },
        )
        .await
        .unwrap();
        let kids = out.root.children.unwrap();
        let env = kids.iter().find(|k| k.name == ".env").unwrap();
        assert!(env.non_accessible);
        let a = kids.iter().find(|k| k.name == "a.txt").unwrap();
        assert!(!a.non_accessible);
    }
}
