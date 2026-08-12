//! `coder::tree` — recursive directory snapshot, bounded by `max_depth`
//! and a `per_folder_limit`. Folders that hit the limit are tagged with
//! a `truncated` block pointing the caller at `coder::list-folder` for
//! pagination — matching the user's "if folder contains thousands of
//! files, it should show an indication it loaded only 50" requirement.
//!
//! Noise folders matching `default_exclude_globs` (.git, node_modules, …)
//! surface as childless stub nodes flagged `truncated` with reason
//! `default_exclude` — never silently hidden; opt out per call with
//! `use_default_excludes: false`. `include_hidden: false` omits dot
//! entries outright, before the per-folder cap is counted, so the cap
//! serves visible names. A total node budget ([`TREE_NODE_BUDGET`])
//! bounds every snapshot — folders reached after it is spent are
//! flagged `truncated` with reason `max_nodes`. Nodes carry only
//! `name`; absolute paths derive from the response's top-level `path`
//! (child = parent + "/" + name), which cuts thousands of redundant
//! tokens from large snapshots.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::path::PathResolver;

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
    /// List hidden (dot-prefixed) entries. Pass `false` to omit them —
    /// files and folders alike — at every level; omitted entries do not
    /// count toward `per_folder_limit`, so the cap serves visible names.
    /// The requested root itself is exempt: explicitly naming a hidden
    /// folder still lists its contents.
    #[serde(default = "default_true")]
    pub include_hidden: bool,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
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
    /// `per_folder_limit`, whose subtree was cut off by `max_depth`,
    /// which matched `default_exclude_globs` (reason "default_exclude"),
    /// or where the snapshot's node budget ran out (reason "max_nodes").
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
    /// by `max_depth`, matched `default_exclude_globs`
    /// (`default_exclude`), or the snapshot's total node budget ran out
    /// (`max_nodes`).
    pub reason: String,
    /// Number of children actually returned.
    pub shown: u32,
    /// Total number of children eligible for listing in the folder,
    /// counted after hidden and default-exclude filtering (only
    /// populated when `reason == "per_folder_limit"`; for depth
    /// truncation we don't peek into the folder).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    pub hint: String,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: TreeInput,
) -> Result<TreeOutput, String> {
    // Offload the synchronous recursive walk to a blocking thread so a large
    // tree can't stall the shared runtime that also dispatches shell::exec/jobs.
    tokio::task::spawn_blocking(move || inner(&resolver, &cfg, req).map_err(err_to_string))
        .await
        .map_err(|e| format!("tree task join failed: {e}"))?
}

/// Total nodes any snapshot may carry. Without it a wide-and-deep root
/// (a home directory) walks for tens of seconds and produces a response
/// large enough to kill the worker's engine socket — the engine stops
/// the invocation at its own timeout, the worker still tries to flush
/// the giant frame, and the connection loops on broken pipes. ~20k
/// nodes is a few MB of JSON and a sub-second walk.
const TREE_NODE_BUDGET: u32 = 20_000;

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: TreeInput,
) -> Result<TreeOutput, CoderError> {
    let abs = resolver.resolve_scope(req.fs_scope.as_ref(), &req.path)?;
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
        include_hidden: req.include_hidden,
    };

    let root = walk(resolver, &abs, &opts, TREE_NODE_BUDGET)?;
    Ok(TreeOutput {
        path: abs.display().to_string(),
        root,
    })
}

struct WalkOpts {
    max_depth: u32,
    per_folder_limit: u32,
    use_default_excludes: bool,
    include_hidden: bool,
}

/// A directory waiting for its listing pass, pointing at its arena slot.
struct PendingDir {
    slot: usize,
    abs: std::path::PathBuf,
    depth: u32,
}

fn max_nodes_truncation(shown: u32, total: Option<u32>) -> TruncationInfo {
    TruncationInfo {
        reason: "max_nodes".to_string(),
        shown,
        total,
        hint: "snapshot node budget exhausted; re-call coder::tree rooted at a \
               subfolder, or use coder::list-folder for paginated access"
            .into(),
    }
}

