//! Flattening the shell worker's tree snapshot into paths.
//!
//! `coder::tree` answers with nested nodes carrying only a `name`; a path is
//! built by joining from the root down. Two callers need that walk — the file
//! picker when there is no git listing to rank, and any surface that wants a
//! flat view — so it lives here once, over `serde_json::Value` rather than a
//! mirrored struct, because the only fields that matter are `name`, `kind` and
//! `children`, and mirroring the rest would couple this worker to shell's
//! response shape for no gain.

use serde_json::Value;

/// Smallest visit budget any walk gets, however small the file limit is. A
/// folder costs a visit but yields no path, so a low limit must still be able
/// to reach past the directories in front of the files.
const MIN_VISIT_BUDGET: usize = 1_000;

/// A flattened snapshot, and whether the walk saw all of it.
#[derive(Debug, Default)]
pub struct Walk {
    /// Every file path found, root-relative, in traversal order.
    pub paths: Vec<String>,
    /// True when the walk stopped early — at `limit` files, or at the visit
    /// budget — so a caller can report a partial listing as partial instead of
    /// presenting it as the whole tree.
    ///
    /// A tree holding exactly `limit` files reports `true` as well: from in
    /// here a listing cut at the boundary is indistinguishable from a complete
    /// one, and claiming completeness wrongly is the worse error.
    pub truncated: bool,
}

/// Flatten a `coder::tree` snapshot to its file paths.
///
/// Directories are descended but not emitted: this feeds the file picker, and a
/// folder is not something you open. `limit` bounds the files collected, and a
/// visit budget bounds the nodes walked to get them — without the second bound
/// a tree of empty folders is traversed in full however low `limit` is, since
/// directories never grow the result.
///
/// The budget is deliberately generous (eight visits per file wanted, and never
/// under [`MIN_VISIT_BUDGET`]) because the cost of hitting it is a short
/// listing. `editor::find` calls this with `max_find_candidates`, so a real
/// checkout is nowhere near it; when it does bite, `truncated` says so.
pub fn walk(tree: &Value, limit: usize) -> Walk {
    let mut state = Visit {
        limit,
        budget: limit.saturating_mul(8).max(MIN_VISIT_BUDGET),
        walk: Walk::default(),
    };
    if let Some(root) = tree.get("root") {
        descend(root, "", &mut state);
    }
    state.walk
}

struct Visit {
    limit: usize,
    budget: usize,
    walk: Walk,
}

impl Visit {
    /// True when the walk must stop, recording that the result is partial.
    fn spent(&mut self) -> bool {
        if self.walk.paths.len() >= self.limit || self.budget == 0 {
            self.walk.truncated = true;
            return true;
        }
        false
    }
}

fn descend(node: &Value, prefix: &str, state: &mut Visit) {
    if state.spent() {
        return;
    }
    let Some(children) = node.get("children").and_then(Value::as_array) else {
        return;
    };
    for child in children {
        if state.spent() {
            return;
        }
        state.budget -= 1;
        let Some(name) = child.get("name").and_then(Value::as_str) else {
            continue;
        };
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if is_dir(child) {
            descend(child, &path, state);
        } else {
            // Anything that is not a directory is openable as far as this is
            // concerned; `editor::open` is the one guard that decides whether
            // the bytes are actually text.
            state.walk.paths.push(path);
        }
    }
}

