//! `coder::update-file` — batched line-oriented and regex edits across one
//! or more files. Line ops are applied **bottom-up** so earlier ops see the
//! original line numbers; regex replaces run afterward on the serialized
//! body. Each file commits atomically via sibling temp + `rename`.
//!
//! Line ops (1-based, inclusive):
//!
//! - `{ op: "insert", at_line: N, content: "..." }` — insert before line N.
//!   `N = lines + 1` appends at the end.
//! - `{ op: "remove", from_line: A, to_line: B }` — delete lines A..=B.
//! - `{ op: "update_lines", from_line: A, to_line: B, content: "..." }` —
//!   overwrite lines A..=B with `content` (split by `\n`).
//!
//! Content op:
//!
//! - `{ op: "replace", pattern: "...", replacement: "...", ignore_case?: bool }`
//!   — regex substitution on the full file body after line ops.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::CoderConfig;
use crate::error::{err_to_string, CoderError};
use crate::path::PathResolver;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateFileInput {
    pub files: Vec<UpdateFileSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateFileSpec {
    pub path: String,
    pub ops: Vec<UpdateOp>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum UpdateOp {
    /// Insert `content` before line `at_line` (1-based). `at_line = lines+1`
    /// appends to end of file.
    Insert { at_line: u32, content: String },
    /// Delete lines `from_line..=to_line` (1-based, inclusive).
    Remove { from_line: u32, to_line: u32 },
    /// Overwrite lines `from_line..=to_line` with `content`.
    #[serde(rename = "update_lines")]
    UpdateLines {
        from_line: u32,
        to_line: u32,
        content: String,
    },
    /// Replace all regex matches in the file body (after line ops).
    Replace {
        pattern: String,
        replacement: String,
        #[serde(default)]
        ignore_case: bool,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateFileOutput {
    pub results: Vec<UpdateFileResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateFileResult {
    pub path: String,
    pub success: bool,
    /// Number of operations applied (only meaningful when `success`).
    pub applied: u32,
    /// Final line count after applying (only meaningful when `success`).
    pub new_line_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: UpdateFileInput,
) -> Result<UpdateFileOutput, String> {
    if req.files.is_empty() {
        return Err(err_to_string(CoderError::BadInput(
            "`files` must not be empty".into(),
        )));
    }
    let mut results = Vec::with_capacity(req.files.len());
    for spec in req.files {
        results.push(update_one(&resolver, &cfg, spec));
    }
    Ok(UpdateFileOutput { results })
}

fn update_one(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    spec: UpdateFileSpec,
) -> UpdateFileResult {
    let path = spec.path.clone();
    match try_update_one(resolver, cfg, spec) {
        Ok((applied, new_line_count)) => UpdateFileResult {
            path,
            success: true,
            applied,
            new_line_count,
            error: None,
        },
        Err(e) => UpdateFileResult {
            path,
            success: false,
            applied: 0,
            new_line_count: 0,
            error: Some(e.to_wire_string()),
        },
    }
}

fn try_update_one(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    spec: UpdateFileSpec,
) -> Result<(u32, u64), CoderError> {
    let abs = resolver.require_writable(&spec.path)?;
    let md = std::fs::metadata(&abs)?;
    if !md.is_file() {
        return Err(CoderError::BadInput(format!(
            "not a regular file: {}",
            spec.path
        )));
    }
    if md.len() > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "current file size {} exceeds max_write_bytes {}",
            md.len(),
            cfg.max_write_bytes
        )));
    }
    if spec.ops.is_empty() {
        return Err(CoderError::BadInput("ops must not be empty".into()));
    }

    let bytes = std::fs::read(&abs)?;
    let (mut lines, ending, has_trailing) = split_file(&bytes);
    let original_len = lines.len();

    let (line_ops, replace_ops): (Vec<&UpdateOp>, Vec<&UpdateOp>) =
        spec.ops.iter().partition(|op| is_line_op(op));

    validate_line_ops(&line_ops, original_len)?;
    apply_line_ops(&mut lines, &line_ops)?;

    let mut new_bytes = join_lines(&lines, ending, has_trailing);
    new_bytes = apply_regex_replaces(new_bytes, &replace_ops)?;

    let (final_lines, _, _) = split_file(&new_bytes);
    if (new_bytes.len() as u64) > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "new file size {} exceeds max_write_bytes {}",
            new_bytes.len(),
            cfg.max_write_bytes
        )));
    }
    atomic_write(&abs, &new_bytes)?;
    Ok((spec.ops.len() as u32, final_lines.len() as u64))
}

