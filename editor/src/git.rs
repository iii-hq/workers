//! Parsers for the git output `editor` asks `shell::exec` to produce.
//!
//! The worker runs no git itself: `shell::exec` owns the process, the jail,
//! the denylist and the timeout. What lives here is the half that has no
//! business being in a shell worker — turning porcelain text into typed rows a
//! UI or an agent can use without re-implementing the format.
//!
//! Porcelain v2 is parsed rather than v1 because v1 cannot express rename
//! scores or the ahead/behind pair, and its path field is ambiguous once a
//! filename contains a space.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diff::Hunk;

/// A single changed path as git sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusEntry {
    /// Path relative to the repository root.
    pub path: String,
    /// Status of the staged copy: `modified`, `added`, `deleted`, `renamed`,
    /// `copied`, `untracked`, `ignored`, `conflicted`, or `unchanged`.
    pub index: String,
    /// Status of the working-tree copy, same vocabulary as `index`.
    pub worktree: String,
    /// True when the staged copy differs from HEAD.
    pub staged: bool,
    /// Original path for a rename or copy.
    pub renamed_from: Option<String>,
}

/// Branch header plus every changed path.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct StatusReport {
    /// Current branch, or `None` on a detached HEAD.
    pub branch: Option<String>,
    /// Configured upstream, e.g. `origin/main`.
    pub upstream: Option<String>,
    /// Commits on this branch the upstream does not have.
    pub ahead: u32,
    /// Commits on the upstream this branch does not have.
    pub behind: u32,
    /// One row per changed path, in git's order.
    pub entries: Vec<StatusEntry>,
    /// True when nothing is modified, staged, or untracked.
    pub clean: bool,
}

fn code_to_word(c: char) -> &'static str {
    match c {
        'M' => "modified",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "typechange",
        'U' => "conflicted",
        '.' => "unchanged",
        _ => "unknown",
    }
}

/// Parse `git status --porcelain=v2 --branch --untracked-files=all`.
///
/// Unknown or malformed lines are skipped rather than failing the call: a
/// status view that renders nothing because one exotic line did not parse is
/// worse than one that renders every line it understood.
pub fn parse_status(stdout: &str) -> StatusReport {
    let mut report = StatusReport::default();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            if rest != "(detached)" {
                report.branch = Some(rest.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            report.upstream = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // "+2 -3"
            for token in rest.split_whitespace() {
                let (sign, num) = token.split_at(1);
                let n: u32 = num.parse().unwrap_or(0);
                match sign {
                    "+" => report.ahead = n,
                    "-" => report.behind = n,
                    _ => {}
                }
            }
        } else if let Some(rest) = line.strip_prefix("1 ") {
            // <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
            if let Some(entry) = parse_ordinary(rest) {
                report.entries.push(entry);
            }
        } else if let Some(rest) = line.strip_prefix("2 ") {
            // <XY> <sub> <mH> <mI> <mW> <hH> <hI> <score> <path>\t<orig>
            if let Some(entry) = parse_rename(rest) {
                report.entries.push(entry);
            }
        } else if let Some(path) = line.strip_prefix("? ") {
            report.entries.push(StatusEntry {
                path: path.to_string(),
                index: "untracked".to_string(),
                worktree: "untracked".to_string(),
                staged: false,
                renamed_from: None,
            });
        } else if let Some(path) = line.strip_prefix("! ") {
            report.entries.push(StatusEntry {
                path: path.to_string(),
                index: "ignored".to_string(),
                worktree: "ignored".to_string(),
                staged: false,
                renamed_from: None,
            });
        } else if let Some(rest) = line.strip_prefix("u ") {
            // Unmerged: `<XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
            // — nine columns before the path, not ten. Reading one field too far
            // meant conflicted entries were silently dropped from the status.
            let fields: Vec<&str> = rest.splitn(10, ' ').collect();
            if let Some(path) = fields.get(9) {
                report.entries.push(StatusEntry {
                    path: (*path).to_string(),
                    index: "conflicted".to_string(),
                    worktree: "conflicted".to_string(),
                    staged: false,
                    renamed_from: None,
                });
            }
        }
    }

    report.clean = report
        .entries
        .iter()
        .all(|e| e.index == "ignored" && e.worktree == "ignored");
    report
}

/// `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>` — path is the remainder, so a
/// filename containing spaces survives.
fn parse_ordinary(rest: &str) -> Option<StatusEntry> {
    let mut fields = rest.splitn(8, ' ');
    let xy = fields.next()?;
    for _ in 0..6 {
        fields.next()?;
    }
    let path = fields.next()?;
    let mut chars = xy.chars();
    let index = code_to_word(chars.next()?);
    let worktree = code_to_word(chars.next()?);
    Some(StatusEntry {
        path: path.to_string(),
        staged: index != "unchanged",
        index: index.to_string(),
        worktree: worktree.to_string(),
        renamed_from: None,
    })
}

