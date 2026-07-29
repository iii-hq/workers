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

/// Every file path in the snapshot, root-relative, in traversal order.
///
/// Directories are descended but not emitted: this feeds the file picker, and
/// a folder is not something you open. `limit` bounds the walk so a huge tree
/// cannot turn one call into an unbounded allocation.
pub fn file_paths(tree: &Value, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    // Directories do not grow `out`, so a tree of mostly-empty folders would
    // be walked in full however low the file limit is. Bound the visit count
    // as well, generously enough that it never truncates a real result.
    let mut budget = limit.saturating_mul(8).max(1_000);
    if let Some(root) = tree.get("root") {
        walk(root, "", limit, &mut budget, &mut out);
    }
    out
}

fn walk(node: &Value, prefix: &str, limit: usize, budget: &mut usize, out: &mut Vec<String>) {
    if out.len() >= limit || *budget == 0 {
        return;
    }
    let Some(children) = node.get("children").and_then(Value::as_array) else {
        return;
    };
    for child in children {
        if out.len() >= limit || *budget == 0 {
            return;
        }
        *budget -= 1;
        let Some(name) = child.get("name").and_then(Value::as_str) else {
            continue;
        };
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if is_dir(child) {
            walk(child, &path, limit, budget, out);
        } else {
            // Anything that is not a directory is openable as far as this is
            // concerned; `editor::open` is the one guard that decides whether
            // the bytes are actually text.
            out.push(path);
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

    #[test]
    fn paths_are_joined_from_the_root_down() {
        let paths = file_paths(&sample(), 100);
        assert_eq!(paths, vec!["README.md", "src/main.rs", "src/app/mod.rs"]);
    }

    #[test]
    fn folders_are_descended_but_not_emitted() {
        let paths = file_paths(&sample(), 100);
        assert!(!paths.iter().any(|p| p == "src" || p == "src/app"));
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
        assert_eq!(file_paths(&tree, 10), vec!["src/a.rs"]);
    }

    #[test]
    fn an_empty_dir_contributes_nothing() {
        let tree = json!({
            "root": {
                "name": "r", "kind": "dir",
                "children": [{ "name": "empty", "kind": "dir", "children": [] }]
            }
        });
        assert!(file_paths(&tree, 10).is_empty());
    }

    /// A tree of empty directories must not be traversed without bound just
    /// because it yields no files.
    #[test]
    fn the_visit_budget_stops_a_directory_only_tree() {
        let mut node = json!({ "name": "leaf", "kind": "dir", "children": [] });
        for i in 0..400 {
            node = json!({ "name": format!("d{i}"), "kind": "dir", "children": [node] });
        }
        assert!(file_paths(&json!({ "root": node }), 10).is_empty());
    }

    #[test]
    fn limit_stops_the_walk() {
        assert_eq!(file_paths(&sample(), 2).len(), 2);
    }

    #[test]
    fn an_empty_or_foreign_shape_yields_nothing_rather_than_panicking() {
        assert!(file_paths(&json!({}), 10).is_empty());
        assert!(file_paths(&json!({ "root": { "name": "x", "kind": "dir" } }), 10).is_empty());
        assert!(file_paths(&json!({ "root": 5 }), 10).is_empty());
    }

    #[test]
    fn a_child_without_a_name_is_skipped_not_fatal() {
        let tree = json!({
            "root": {
                "name": "r", "kind": "dir",
                "children": [ { "kind": "file" }, { "name": "ok.rs", "kind": "file" } ]
            }
        });
        assert_eq!(file_paths(&tree, 10), vec!["ok.rs"]);
    }
}
