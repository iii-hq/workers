//! Unified line diff between two in-memory texts.
//!
//! This is the one piece of the worker that touches nothing else: strings in,
//! patch out. Callers already hold both sides — an agent has the text it is
//! about to write, the console page has the buffer and what `shell::fs::read`
//! returned — so `editor::diff` never needs a path, a jail, or a filesystem.
//!
//! Hunks are returned as structured ranges *alongside* the rendered patch. The
//! patch is for humans and for `git apply`; the ranges are what a gutter or a
//! review UI paints, and computing them here means no consumer has to re-parse
//! `@@` headers to find out which lines moved.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

/// One `@@` block: the line ranges it covers on each side, plus the counts a
/// gutter needs without re-reading the patch body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hunk {
    /// First line of the hunk on the "before" side, 1-based. `0` when the
    /// before side is empty (pure addition), matching unified-diff convention.
    pub old_start: u32,
    /// Number of "before" lines the hunk spans.
    pub old_lines: u32,
    /// First line of the hunk on the "after" side, 1-based.
    pub new_start: u32,
    /// Number of "after" lines the hunk spans.
    pub new_lines: u32,
    /// Lines added within this hunk.
    pub added: u32,
    /// Lines removed within this hunk.
    pub removed: u32,
}

/// A rendered patch plus the structured view of the same edits.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DiffResult {
    /// Unified diff. Empty when the two sides are identical.
    pub patch: String,
    /// One entry per `@@` block, in file order.
    pub hunks: Vec<Hunk>,
    /// Total lines added across every hunk.
    pub added: u32,
    /// Total lines removed across every hunk.
    pub removed: u32,
    /// True when `before` and `after` are byte-identical.
    pub identical: bool,
    /// True when either side exceeded `max_bytes` and the diff was skipped.
    /// `patch` and `hunks` are empty in that case — a caller that ignores this
    /// flag would read "no changes" from a file that was simply too big.
    pub truncated: bool,
}

/// Diff `before` against `after`.
///
/// `context` is the number of unchanged lines kept around each hunk (the `-U`
/// of `git diff`). `path` labels the `---`/`+++` header; `None` renders the
/// git-style `a/`-less placeholder so the patch is still valid to read.
///
/// Refuses inputs over `max_bytes` rather than spending unbounded time and
/// memory on a minified bundle or a checked-in binary blob.
pub fn diff(
    before: &str,
    after: &str,
    path: Option<&str>,
    context: usize,
    max_bytes: usize,
) -> DiffResult {
    if before == after {
        return DiffResult {
            patch: String::new(),
            hunks: Vec::new(),
            added: 0,
            removed: 0,
            identical: true,
            truncated: false,
        };
    }

    if before.len() > max_bytes || after.len() > max_bytes {
        return DiffResult {
            patch: String::new(),
            hunks: Vec::new(),
            added: 0,
            removed: 0,
            identical: false,
            truncated: true,
        };
    }

    let text_diff = TextDiff::from_lines(before, after);
    let label = path.unwrap_or("file");

    let mut patch = format!("--- a/{label}\n+++ b/{label}\n");
    let mut hunks = Vec::new();
    let mut added = 0u32;
    let mut removed = 0u32;

    for group in text_diff.grouped_ops(context).iter() {
        // `grouped_ops` yields the ops of one hunk; the first and last carry
        // its bounds on both sides.
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;

        let mut hunk_added = 0u32;
        let mut hunk_removed = 0u32;
        let mut body = String::new();

        for op in group {
            for change in text_diff.iter_changes(op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => {
                        hunk_removed += 1;
                        '-'
                    }
                    ChangeTag::Insert => {
                        hunk_added += 1;
                        '+'
                    }
                    ChangeTag::Equal => ' ',
                };
                body.push(sign);
                body.push_str(change.value());
                // A final line with no trailing newline must not run into the
                // next patch line, or the patch stops being applyable.
                if !change.value().ends_with('\n') {
                    body.push('\n');
                    body.push_str("\\ No newline at end of file\n");
                }
            }
        }

        let old_len = (old_range.end - old_range.start) as u32;
        let new_len = (new_range.end - new_range.start) as u32;
        // Unified diff numbers lines from 1, except an empty range on one
        // side, which is written as start 0.
        let old_start = if old_len == 0 {
            old_range.start as u32
        } else {
            old_range.start as u32 + 1
        };
        let new_start = if new_len == 0 {
            new_range.start as u32
        } else {
            new_range.start as u32 + 1
        };

        patch.push_str(&format!(
            "@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"
        ));
        patch.push_str(&body);

        added += hunk_added;
        removed += hunk_removed;
        hunks.push(Hunk {
            old_start,
            old_lines: old_len,
            new_start,
            new_lines: new_len,
            added: hunk_added,
            removed: hunk_removed,
        });
    }

    DiffResult {
        patch,
        hunks,
        added,
        removed,
        identical: false,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 1 << 20;

    #[test]
    fn identical_inputs_report_no_change() {
        let r = diff("a\nb\n", "a\nb\n", None, 3, MAX);
        assert!(r.identical);
        assert!(r.patch.is_empty());
        assert!(r.hunks.is_empty());
        assert_eq!((r.added, r.removed), (0, 0));
    }

    #[test]
    fn single_line_change_counts_both_sides() {
        let r = diff("a\nb\nc\n", "a\nB\nc\n", Some("x.txt"), 3, MAX);
        assert!(!r.identical);
        assert_eq!((r.added, r.removed), (1, 1));
        assert_eq!(r.hunks.len(), 1);
        assert!(r.patch.contains("--- a/x.txt"));
        assert!(r.patch.contains("+++ b/x.txt"));
        assert!(r.patch.contains("-b\n"));
        assert!(r.patch.contains("+B\n"));
    }

    #[test]
    fn pure_insertion_into_empty_file_starts_at_zero() {
        let r = diff("", "hello\n", None, 3, MAX);
        assert_eq!(r.hunks.len(), 1);
        assert_eq!(r.hunks[0].old_start, 0);
        assert_eq!(r.hunks[0].old_lines, 0);
        assert_eq!(r.hunks[0].new_start, 1);
        assert_eq!((r.added, r.removed), (1, 0));
    }

    #[test]
    fn distant_edits_produce_separate_hunks() {
        let before: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let mut after: Vec<String> = (0..40).map(|i| format!("line {i}\n")).collect();
        after[1] = "CHANGED near top\n".to_string();
        after[38] = "CHANGED near bottom\n".to_string();
        let r = diff(&before, &after.concat(), None, 3, MAX);
        assert_eq!(r.hunks.len(), 2, "context 3 must not merge distant edits");
        assert_eq!((r.added, r.removed), (2, 2));
    }

    #[test]
    fn missing_trailing_newline_is_marked() {
        let r = diff("a\nb", "a\nc", None, 3, MAX);
        assert!(r.patch.contains("\\ No newline at end of file"));
    }

    #[test]
    fn oversized_input_is_refused_not_silently_empty() {
        let big = "x\n".repeat(100);
        let r = diff(&big, "y\n", None, 3, 8);
        assert!(r.truncated);
        assert!(!r.identical, "truncated must never look like 'no changes'");
        assert!(r.patch.is_empty());
    }

    #[test]
    fn context_zero_yields_tight_hunks() {
        let before = "a\nb\nc\nd\ne\n";
        let after = "a\nb\nX\nd\ne\n";
        let r = diff(before, after, None, 0, MAX);
        assert_eq!(r.hunks.len(), 1);
        assert_eq!(r.hunks[0].old_lines, 1);
        assert_eq!(r.hunks[0].new_lines, 1);
    }
}
