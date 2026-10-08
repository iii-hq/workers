//! Three-way merge of a kit file the user edited and the kit also changed.
//!
//! `base` is the content the kit installed (its sha is in `kits.lock`,
//! fetched back with `GET /blobs/<sha>`), `ours` the file on disk, `theirs`
//! the new kit version. A clean merge yields the merged text; otherwise the
//! text carries conflict markers and the user decides (kit / mine / merged).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    Clean,
    Conflicts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    pub status: MergeStatus,
    /// The merged text; with conflict markers when `status` is `conflicts`.
    pub content: String,
    /// Number of conflict regions.
    pub conflicts: usize,
}

/// Merge `ours` and `theirs` against `base`.
pub fn three_way(base: &str, ours: &str, theirs: &str) -> MergeResult {
    if ours == theirs {
        return MergeResult {
            status: MergeStatus::Clean,
            content: ours.to_string(),
            conflicts: 0,
        };
    }
    if ours == base {
        return MergeResult {
            status: MergeStatus::Clean,
            content: theirs.to_string(),
            conflicts: 0,
        };
    }
    if theirs == base {
        return MergeResult {
            status: MergeStatus::Clean,
            content: ours.to_string(),
            conflicts: 0,
        };
    }
    match diffy::merge(base, ours, theirs) {
        Ok(content) => MergeResult {
            status: MergeStatus::Clean,
            content,
            conflicts: 0,
        },
        Err(content) => {
            let content = relabel_markers(&content);
            let conflicts = content
                .lines()
                .filter(|l| l.starts_with("<<<<<<<"))
                .count()
                .max(1);
            MergeResult {
                status: MergeStatus::Conflicts,
                content,
                conflicts,
            }
        }
    }
}

/// Name the sides in the reader's terms: `yours` (the file on disk),
/// `installed` (the base) and `kit` (the new version).
fn relabel_markers(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let relabeled = match body {
            "<<<<<<< ours" => "<<<<<<< yours",
            "||||||| original" => "||||||| installed",
            ">>>>>>> theirs" => ">>>>>>> kit",
            other => other,
        };
        out.push_str(relabeled);
        out.push_str(eol);
    }
    out
}

/// Does `content` still carry conflict markers?
pub fn has_conflict_markers(content: &str) -> bool {
    let mut open = false;
    for line in content.lines() {
        if line.starts_with("<<<<<<<") {
            open = true;
        } else if open && line.starts_with(">>>>>>>") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "# Flow\n\nStep one.\nStep two.\nStep three.\n";

    #[test]
    fn disjoint_edits_merge_cleanly() {
        let ours = "# Flow\n\nStep one, carefully.\nStep two.\nStep three.\n";
        let theirs = "# Flow\n\nStep one.\nStep two.\nStep three.\nStep four.\n";
        let merged = three_way(BASE, ours, theirs);
        assert_eq!(merged.status, MergeStatus::Clean);
        assert_eq!(
            merged.content,
            "# Flow\n\nStep one, carefully.\nStep two.\nStep three.\nStep four.\n"
        );
        assert_eq!(merged.conflicts, 0);
    }

    #[test]
    fn overlapping_edits_conflict_with_markers() {
        let ours = "# Flow\n\nStep one.\nStep 2 (mine).\nStep three.\n";
        let theirs = "# Flow\n\nStep one.\nStep 2 (kit).\nStep three.\n";
        let merged = three_way(BASE, ours, theirs);
        assert_eq!(merged.status, MergeStatus::Conflicts);
        assert_eq!(merged.conflicts, 1);
        assert!(merged.content.contains("<<<<<<< yours"));
        assert!(merged.content.contains(">>>>>>> kit"));
        assert!(merged.content.contains("Step 2 (mine)."));
        assert!(merged.content.contains("Step 2 (kit)."));
        assert!(has_conflict_markers(&merged.content));
    }

    #[test]
    fn trivial_sides_short_circuit() {
        assert_eq!(three_way(BASE, BASE, "x\n").content, "x\n");
        assert_eq!(three_way(BASE, "y\n", BASE).content, "y\n");
        assert_eq!(three_way(BASE, "z\n", "z\n").status, MergeStatus::Clean);
        assert!(!has_conflict_markers(BASE));
    }
}