/// Breadth-first walk under the global node budget. Level order is the
/// point: when the budget cannot cover the whole tree, it is spent on
/// the SHALLOW entries first, so every folder the caller can actually
/// see lists completely and only deep subtrees come back as `max_nodes`
/// stubs. A depth-first walk under the same budget starves later
/// siblings — the first big subtree eats the budget and the root
/// listing itself comes back short.
///
/// Nodes live in an arena where parents always precede their children;
/// a reverse pass at the end attaches fully-built subtrees.
fn walk(
    resolver: &PathResolver,
    root_abs: &Path,
    opts: &WalkOpts,
    mut budget: u32,
) -> Result<TreeNode, CoderError> {
    // Deliberately bare `?` (generic From fallback, no path in the message):
    // naming a walked path here would violate the REDACTION INVARIANT.
    // Do not "fix" this to io_for_path.
    let md = std::fs::metadata(root_abs)?;
    let name = root_abs
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());

    let mut slots: Vec<Option<TreeNode>> = vec![Some(TreeNode {
        name,
        kind: NodeKind::Dir,
        size: md.len(),
        mtime: unix_mtime(&md),
        non_accessible: resolver.is_non_accessible(root_abs),
        children: None,
        truncated: None,
    })];
    // Children (in listing order) to attach to each slot, and whether
    // the slot's dir got a listing pass (stubbed dirs and files keep
    // `children: None`).
    let mut child_slots: Vec<Vec<usize>> = vec![Vec::new()];
    let mut listed: Vec<bool> = vec![false];
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(PendingDir {
        slot: 0,
        abs: root_abs.to_path_buf(),
        depth: 0,
    });

    while let Some(PendingDir { slot, abs, depth }) = queue.pop_front() {
        let set_truncated = |slots: &mut Vec<Option<TreeNode>>, info: TruncationInfo| {
            if let Some(node) = slots[slot].as_mut() {
                node.truncated = Some(info);
            }
        };

        // `abs` is always a directory here (only dirs are queued), so
        // the dir-boundary check applies. The excluded node still
        // appears — never silently hidden. The root can't trip this:
        // inner() disables the filter when the requested root is itself
        // excluded.
        if opts.use_default_excludes && resolver.is_default_excluded_dir(&abs) {
            set_truncated(
                &mut slots,
                TruncationInfo {
                    reason: "default_exclude".to_string(),
                    shown: 0,
                    total: None,
                    hint: "folder matches default_exclude_globs (coder::info lists them); \
                           re-call coder::tree with use_default_excludes: false to descend"
                        .into(),
                },
            );
            continue;
        }

        if depth >= opts.max_depth {
            set_truncated(
                &mut slots,
                TruncationInfo {
                    reason: "max_depth".to_string(),
                    shown: 0,
                    total: None,
                    hint: "raise max_depth or call coder::tree with this path as the new root"
                        .into(),
                },
            );
            continue;
        }

        // Discovered before the budget ran out, dequeued after: stub it
        // without reading the directory at all.
        if budget == 0 {
            set_truncated(&mut slots, max_nodes_truncation(0, None));
            continue;
        }

        let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(&abs) {
            Ok(it) => it.filter_map(|e| e.ok()).collect(),
            Err(e) => {
                // Surface as a node with no children rather than failing the whole tree.
                set_truncated(
                    &mut slots,
                    TruncationInfo {
                        reason: "io_error".to_string(),
                        shown: 0,
                        total: None,
                        hint: format!("read_dir failed: {e}"),
                    },
                );
                continue;
            }
        };
        if !opts.include_hidden {
            // Dot entries drop out BEFORE the per-folder cap is counted:
            // byte order sorts "." names first, so in a home-shaped folder
            // they would otherwise fill the cap by themselves and every
            // visible name would land in the truncated remainder.
            entries.retain(|e| !e.file_name().to_string_lossy().starts_with('.'));
        }
        if opts.use_default_excludes {
            // Excluded non-directory entries are omitted outright — matched
            // against the configured globs ONLY (no dir companions), so a
            // file or symlink merely NAMED like an excluded directory is
            // kept. Directories stay regardless; excluded ones surface as
            // childless stubs on their own listing pass.
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

        let mut budget_hit = false;
        for e in visible {
            if budget == 0 {
                budget_hit = true;
                break;
            }
            budget -= 1;
            let child_abs = e.path();
            // Entries whose metadata vanishes mid-walk (unlink race) are
            // skipped rather than failing the whole snapshot.
            let cmd = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            slots.push(Some(TreeNode {
                name: e.file_name().to_string_lossy().into_owned(),
                kind: if is_dir {
                    NodeKind::Dir
                } else {
                    classify(&cmd)
                },
                size: cmd.len(),
                mtime: unix_mtime(&cmd),
                non_accessible: resolver.is_non_accessible(&child_abs),
                children: None,
                truncated: None,
            }));
            child_slots.push(Vec::new());
            listed.push(false);
            let child = slots.len() - 1;
            child_slots[slot].push(child);
            if is_dir {
                queue.push_back(PendingDir {
                    slot: child,
                    abs: child_abs,
                    depth: depth + 1,
                });
            }
        }
        let shown = child_slots[slot].len() as u32;
        listed[slot] = true;
        if budget_hit {
            set_truncated(&mut slots, max_nodes_truncation(shown, Some(total)));
        } else if truncated_here {
            set_truncated(
                &mut slots,
                TruncationInfo {
                    reason: "per_folder_limit".to_string(),
                    shown: cap as u32,
                    total: Some(total),
                    hint: "use coder::list-folder for paginated access to all entries".into(),
                },
            );
        }
    }

    // Assemble bottom-up: children always sit at higher indices than
    // their parent, so a reverse pass attaches fully-built subtrees.
    for i in (0..slots.len()).rev() {
        if !listed[i] {
            continue;
        }
        let ids = std::mem::take(&mut child_slots[i]);
        let children: Vec<TreeNode> = ids
            .into_iter()
            .map(|j| slots[j].take().expect("child slot taken once"))
            .collect();
        if let Some(node) = slots[i].as_mut() {
            node.children = Some(children);
        }
    }
    Ok(slots[0].take().expect("root slot"))
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
            include_hidden: true,
            fs_scope: None,
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
    async fn rooting_at_subpath_returns_only_that_subtree() {
        // Rooting the tree at a subdirectory must return that subtree's
        // contents and EXCLUDE everything outside it — the negative
        // assertion the dropped tree BDD scenario carried.
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/inside.txt"), "in").unwrap();
        std::fs::write(tmp.path().join("outside.txt"), "out").unwrap();

        let out = handle(r, c, input("sub")).await.unwrap();
        let names: Vec<_> = out
            .root
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["inside.txt"],
            "rooting at 'sub' must list only its contents"
        );
        // The outside sibling must not appear anywhere in the rooted tree.
        assert!(
            !subtree_contains(&out.root, "outside.txt"),
            "outside.txt must be excluded when rooted at 'sub'"
        );
    }

    /// Recursively test whether any node in the tree is named `name`.
    fn subtree_contains(node: &TreeNode, name: &str) -> bool {
        if node.name == name {
            return true;
        }
        node.children
            .as_ref()
            .map(|kids| kids.iter().any(|k| subtree_contains(k, name)))
            .unwrap_or(false)
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

    /// The node budget must stop a walk and say so — a snapshot cut
    /// short by it has to carry `max_nodes` stubs, never pass itself
    /// off as complete. Exercised through `walk` directly so the test
    /// doesn't need [`TREE_NODE_BUDGET`]-many real files.
    #[tokio::test]
    async fn node_budget_stops_the_walk_and_flags_max_nodes() {
        let (tmp, r, _c) = setup();
        for i in 0..6 {
            std::fs::write(tmp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let opts = WalkOpts {
            max_depth: 4,
            per_folder_limit: 50,
            use_default_excludes: true,
            include_hidden: true,
        };
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        let node = walk(&r, &base, &opts, 4).unwrap();
        assert_eq!(node.children.as_ref().unwrap().len(), 4);
        let trunc = node.truncated.as_ref().unwrap();
        assert_eq!(trunc.reason, "max_nodes");
        assert_eq!(trunc.shown, 4);
        assert_eq!(trunc.total, Some(6));
        assert!(trunc.hint.contains("coder::list-folder"));
    }

    /// The budget is spent breadth-first: every shallow entry lists
    /// before any deep subtree spends a node, so a big early-alphabet
    /// folder can't starve its siblings out of the root listing — the
    /// failure mode that made a depth-first budget return 7 of a home
    /// directory's 123 visible folders.
    #[tokio::test]
    async fn node_budget_spends_breadth_first_so_shallow_entries_win() {
        let (tmp, r, _c) = setup();
        std::fs::create_dir(tmp.path().join("aa")).unwrap();
        for i in 0..3 {
            std::fs::write(tmp.path().join(format!("aa/f{i}.txt")), "x").unwrap();
        }
        std::fs::create_dir(tmp.path().join("zz")).unwrap();
        std::fs::write(tmp.path().join("zz/late.txt"), "x").unwrap();
        let opts = WalkOpts {
            max_depth: 4,
            per_folder_limit: 50,
            use_default_excludes: true,
            include_hidden: true,
        };
        // 5 nodes: aa + zz (the whole level), then aa/f0..f2 — zz is
        // dequeued with the budget spent and must come back a stub.
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        let node = walk(&r, &base, &opts, 5).unwrap();
        assert!(
            node.truncated.is_none(),
            "the root listed every direct child"
        );
        let kids = node.children.as_ref().unwrap();
        let names: Vec<_> = kids.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, vec!["aa", "zz"]);
        let aa = kids.iter().find(|k| k.name == "aa").unwrap();
        assert_eq!(aa.children.as_ref().unwrap().len(), 3);
        assert!(aa.truncated.is_none());
        let zz = kids.iter().find(|k| k.name == "zz").unwrap();
        let trunc = zz.truncated.as_ref().unwrap();
        assert_eq!(trunc.reason, "max_nodes");
        assert_eq!(trunc.shown, 0);
        assert!(
            zz.children.is_none(),
            "a budget stub never got a listing pass"
        );
    }

    /// The home-directory shape that motivated the flag: enough dot
    /// entries to fill the per-folder cap by themselves (byte order
    /// sorts them first), with the visible names behind them. The filter
    /// must both omit the dots and stop them consuming the cap.
    #[tokio::test]
    async fn include_hidden_false_omits_dot_entries_and_frees_the_cap() {
        let (tmp, r, _c) = setup();
        for i in 0..5 {
            std::fs::create_dir(tmp.path().join(format!(".h{i}"))).unwrap();
        }
        std::fs::write(tmp.path().join(".hidden.txt"), "x").unwrap();
        std::fs::create_dir(tmp.path().join("projects")).unwrap();
        std::fs::write(tmp.path().join("readme.md"), "x").unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            tree_default_depth: 4,
            tree_per_folder_limit: 3,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            cfg,
            TreeInput {
                include_hidden: false,
                ..input(".")
            },
        )
        .await
        .unwrap();
        let kids = out.root.children.as_ref().unwrap();
        let names: Vec<_> = kids.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, vec!["projects", "readme.md"]);
        assert!(
            out.root.truncated.is_none(),
            "omitted hidden entries must not count toward per_folder_limit"
        );
    }

    /// Hidden entries filter at EVERY level, not just the root's own
    /// children — and the default (`include_hidden: true`) keeps them.
    #[tokio::test]
    async fn hidden_filter_applies_at_depth_and_defaults_to_listing() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/.cache"), "x").unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "x").unwrap();

        let filtered = handle(
            r.clone(),
            c.clone(),
            TreeInput {
                include_hidden: false,
                ..input(".")
            },
        )
        .await
        .unwrap();
        let src = &filtered.root.children.as_ref().unwrap()[0];
        let names: Vec<_> = src
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(names, vec!["lib.rs"]);

        let unfiltered = handle(r, c, input(".")).await.unwrap();
        let src = &unfiltered.root.children.as_ref().unwrap()[0];
        assert!(subtree_contains(src, ".cache"), "default must keep dots");
    }

    /// Explicitly rooting the walk at a hidden folder expresses intent to
    /// see it — the filter must not blank the listing, only its own
    /// hidden CHILDREN drop out.
    #[tokio::test]
    async fn hidden_root_explicitly_requested_still_lists_contents() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join(".claude")).unwrap();
        std::fs::write(tmp.path().join(".claude/settings.json"), "x").unwrap();
        std::fs::write(tmp.path().join(".claude/.credentials"), "x").unwrap();

        let out = handle(
            r,
            c,
            TreeInput {
                include_hidden: false,
                ..input(".claude")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.root.name, ".claude");
        let names: Vec<_> = out
            .root
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(names, vec!["settings.json"]);
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