/// Rename/copy line: one extra score field, and the path pair is tab-separated.
fn parse_rename(rest: &str) -> Option<StatusEntry> {
    let mut fields = rest.splitn(9, ' ');
    let xy = fields.next()?;
    for _ in 0..7 {
        fields.next()?;
    }
    let paths = fields.next()?;
    let (path, orig) = paths.split_once('\t')?;
    let mut chars = xy.chars();
    let index = code_to_word(chars.next()?);
    let worktree = code_to_word(chars.next()?);
    Some(StatusEntry {
        path: path.to_string(),
        staged: index != "unchanged",
        index: index.to_string(),
        worktree: worktree.to_string(),
        renamed_from: Some(orig.to_string()),
    })
}

/// Pull the hunk ranges out of a unified diff.
///
/// Only `@@` headers are read, so this is cheap on a large diff and works
/// whatever `-U` the caller passed. `@@ -a,b +c,d @@` omits `,b` when the
/// count is 1, which is the case that silently breaks naive parsers.
pub fn parse_hunk_headers(diff_text: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;

    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            let Some(header) = rest.split(" @@").next() else {
                continue;
            };
            let mut parts = header.split_whitespace();
            let (Some(old), Some(new)) = (parts.next(), parts.next()) else {
                continue;
            };
            let Some((old_start, old_lines)) = parse_range(old.trim_start_matches('-')) else {
                continue;
            };
            let Some((new_start, new_lines)) = parse_range(new.trim_start_matches('+')) else {
                continue;
            };
            current = Some(Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                added: 0,
                removed: 0,
            });
        } else if let Some(h) = current.as_mut() {
            if line.starts_with('+') && !line.starts_with("+++") {
                h.added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                h.removed += 1;
            }
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    hunks
}

/// `12,3` or the count-elided `12`.
fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_header_and_ahead_behind() {
        let out = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -3\n";
        let r = parse_status(out);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(r.upstream.as_deref(), Some("origin/main"));
        assert_eq!((r.ahead, r.behind), (2, 3));
        assert!(r.clean);
    }

    #[test]
    fn detached_head_has_no_branch() {
        let r = parse_status("# branch.head (detached)\n");
        assert!(r.branch.is_none());
    }

    #[test]
    fn ordinary_entry_splits_index_and_worktree() {
        let out = "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\n";
        let r = parse_status(out);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "src/main.rs");
        assert_eq!(r.entries[0].index, "unchanged");
        assert_eq!(r.entries[0].worktree, "modified");
        assert!(!r.entries[0].staged);
        assert!(!r.clean);
    }

    #[test]
    fn staged_entry_is_flagged() {
        let out = "1 M. N... 100644 100644 100644 aaa bbb src/lib.rs\n";
        let r = parse_status(out);
        assert!(r.entries[0].staged);
        assert_eq!(r.entries[0].index, "modified");
    }

    #[test]
    fn path_with_spaces_survives() {
        let out = "1 .M N... 100644 100644 100644 aaa bbb my docs/notes v2.md\n";
        let r = parse_status(out);
        assert_eq!(r.entries[0].path, "my docs/notes v2.md");
    }

    #[test]
    fn rename_carries_the_original_path() {
        let out = "2 R. N... 100644 100644 100644 aaa bbb R100 new/name.rs\told/name.rs\n";
        let r = parse_status(out);
        assert_eq!(r.entries[0].path, "new/name.rs");
        assert_eq!(r.entries[0].renamed_from.as_deref(), Some("old/name.rs"));
        assert_eq!(r.entries[0].index, "renamed");
    }

    #[test]
    fn untracked_and_ignored_are_distinguished() {
        let r = parse_status("? new.rs\n! target/\n");
        assert_eq!(r.entries[0].index, "untracked");
        assert_eq!(r.entries[1].index, "ignored");
        assert!(!r.clean, "an untracked file is not a clean tree");
    }

    #[test]
    fn only_ignored_entries_still_count_as_clean() {
        let r = parse_status("! target/\n");
        assert!(r.clean);
    }

    #[test]
    fn unmerged_entries_are_reported_as_conflicted() {
        // Nine columns then the path, per porcelain v2.
        let out = "u UU N... 100644 100644 100644 100644 aaa bbb ccc src/conflict.rs\n";
        let r = parse_status(out);
        assert_eq!(r.entries.len(), 1, "an unmerged entry must not be dropped");
        assert_eq!(r.entries[0].path, "src/conflict.rs");
        assert_eq!(r.entries[0].worktree, "conflicted");
        assert!(!r.clean);
    }

    #[test]
    fn malformed_line_is_skipped_not_fatal() {
        let r = parse_status("1 broken\n? real.rs\n");
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "real.rs");
    }

    #[test]
    fn hunk_headers_parse_with_and_without_counts() {
        let d = "@@ -1,3 +1,4 @@\n context\n+added\n@@ -20 +21,2 @@\n-gone\n+new\n+more\n";
        let hunks = parse_hunk_headers(d);
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].old_start, hunks[0].old_lines), (1, 3));
        assert_eq!(hunks[0].added, 1);
        assert_eq!(
            (hunks[1].old_start, hunks[1].old_lines),
            (20, 1),
            "an elided count means exactly one line"
        );
        assert_eq!((hunks[1].added, hunks[1].removed), (2, 1));
    }

    #[test]
    fn file_header_lines_are_not_counted_as_edits() {
        let d = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let hunks = parse_hunk_headers(d);
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].added, hunks[0].removed), (1, 1));
    }
}
