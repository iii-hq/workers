//! `coder::tree` — recursive directory snapshot, bounded by `max_depth`
//! and a `per_folder_limit`. Folders that hit the limit are tagged with
//! a `truncated` block pointing the caller at `coder::list-folder` for
//! pagination — matching the user's "if folder contains thousands of
//! files, it should show an indication it loaded only 50" requirement.
//!
//! Noise folders matching `default_exclude_globs` (.git, node_modules, …)
//! surface as childless stub nodes flagged `truncated` with reason
//! `default_exclude` — never silently hidden; opt out per call with
//! `use_default_excludes: false`. Nodes carry only `name`; absolute paths
//! derive from the response's top-level `path` (child = parent + "/" +
//! name), which cuts thousands of redundant tokens from large snapshots.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::CoderConfig;
use crate::error::{err_to_string, CoderError};
use crate::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_tree_input")]
pub struct TreeInput {
    /// Base folder for the snapshot. Relative to the primary allowed root,
    /// or an absolute path inside any allowed root. Defaults to `.` (the
    /// primary root itself). Call `coder::info` to see the allowed roots.
    /// Paths outside every allowed root are rejected — use the shell
    /// worker's `shell::fs::*` for host paths outside the jail.
    #[serde(default = "default_path")]
    pub path: String,
    /// Maximum depth to descend; the root node is depth 0.
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// Maximum children listed per folder. When more exist, the folder is
    /// flagged `truncated` and callers should switch to `coder::list-folder`.
    #[serde(default)]
    pub per_folder_limit: Option<u32>,
    /// Apply the worker's `default_exclude_globs` config (noise folders
    /// like .git/node_modules/target — call `coder::info` for the active
    /// list). Excluded directories still appear as childless nodes
    /// flagged `truncated` with reason "default_exclude"; excluded files
    /// are omitted. Pass `false` to list everything.
    #[serde(default = "default_true")]
    pub use_default_excludes: bool,
}

fn default_path() -> String {
    ".".to_string()
}

fn default_true() -> bool {
    true
}