fn is_line_op(op: &UpdateOp) -> bool {
    matches!(
        op,
        UpdateOp::Insert { .. } | UpdateOp::Remove { .. } | UpdateOp::UpdateLines { .. }
    )
}

/// Apply line ops to `lines` in place. Ops are sorted by their first-affected
/// original line, descending, then applied so each op sees the original
/// line numbers from the caller's perspective.
fn apply_line_ops(lines: &mut Vec<String>, ops: &[&UpdateOp]) -> Result<(), CoderError> {
    if ops.is_empty() {
        return Ok(());
    }

    let mut order: Vec<usize> = (0..ops.len()).collect();
    order.sort_by(|&i, &j| anchor(ops[j]).cmp(&anchor(ops[i])));

    for &i in &order {
        match ops[i] {
            UpdateOp::Insert { at_line, content } => {
                let pos = (*at_line as usize) - 1;
                let new_lines = split_content(content);
                lines.splice(pos..pos, new_lines);
            }
            UpdateOp::Remove { from_line, to_line } => {
                let a = (*from_line as usize) - 1;
                let b = *to_line as usize;
                lines.splice(a..b, std::iter::empty::<String>());
            }
            UpdateOp::UpdateLines {
                from_line,
                to_line,
                content,
            } => {
                let a = (*from_line as usize) - 1;
                let b = *to_line as usize;
                let new_lines = split_content(content);
                lines.splice(a..b, new_lines);
            }
            UpdateOp::Replace { .. } => unreachable!("line ops only"),
        }
    }
    Ok(())
}

fn apply_regex_replaces(mut bytes: Vec<u8>, ops: &[&UpdateOp]) -> Result<Vec<u8>, CoderError> {
    for op in ops {
        let UpdateOp::Replace {
            pattern,
            replacement,
            ignore_case,
        } = op
        else {
            continue;
        };
        if pattern.is_empty() {
            return Err(CoderError::BadInput(
                "replace.pattern must not be empty".into(),
            ));
        }
        let mut builder = regex::RegexBuilder::new(pattern);
        builder.case_insensitive(*ignore_case);
        let re = builder
            .build()
            .map_err(|e| CoderError::BadInput(format!("bad regex {pattern:?}: {e}")))?;
        let s = String::from_utf8_lossy(&bytes);
        let out = re.replace_all(&s, replacement.as_str());
        bytes = out.into_owned().into_bytes();
    }
    Ok(bytes)
}

/// Apply line ops only — used by unit tests that exercise line semantics
/// without the full file pipeline.
pub fn apply_ops(
    lines: &mut Vec<String>,
    ops: &[UpdateOp],
    original_len: usize,
) -> Result<u32, CoderError> {
    if ops.is_empty() {
        return Err(CoderError::BadInput("ops must not be empty".into()));
    }
    if ops.iter().any(|op| matches!(op, UpdateOp::Replace { .. })) {
        return Err(CoderError::BadInput(
            "apply_ops does not support regex replace ops".into(),
        ));
    }
    let refs: Vec<&UpdateOp> = ops.iter().collect();
    validate_line_ops(&refs, original_len)?;
    apply_line_ops(lines, &refs)?;
    Ok(ops.len() as u32)
}

fn anchor(op: &UpdateOp) -> u32 {
    match op {
        UpdateOp::Insert { at_line, .. } => *at_line,
        UpdateOp::Remove { from_line, .. } => *from_line,
        UpdateOp::UpdateLines { from_line, .. } => *from_line,
        UpdateOp::Replace { .. } => 0,
    }
}

fn validate_line_ops(ops: &[&UpdateOp], original_len: usize) -> Result<(), CoderError> {
    let len = original_len as u32;
    for op in ops {
        match op {
            UpdateOp::Insert { at_line, .. } => {
                if *at_line == 0 || *at_line > len + 1 {
                    return Err(CoderError::BadInput(format!(
                        "insert.at_line {at_line} out of range (1..={})",
                        len + 1
                    )));
                }
            }
            UpdateOp::Remove { from_line, to_line }
            | UpdateOp::UpdateLines {
                from_line, to_line, ..
            } => {
                if *from_line == 0 || *from_line > *to_line || *to_line > len {
                    return Err(CoderError::BadInput(format!(
                        "range {from_line}-{to_line} invalid for file with {len} lines"
                    )));
                }
            }
            UpdateOp::Replace { .. } => unreachable!("line ops only"),
        }
    }
    for i in 0..ops.len() {
        for j in (i + 1)..ops.len() {
            if covers_overlap(ops[i], ops[j]) {
                return Err(CoderError::BadInput(format!(
                    "ops #{} and #{} overlap in original-line space",
                    i + 1,
                    j + 1
                )));
            }
        }
    }
    Ok(())
}