/// shell spells a directory `dir` (its `NodeKind` is lowercase-renamed
/// `File | Dir | Symlink | Other`). `folder` is accepted too so a future
/// rename on that side degrades to a wrong-looking tree rather than to
/// directories being opened as files.
fn is_dir(node: &Value) -> bool {
    matches!(
        node.get("kind").and_then(Value::as_str),
        Some("dir") | Some("folder")
    ) || node.get("children").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "path": "/repo",
            "root": {
                "name": "repo",
                "kind": "dir",
                "children": [
                    { "name": "README.md", "kind": "file" },
                    {
                        "name": "src",
                        "kind": "dir",
                        "children": [
                            { "name": "main.rs", "kind": "file" },
                            {
                                "name": "app",
                                "kind": "dir",
                                "children": [{ "name": "mod.rs", "kind": "file" }]
                            }
                        ]
                    }
                ]
            }
        })
    }

    /// A directory tree shaped like a real checkout: nested a few levels deep,
    /// dozens of folders, hundreds of files. `editor::find` walks with
    /// `max_find_candidates`, so this is the case that must never come back
    /// short — a bound that truncated an ordinary repository would be a worse
    /// bug than the unbounded walk it replaced.
    fn repository_shaped(depth: usize, dirs_per_level: usize, files_per_dir: usize) -> Value {
        fn build(prefix: &str, level: usize, depth: usize, dirs: usize, files: usize) -> Value {
            let mut children: Vec<Value> = (0..files)
                .map(|f| json!({ "name": format!("{prefix}f{f}.rs"), "kind": "file" }))
                .collect();
            if level < depth {
                for d in 0..dirs {
                    let name = format!("{prefix}d{level}_{d}");
                    let mut child = build(&name, level + 1, depth, dirs, files);
                    child["name"] = json!(name);
                    child["kind"] = json!("dir");
                    children.push(child);
                }
            }
            json!({ "children": children })
        }
        let mut root = build("", 0, depth, dirs_per_level, files_per_dir);
        root["name"] = json!("repo");
        root["kind"] = json!("dir");
        json!({ "root": root })
    }

    fn count_files(node: &Value) -> usize {
        match node.get("children").and_then(Value::as_array) {
            Some(children) => children.iter().map(count_files).sum(),
            None => 1,
        }
    }

    #[test]
    fn paths_are_joined_from_the_root_down() {
        let walked = walk(&sample(), 100);
        assert_eq!(
            walked.paths,
            vec!["README.md", "src/main.rs", "src/app/mod.rs"]
        );
        assert!(!walked.truncated, "three files is not a truncated walk");
    }

    #[test]
    fn folders_are_descended_but_not_emitted() {
        let paths = walk(&sample(), 100).paths;
        assert!(!paths.iter().any(|p| p == "src" || p == "src/app"));
    }

    /// The visit budget must not cost a real tree any of its files.
    #[test]
    fn a_repository_shaped_tree_comes_back_whole() {
        // 3 levels × 4 folders each = 84 directories, 5 files in every one:
        // 425 files, ~509 visits. Well past MIN_VISIT_BUDGET's reach at a small
        // limit, and nowhere near the budget a real `editor::find` call gets.
        let tree = repository_shaped(3, 4, 5);
        let total = count_files(tree.get("root").expect("a root"));
        assert!(total > 400, "fixture is meant to be repository-sized");

        // The shipped `max_find_candidates`, i.e. what `editor::find` passes.
        let walked = walk(&tree, 50_000);
        assert_eq!(
            walked.paths.len(),
            total,
            "every file in an ordinary tree must be returned"
        );
        assert!(
            !walked.truncated,
            "a complete walk must not claim to be partial"
        );
    }

    /// Pins shell's real vocabulary. Reading `dir` as a file is what made
    /// the tree open directories instead of expanding them.
    #[test]
    fn a_dir_node_is_descended_not_emitted() {
        let tree = json!({
            "root": {
                "name": "r", "kind": "dir",
                "children": [{
                    "name": "src", "kind": "dir",
                    "children": [{ "name": "a.rs", "kind": "file" }]
                }]
            }
        });
        assert_eq!(walk(&tree, 10).paths, vec!["src/a.rs"]);
    }

    #[test]
    fn an_empty_dir_contributes_nothing() {
        let tree = json!({
            "root": {
                "name": "r", "kind": "dir",
                "children": [{ "name": "empty", "kind": "dir", "children": [] }]
            }
        });
        let walked = walk(&tree, 10);
        assert!(walked.paths.is_empty());
        assert!(!walked.truncated, "an empty tree was walked in full");
    }

    /// A tree of empty directories must not be traversed without bound just
    /// because it yields no files — and when the budget does stop it, the
    /// result has to say so rather than pass a partial listing off as whole.
    #[test]
    fn the_visit_budget_stops_a_directory_only_tree_and_reports_it() {
        // More empty folders than MIN_VISIT_BUDGET, with a file behind them
        // that only an unbounded walk would ever reach.
        let mut children: Vec<Value> = (0..MIN_VISIT_BUDGET + 200)
            .map(|i| json!({ "name": format!("d{i}"), "kind": "dir", "children": [] }))
            .collect();
        children.push(json!({ "name": "behind-the-budget.rs", "kind": "file" }));
        let tree = json!({ "root": { "name": "r", "kind": "dir", "children": children } });

        let walked = walk(&tree, 10);
        assert!(
            walked.paths.is_empty(),
            "the walk must stop before the file the budget cannot reach"
        );
        assert!(
            walked.truncated,
            "a walk cut short by the budget must not look complete"
        );
    }

    /// The same shape inside the budget: the file is found, and nothing claims
    /// to be truncated. This is the half that keeps the bound from quietly
    /// shortening an ordinary listing.
    #[test]
    fn folders_within_the_budget_do_not_hide_the_files_behind_them() {
        let mut children: Vec<Value> = (0..100)
            .map(|i| json!({ "name": format!("d{i}"), "kind": "dir", "children": [] }))
            .collect();
        children.push(json!({ "name": "after-the-folders.rs", "kind": "file" }));
        let tree = json!({ "root": { "name": "r", "kind": "dir", "children": children } });

        let walked = walk(&tree, 10);
        assert_eq!(walked.paths, vec!["after-the-folders.rs"]);
        assert!(!walked.truncated);
    }

    #[test]
    fn limit_stops_the_walk_and_says_so() {
        let walked = walk(&sample(), 2);
        assert_eq!(walked.paths.len(), 2);
        assert!(walked.truncated, "two of three files is a partial listing");
    }

    #[test]
    fn an_empty_or_foreign_shape_yields_nothing_rather_than_panicking() {
        assert!(walk(&json!({}), 10).paths.is_empty());
        assert!(walk(&json!({ "root": { "name": "x", "kind": "dir" } }), 10)
            .paths
            .is_empty());
        assert!(walk(&json!({ "root": 5 }), 10).paths.is_empty());
    }

    #[test]
    fn a_child_without_a_name_is_skipped_not_fatal() {
        let tree = json!({
            "root": {
                "name": "r", "kind": "dir",
                "children": [ { "kind": "file" }, { "name": "ok.rs", "kind": "file" } ]
            }
        });
        assert_eq!(walk(&tree, 10).paths, vec!["ok.rs"]);
    }
}