// examples are wire-contract; goldens pin them.
fn example_tree_input() -> serde_json::Value {
    serde_json::json!({
        "path": ".",
        "max_depth": 3
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeOutput {
    /// Canonical absolute path of the requested folder (resolved through
    /// the jail). Nodes carry only `name`, and the root node's path IS
    /// this `path` — do not join the root's `name` onto it; derive
    /// children by joining from here: child path = parent path + "/" +
    /// name. Operations on derived paths re-validate through the jail.
    pub path: String,
    /// Root node of the snapshot; its `name` is the folder's basename.
    pub root: TreeNode,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeNode {
    /// Entry basename. The ROOT node's path is the response's top-level
    /// `path` itself; every other node's path derives by joining from
    /// there: child path = parent path + "/" + name.
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub mtime: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub non_accessible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
    /// Set on directories whose `children` was capped at
    /// `per_folder_limit`, whose subtree was cut off by `max_depth`, or
    /// which matched `default_exclude_globs` (reason "default_exclude").
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
    /// Reason this folder was truncated: hit `per_folder_limit`, cut off
    /// by `max_depth`, or matched `default_exclude_globs`
    /// (`default_exclude`).
    pub reason: String,
    /// Number of children actually returned.
    pub shown: u32,
    /// Total number of children eligible for listing in the folder,
    /// counted after default-exclude filtering (only populated when
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
    // NotFound is intercepted with the wire path in scope so the C211
    // message names the path the caller supplied (standardized wording —
    // REDACTION INVARIANT: identical to the glob-denied message).
    let md = std::fs::metadata(&abs).map_err(|e| CoderError::io_for_path(e, &req.path))?;
    if !md.is_dir() {
        return Err(CoderError::BadInput(format!(
            "not a directory: {}",
            req.path
        )));
    }
    // Explicitly naming an excluded folder as the walk root expresses
    // the caller's intent to see inside it: the default-exclude filter is
    // disabled for that ENTIRE walk. Anything less returns an
    // affirmatively false "empty" listing — direct file children omitted,
    // subdirectories stubbed one level down.
    let use_default_excludes = req.use_default_excludes && !resolver.is_default_excluded_dir(&abs);
    let opts = WalkOpts {
        max_depth: req.max_depth.unwrap_or(cfg.tree_default_depth),
        per_folder_limit: req
            .per_folder_limit
            .unwrap_or(cfg.tree_per_folder_limit)
            .max(1),
        use_default_excludes,
    };

    let root = walk_dir(resolver, &abs, 0, &opts)?;
    Ok(TreeOutput {
        path: abs.display().to_string(),
        root,
    })
}

struct WalkOpts {
    max_depth: u32,
    per_folder_limit: u32,
    use_default_excludes: bool,
}

fn walk_dir(
    resolver: &PathResolver,
    abs: &Path,
    depth: u32,
    opts: &WalkOpts,
) -> Result<TreeNode, CoderError> {
    // Deliberately bare `?` (generic From fallback, no path in the message):
    // `abs` here can be a DISCOVERED child during the walk — naming it would
    // violate the REDACTION INVARIANT. Do not "fix" this to io_for_path.
    let md = std::fs::metadata(abs)?;
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());

    let mut node = TreeNode {
        name,
        kind: NodeKind::Dir,
        size: md.len(),
        mtime: unix_mtime(&md),
        non_accessible: resolver.is_non_accessible(abs),
        children: None,
        truncated: None,
    };

    // `abs` is always a directory here (the walk only recurses into
    // dirs), so the dir-boundary check applies. The excluded node still
    // appears — never silently hidden. The root can't trip this: inner()
    // disables the filter when the requested root is itself excluded.
    if opts.use_default_excludes && resolver.is_default_excluded_dir(abs) {
        node.truncated = Some(TruncationInfo {
            reason: "default_exclude".to_string(),
            shown: 0,
            total: None,
            hint: "folder matches default_exclude_globs (coder::info lists them); \
                   re-call coder::tree with use_default_excludes: false to descend"
                .into(),
        });
        return Ok(node);
    }

    if depth >= opts.max_depth {
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
    if opts.use_default_excludes {
        // Excluded non-directory entries are omitted outright — matched
        // against the configured globs ONLY (no dir companions), so a
        // file or symlink merely NAMED like an excluded directory is
        // kept. Directories stay regardless; excluded ones surface as
        // childless stubs in the recursive call.
        entries.retain(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            is_dir || !resolver.is_default_excluded(&e.path())
        });
    }
    entries.sort_by_key(|a| a.file_name());

    let total = entries.len() as u32;
    let cap = opts.per_folder_limit as usize;
    let truncated_here = total as usize > cap;
    let visible = if truncated_here {
        &entries[..cap]
    } else {
        &entries[..]
    };

    let mut children = Vec::with_capacity(visible.len());
    for e in visible {
        let child_abs = e.path();
        let ft = e.file_type().ok();
        if ft.as_ref().is_some_and(|t| t.is_dir()) {
            let sub = walk_dir(resolver, &child_abs, depth + 1, opts)?;
            children.push(sub);
        } else {
            let cmd = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            children.push(TreeNode {
                name: e.file_name().to_string_lossy().into_owned(),
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
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            tree_default_depth: 4,
            tree_per_folder_limit: 50,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    fn input(path: &str) -> TreeInput {
        TreeInput {
            path: path.into(),
            max_depth: None,
            per_folder_limit: None,
            use_default_excludes: true,
        }
    }

    #[tokio::test]
    async fn tree_with_nested_dirs() {
        let (tmp, r, c) = setup();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/c.txt"), "hi").unwrap();
        std::fs::write(tmp.path().join("z.txt"), "x").unwrap();

        let out = handle(r, c, input(".")).await.unwrap();
        let root = &out.root;
        assert!(matches!(root.kind, NodeKind::Dir));
        let children = root.children.as_ref().unwrap();
        let names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "z.txt"]);
        let a = &children[0];
        let a_children = a.children.as_ref().unwrap();
        assert_eq!(a_children[0].name, "b");
        let b_children = a_children[0].children.as_ref().unwrap();
        assert_eq!(b_children[0].name, "c.txt");
        // The response's top-level path is canonical-absolute (decision
        // D2-eng); nodes carry only names.
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(out.path, base.display().to_string());
        // WIRE-CONTRACT PIN: the documented derivation rule (child path =
        // parent path + "/" + name) must reproduce the real fs path.
        let derived = format!(
            "{}/{}/{}/{}",
            out.path, a.name, a_children[0].name, b_children[0].name
        );
        assert_eq!(derived, base.join("a/b/c.txt").display().to_string());
    }

    #[tokio::test]
    async fn max_depth_truncates_subtree() {
        let (tmp, r, _c) = setup();
        std::fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        std::fs::write(tmp.path().join("a/b/c/x.txt"), "x").unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            tree_default_depth: 1,
            tree_per_folder_limit: 50,
            ..CoderConfig::default()
        });
        let out = handle(r, cfg, input(".")).await.unwrap();
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
            base_paths: vec![tmp.path().to_path_buf()],
            tree_default_depth: 4,
            tree_per_folder_limit: 3,
            ..CoderConfig::default()
        });
        let out = handle(r, cfg, input(".")).await.unwrap();
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
        let out = handle(r, c, input(".")).await.unwrap();
        let kids = out.root.children.unwrap();
        let env = kids.iter().find(|k| k.name == ".env").unwrap();
        assert!(env.non_accessible);
        let a = kids.iter().find(|k| k.name == "a.txt").unwrap();
        assert!(!a.non_accessible);
    }

    #[tokio::test]
    async fn default_excluded_dir_appears_as_childless_stub() {
        let (tmp, r, c) = setup();
        std::fs::create_dir_all(tmp.path().join("node_modules/pkg")).unwrap();
        std::fs::write(tmp.path().join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::write(tmp.path().join("main.rs"), "x").unwrap();

        let out = handle(r, c, input(".")).await.unwrap();
        let kids = out.root.children.unwrap();
        let nm = kids
            .iter()
            .find(|k| k.name == "node_modules")
            .expect("excluded dir must still appear, never silently hidden");
        assert!(matches!(nm.kind, NodeKind::Dir));
        assert!(nm.children.is_none(), "descent must be suppressed");
        let trunc = nm.truncated.as_ref().unwrap();
        assert_eq!(trunc.reason, "default_exclude");
        assert_eq!(trunc.shown, 0);
        assert_eq!(trunc.total, None);
        assert!(
            trunc.hint.contains("use_default_excludes"),
            "hint must teach the opt-out: {}",
            trunc.hint
        );
    }

    #[tokio::test]
    async fn use_default_excludes_false_descends_into_excluded_dirs() {
        let (tmp, r, c) = setup();
        std::fs::create_dir_all(tmp.path().join("node_modules/pkg")).unwrap();
        std::fs::write(tmp.path().join("node_modules/pkg/index.js"), "x").unwrap();

        let out = handle(
            r,
            c,
            TreeInput {
                use_default_excludes: false,
                ..input(".")
            },
        )
        .await
        .unwrap();
        let kids = out.root.children.unwrap();
        let nm = kids.iter().find(|k| k.name == "node_modules").unwrap();
        assert!(nm.truncated.is_none());
        let nm_kids = nm.children.as_ref().expect("opt-out must descend");
        assert_eq!(nm_kids[0].name, "pkg");
    }

    #[tokio::test]
    async fn default_excluded_file_omitted_from_listing() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("debug.log"), "x").unwrap();
        std::fs::write(tmp.path().join("keep.txt"), "x").unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            default_exclude_globs: vec!["**/*.log".to_string()],
            ..CoderConfig::default()
        });
        let r = Arc::new(PathResolver::new(&cfg).unwrap());
        let out = handle(r, cfg, input(".")).await.unwrap();
        let kids = out.root.children.unwrap();
        let names: Vec<_> = kids.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, vec!["keep.txt"]);
        assert!(
            out.root.truncated.is_none(),
            "omitted files must not count toward per_folder_limit truncation"
        );
    }

    fn assert_no_default_exclude_stubs(node: &TreeNode) {
        if let Some(t) = &node.truncated {
            assert_ne!(
                t.reason, "default_exclude",
                "unexpected default_exclude stub at node {:?}",
                node.name
            );
        }
        for child in node.children.iter().flatten() {
            assert_no_default_exclude_stubs(child);
        }
    }

    #[tokio::test]
    async fn explicitly_requested_excluded_root_disables_filter_for_whole_walk() {
        let (tmp, r, c) = setup();
        std::fs::create_dir_all(tmp.path().join("node_modules/pkg")).unwrap();
        std::fs::write(tmp.path().join("node_modules/.package-lock.json"), "x").unwrap();
        std::fs::write(tmp.path().join("node_modules/pkg/index.js"), "x").unwrap();

        let out = handle(r, c, input("node_modules")).await.unwrap();
        assert!(out.root.truncated.is_none());
        let kids = out.root.children.as_ref().expect("explicit root must list");
        let names: Vec<_> = kids.iter().map(|k| k.name.as_str()).collect();
        // File children directly inside the excluded root must be visible…
        assert_eq!(names, vec![".package-lock.json", "pkg"]);
        // …and subdirectories must actually descend, not stub one level down.
        let pkg = kids.iter().find(|k| k.name == "pkg").unwrap();
        assert!(pkg.truncated.is_none());
        assert_eq!(pkg.children.as_ref().unwrap()[0].name, "index.js");
        assert_no_default_exclude_stubs(&out.root);
    }

    #[tokio::test]
    async fn entries_merely_named_like_excluded_dirs_are_kept_as_leaves() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("real")).unwrap();
        std::fs::write(tmp.path().join("dist"), "not a dir").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("node_modules"))
            .unwrap();

        let out = handle(r, c, input(".")).await.unwrap();
        let kids = out.root.children.unwrap();
        let dist = kids
            .iter()
            .find(|k| k.name == "dist")
            .expect("a FILE named dist must not be dropped by the dir companion");
        assert!(matches!(dist.kind, NodeKind::File));
        assert!(dist.truncated.is_none());
        let nm = kids
            .iter()
            .find(|k| k.name == "node_modules")
            .expect("a SYMLINK named node_modules must not be dropped");
        assert!(matches!(nm.kind, NodeKind::Symlink));
        assert!(nm.truncated.is_none());
    }
}