fn covers_overlap(a: &UpdateOp, b: &UpdateOp) -> bool {
    let (a_lo, a_hi) = cover(a);
    let (b_lo, b_hi) = cover(b);
    a_lo.max(b_lo) <= a_hi.min(b_hi)
}

/// Inclusive range of original lines an op touches. Inserts cover a single
/// point at their `at_line`.
fn cover(op: &UpdateOp) -> (u32, u32) {
    match op {
        UpdateOp::Insert { at_line, .. } => (*at_line, *at_line),
        UpdateOp::Remove { from_line, to_line } => (*from_line, *to_line),
        UpdateOp::UpdateLines {
            from_line, to_line, ..
        } => (*from_line, *to_line),
        UpdateOp::Replace { .. } => (0, 0),
    }
}

/// Split `content` into lines, dropping the trailing empty line a
/// final `\n` would create. `\r` from CRLF endings is trimmed.
fn split_content(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum LineEnding {
    Lf,
    CrLf,
}

fn split_file(bytes: &[u8]) -> (Vec<String>, LineEnding, bool) {
    let s = String::from_utf8_lossy(bytes);
    let ending = if s.contains("\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    let has_trailing = s.ends_with('\n');
    let lines: Vec<String> = s
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    (lines, ending, has_trailing)
}

fn join_lines(lines: &[String], ending: LineEnding, trailing: bool) -> Vec<u8> {
    let sep = match ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
    };
    let mut out = lines.join(sep);
    if trailing && !out.is_empty() {
        out.push_str(sep);
    } else if trailing && lines.is_empty() {
        // Preserve a file that was just "\n" or empty as empty.
    }
    out.into_bytes()
}

/// Write atomically via sibling temp file + rename.
fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), CoderError> {
    let parent = target
        .parent()
        .ok_or_else(|| CoderError::Io(format!("no parent for {}", target.display())))?;
    let mut tmp = std::ffi::OsString::from(target.file_name().unwrap_or_default());
    tmp.push(".coder-tmp-");
    tmp.push(format!("{}", std::process::id()));
    tmp.push("-");
    tmp.push(format!("{}", rand_suffix()));
    let tmp_path = parent.join(tmp);
    std::fs::write(&tmp_path, bytes).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        CoderError::Io(format!("tmp write: {e}"))
    })?;
    std::fs::rename(&tmp_path, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        CoderError::Io(format!("rename: {e}"))
    })?;
    Ok(())
}

/// Cheap unique suffix so concurrent updates on the same target don't
/// collide on the temp path. Uses nanos-mod-1M; collision probability
/// is acceptable for the rename-or-fail path below.
fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(s: &str) -> Vec<String> {
        s.lines().map(|l| l.to_string()).collect()
    }

    #[test]
    fn insert_at_1_prepends() {
        let mut lines = lines_of("a\nb\nc");
        apply_ops(
            &mut lines,
            &[UpdateOp::Insert {
                at_line: 1,
                content: "X".into(),
            }],
            3,
        )
        .unwrap();
        assert_eq!(lines, vec!["X", "a", "b", "c"]);
    }

    #[test]
    fn insert_at_end_plus_one_appends() {
        let mut lines = lines_of("a\nb\nc");
        apply_ops(
            &mut lines,
            &[UpdateOp::Insert {
                at_line: 4,
                content: "X".into(),
            }],
            3,
        )
        .unwrap();
        assert_eq!(lines, vec!["a", "b", "c", "X"]);
    }

    #[test]
    fn insert_multiline() {
        let mut lines = lines_of("a\nb");
        apply_ops(
            &mut lines,
            &[UpdateOp::Insert {
                at_line: 2,
                content: "X\nY".into(),
            }],
            2,
        )
        .unwrap();
        assert_eq!(lines, vec!["a", "X", "Y", "b"]);
    }

    #[test]
    fn remove_range() {
        let mut lines = lines_of("a\nb\nc\nd\ne");
        apply_ops(
            &mut lines,
            &[UpdateOp::Remove {
                from_line: 2,
                to_line: 4,
            }],
            5,
        )
        .unwrap();
        assert_eq!(lines, vec!["a", "e"]);
    }

    #[test]
    fn update_lines_range_multiline() {
        let mut lines = lines_of("a\nb\nc\nd");
        apply_ops(
            &mut lines,
            &[UpdateOp::UpdateLines {
                from_line: 2,
                to_line: 3,
                content: "X\nY\nZ".into(),
            }],
            4,
        )
        .unwrap();
        assert_eq!(lines, vec!["a", "X", "Y", "Z", "d"]);
    }

    #[test]
    fn bottom_up_apply_preserves_original_line_numbers() {
        let mut lines = lines_of("a\nb\nc\nd\ne");
        let ops = vec![
            UpdateOp::Insert {
                at_line: 1,
                content: "X".into(),
            },
            UpdateOp::Remove {
                from_line: 2,
                to_line: 4,
            },
            UpdateOp::UpdateLines {
                from_line: 5,
                to_line: 5,
                content: "E".into(),
            },
        ];
        apply_ops(&mut lines, &ops, 5).unwrap();
        assert_eq!(lines, vec!["X", "a", "E"]);
    }

    #[test]
    fn overlapping_remove_and_update_lines_rejected() {
        let mut lines = lines_of("a\nb\nc\nd");
        let err = apply_ops(
            &mut lines,
            &[
                UpdateOp::Remove {
                    from_line: 1,
                    to_line: 3,
                },
                UpdateOp::UpdateLines {
                    from_line: 3,
                    to_line: 4,
                    content: "X".into(),
                },
            ],
            4,
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn insert_inside_remove_range_rejected() {
        let mut lines = lines_of("a\nb\nc\nd");
        let err = apply_ops(
            &mut lines,
            &[
                UpdateOp::Remove {
                    from_line: 1,
                    to_line: 3,
                },
                UpdateOp::Insert {
                    at_line: 2,
                    content: "X".into(),
                },
            ],
            4,
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn duplicate_insert_at_same_line_rejected() {
        let mut lines = lines_of("a\nb");
        let err = apply_ops(
            &mut lines,
            &[
                UpdateOp::Insert {
                    at_line: 1,
                    content: "X".into(),
                },
                UpdateOp::Insert {
                    at_line: 1,
                    content: "Y".into(),
                },
            ],
            2,
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn out_of_range_remove_rejected() {
        let mut lines = lines_of("a\nb");
        let err = apply_ops(
            &mut lines,
            &[UpdateOp::Remove {
                from_line: 1,
                to_line: 9,
            }],
            2,
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn zero_line_rejected() {
        let mut lines = lines_of("a");
        let err = apply_ops(
            &mut lines,
            &[UpdateOp::Insert {
                at_line: 0,
                content: "X".into(),
            }],
            1,
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn empty_ops_rejected() {
        let mut lines = lines_of("a");
        let err = apply_ops(&mut lines, &[], 1).unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn split_file_detects_crlf() {
        let bytes = b"a\r\nb\r\nc\r\n";
        let (lines, ending, trailing) = split_file(bytes);
        assert_eq!(lines, vec!["a", "b", "c"]);
        assert!(matches!(ending, LineEnding::CrLf));
        assert!(trailing);
        let out = join_lines(&lines, ending, trailing);
        assert_eq!(out, b"a\r\nb\r\nc\r\n");
    }

    #[test]
    fn split_file_no_trailing_newline_preserved() {
        let bytes = b"a\nb";
        let (lines, ending, trailing) = split_file(bytes);
        assert_eq!(lines, vec!["a", "b"]);
        assert!(matches!(ending, LineEnding::Lf));
        assert!(!trailing);
        let out = join_lines(&lines, ending, trailing);
        assert_eq!(out, b"a\nb");
    }

    #[test]
    fn regex_replace_all_matches() {
        let out = apply_regex_replaces(
            b"foo bar foo".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "foo".into(),
                replacement: "baz".into(),
                ignore_case: false,
            }],
        )
        .unwrap();
        assert_eq!(out, b"baz bar baz");
    }

    #[test]
    fn regex_replace_capture_groups() {
        let out = apply_regex_replaces(
            b"HOST=8080".to_vec(),
            &[&UpdateOp::Replace {
                pattern: r"(\w+)=(\d+)".into(),
                replacement: "$1: $2".into(),
                ignore_case: false,
            }],
        )
        .unwrap();
        assert_eq!(out, b"HOST: 8080");
    }

    #[test]
    fn regex_replace_ignore_case() {
        let out = apply_regex_replaces(
            b"Foo FOO".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "foo".into(),
                replacement: "bar".into(),
                ignore_case: true,
            }],
        )
        .unwrap();
        assert_eq!(out, b"bar bar");
    }

    #[test]
    fn regex_replace_invalid_pattern_rejected() {
        let err = apply_regex_replaces(
            b"x".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "[unclosed".into(),
                replacement: "y".into(),
                ignore_case: false,
            }],
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn regex_replace_empty_pattern_rejected() {
        let err = apply_regex_replaces(
            b"x".to_vec(),
            &[&UpdateOp::Replace {
                pattern: String::new(),
                replacement: "y".into(),
                ignore_case: false,
            }],
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn regex_replace_no_match_is_noop() {
        let out = apply_regex_replaces(
            b"hello".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "missing".into(),
                replacement: "x".into(),
                ignore_case: false,
            }],
        )
        .unwrap();
        assert_eq!(out, b"hello");
    }
}

#[cfg(test)]
mod handler_tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_path: tmp.path().to_path_buf(),
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            max_write_bytes: 1024 * 1024,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn end_to_end_single_file_update_lines_writes_atomically() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "1\n2\n3\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::UpdateLines {
                        from_line: 2,
                        to_line: 2,
                        content: "TWO".into(),
                    }],
                }],
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 1);
        let r0 = &out.results[0];
        assert!(r0.success, "got: {:?}", r0.error);
        assert_eq!(r0.applied, 1);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "1\nTWO\n3\n"
        );
    }

    #[tokio::test]
    async fn end_to_end_regex_replace() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "foo bar foo\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "foo".into(),
                        replacement: "baz".into(),
                        ignore_case: false,
                    }],
                }],
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "baz bar baz\n"
        );
    }

    #[tokio::test]
    async fn mixed_update_lines_then_regex_replace() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "OLD\nkeep\nOLD\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![
                        UpdateOp::Remove {
                            from_line: 2,
                            to_line: 2,
                        },
                        UpdateOp::Replace {
                            pattern: "OLD".into(),
                            replacement: "NEW".into(),
                            ignore_case: false,
                        },
                    ],
                }],
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(out.results[0].applied, 2);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "NEW\nNEW\n"
        );
    }

    #[tokio::test]
    async fn regex_replace_introducing_newline_updates_line_count() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "a=b\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "=".into(),
                        replacement: "=\n".into(),
                        ignore_case: false,
                    }],
                }],
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(out.results[0].new_line_count, 2);
    }

    #[tokio::test]
    async fn non_accessible_path_yields_per_file_error_without_failing_batch() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), "API=1\n").unwrap();
        std::fs::write(tmp.path().join("a.txt"), "1\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![
                    UpdateFileSpec {
                        path: ".env".into(),
                        ops: vec![UpdateOp::Remove {
                            from_line: 1,
                            to_line: 1,
                        }],
                    },
                    UpdateFileSpec {
                        path: "a.txt".into(),
                        ops: vec![UpdateOp::Insert {
                            at_line: 1,
                            content: "X".into(),
                        }],
                    },
                ],
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(!out.results[0].success);
        assert!(out.results[0].error.as_deref().unwrap().contains("C211"));
        assert!(out.results[1].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "X\n1\n"
        );
    }

    #[tokio::test]
    async fn original_file_untouched_when_ops_invalid() {
        let (tmp, r, c) = setup();
        let original = "1\n2\n3\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![
                        UpdateOp::Remove {
                            from_line: 1,
                            to_line: 2,
                        },
                        UpdateOp::UpdateLines {
                            from_line: 2,
                            to_line: 3,
                            content: "X".into(),
                        },
                    ],
                }],
            },
        )
        .await
        .unwrap();
        assert!(!out.results[0].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn empty_files_array_rejected() {
        let (_tmp, r, c) = setup();
        let err = handle(r, c, UpdateFileInput { files: vec![] })
            .await
            .unwrap_err();
        assert!(err.contains("C210"));
    }
}
