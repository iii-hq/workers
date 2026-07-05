//! V4A patch format ("apply_patch"): parser + application engine.
//!
//! The format codex-family models are trained to emit:
//!
//! ```text
//! *** Begin Patch
//! [*** Environment ID: <id>]      (accepted and ignored)
//! *** Add File: <path>            followed by +<line> content lines
//! *** Delete File: <path>
//! *** Update File: <path>         [*** Move to: <path>] then chunks:
//! @@ <context line>               (or bare @@)
//!  <context> / -<old> / +<new>    change lines
//! *** End of File                 (optional, pins the chunk to EOF)
//! *** End Patch
//! ```
//!
//! `seek_sequence`, `compute_replacements`, and `apply_replacements` are
//! ported from OpenAI's `codex-rs/apply-patch` crate
//! (github.com/openai/codex, Apache-2.0) so context matching behaves
//! exactly like the reference implementation (exact → rstrip → trim →
//! unicode-normalised passes). The parser is a compact non-streaming
//! re-implementation of the same grammar, pinned by the reference test
//! cases in this module.

use std::path::PathBuf;

/// One parsed patch operation.
#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::enum_variant_names)] // mirrors the upstream crate
pub enum Hunk {
    AddFile {
        path: PathBuf,
        contents: String,
    },
    DeleteFile {
        path: PathBuf,
    },
    UpdateFile {
        path: PathBuf,
        move_path: Option<PathBuf>,
        chunks: Vec<UpdateFileChunk>,
    },
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct UpdateFileChunk {
    /// A single context line (usually a class/fn definition) that narrows
    /// down where `old_lines` should be searched for.
    pub change_context: Option<String>,
    /// Contiguous block of lines to replace with `new_lines`.
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    /// When true, `old_lines` must match at the end of the file.
    pub is_end_of_file: bool,
}

impl UpdateFileChunk {
    fn is_empty(&self) -> bool {
        self.change_context.is_none()
            && self.old_lines.is_empty()
            && self.new_lines.is_empty()
            && !self.is_end_of_file
    }
}

/// Parse failure with a prescriptive message (line numbers are 1-based
/// within the patch text).
#[derive(Debug, PartialEq)]
pub struct PatchError(pub String);

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

const BEGIN_MARKER: &str = "*** Begin Patch";
const END_MARKER: &str = "*** End Patch";
const ADD_MARKER: &str = "*** Add File: ";
const DELETE_MARKER: &str = "*** Delete File: ";
const UPDATE_MARKER: &str = "*** Update File: ";
const MOVE_MARKER: &str = "*** Move to: ";
const EOF_MARKER: &str = "*** End of File";
const ENV_ID_MARKER: &str = "*** Environment ID: ";

/// Parse a full patch text into hunks. Strict boundaries: the first
/// non-blank line must be `*** Begin Patch` and the last `*** End Patch`.
pub fn parse_patch(patch: &str) -> Result<Vec<Hunk>, PatchError> {
    let lines: Vec<&str> = patch.trim().lines().collect();
    match (lines.first(), lines.last()) {
        (Some(first), _) if first.trim() != BEGIN_MARKER => {
            return Err(PatchError(
                "the first line of the patch must be '*** Begin Patch'".into(),
            ));
        }
        (Some(_), Some(last)) if last.trim() == END_MARKER => {}
        _ => {
            return Err(PatchError(
                "the last line of the patch must be '*** End Patch'".into(),
            ));
        }
    }
    let body = &lines[1..lines.len() - 1];

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut i = 0usize;
    // 1-based line number of body[i] within the full patch text.
    let line_no = |i: usize| i + 2;

    while i < body.len() {
        let line = body[i];
        if let Some(path) = line.strip_prefix(ADD_MARKER) {
            let (contents, next) = parse_add_lines(body, i + 1);
            hunks.push(Hunk::AddFile {
                path: PathBuf::from(path.trim()),
                contents,
            });
            i = next;
        } else if let Some(path) = line.strip_prefix(DELETE_MARKER) {
            hunks.push(Hunk::DeleteFile {
                path: PathBuf::from(path.trim()),
            });
            i += 1;
        } else if let Some(path) = line.strip_prefix(UPDATE_MARKER) {
            let (hunk, next) = parse_update_hunk(body, i, path.trim())?;
            hunks.push(hunk);
            i = next;
        } else if line.strip_prefix(ENV_ID_MARKER).is_some() && hunks.is_empty() {
            i += 1; // accepted and ignored
        } else if line.trim().is_empty() {
            i += 1; // blank line between hunks
        } else {
            return Err(PatchError(format!(
                "unexpected line {} in patch: {:?} — every hunk starts with \
                 '*** Add File: ', '*** Delete File: ' or '*** Update File: '",
                line_no(i),
                line
            )));
        }
    }
    Ok(hunks)
}

/// Consume `+`-prefixed content lines of an Add File hunk.
fn parse_add_lines(body: &[&str], mut i: usize) -> (String, usize) {
    let mut contents = String::new();
    while i < body.len() {
        let Some(rest) = body[i].strip_prefix('+') else {
            break;
        };
        contents.push_str(rest);
        contents.push('\n');
        i += 1;
    }
    (contents, i)
}

/// Parse an Update File hunk starting at `body[start]` (the marker line).
fn parse_update_hunk(body: &[&str], start: usize, path: &str) -> Result<(Hunk, usize), PatchError> {
    let mut i = start + 1;
    let move_path = if i < body.len() {
        body[i].strip_prefix(MOVE_MARKER).map(|p| {
            i += 1;
            PathBuf::from(p.trim())
        })
    } else {
        None
    };

    let mut chunks: Vec<UpdateFileChunk> = Vec::new();
    let mut current = UpdateFileChunk::default();
    let mut current_started = false;
    let mut flush = |current: &mut UpdateFileChunk, started: &mut bool| -> Result<(), PatchError> {
        if *started && !current.is_empty() {
            chunks.push(std::mem::take(current));
        }
        *started = false;
        Ok(())
    };

    while i < body.len() {
        let line = body[i];
        if line == EOF_MARKER {
            // Per the grammar (`change: (change_context | change_line)+
            // eof_line?`) the EOF marker terminates the whole update hunk.
            current.is_end_of_file = true;
            current_started = true;
            flush(&mut current, &mut current_started)?;
            i += 1;
            break;
        }
        if line.starts_with("*** ") {
            break; // next hunk marker
        }
        if line == "@@" || line.starts_with("@@ ") {
            flush(&mut current, &mut current_started)?;
            current.change_context = line.strip_prefix("@@ ").map(|c| c.to_string());
            current_started = true;
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            current.new_lines.push(rest.to_string());
        } else if let Some(rest) = line.strip_prefix('-') {
            current.old_lines.push(rest.to_string());
        } else if let Some(rest) = line.strip_prefix(' ') {
            current.old_lines.push(rest.to_string());
            current.new_lines.push(rest.to_string());
        } else if line.is_empty() {
            // Models routinely trim the ' ' diff prefix off blank context
            // lines; treat a fully empty line as empty context.
            current.old_lines.push(String::new());
            current.new_lines.push(String::new());
        } else {
            return Err(PatchError(format!(
                "unexpected line {} in update hunk for {:?}: {:?} — change \
                 lines must start with '+', '-', ' ' or '@@'",
                i + 2,
                path,
                line
            )));
        }
        current_started = true;
        i += 1;
    }
    flush(&mut current, &mut current_started)?;

    if chunks.is_empty() {
        return Err(PatchError(format!(
            "update file hunk for path {path:?} is empty"
        )));
    }
    Ok((
        Hunk::UpdateFile {
            path: PathBuf::from(path),
            move_path,
            chunks,
        },
        i,
    ))
}

// ---------------------------------------------------------------------------
// Context matching + replacement — ported from codex-rs/apply-patch
// (github.com/openai/codex, Apache-2.0).
// ---------------------------------------------------------------------------

/// Find `pattern` within `lines` at or after `start`, with decreasing
/// strictness: exact, rstrip, trim, then unicode-normalised. When `eof`
/// is true, try matching at the end of file first.
pub(crate) fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let search_start = if eof && lines.len() >= pattern.len() {
        lines.len() - pattern.len()
    } else {
        start
    };
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        if pattern
            .iter()
            .enumerate()
            .all(|(p, pat)| lines[i + p].trim_end() == pat.trim_end())
        {
            return Some(i);
        }
    }
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        if pattern
            .iter()
            .enumerate()
            .all(|(p, pat)| lines[i + p].trim() == pat.trim())
        {
            return Some(i);
        }
    }

    fn normalise(s: &str) -> String {
        s.trim()
            .chars()
            .map(|c| match c {
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
                | '\u{2212}' => '-',
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
                '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
                | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
                | '\u{3000}' => ' ',
                other => other,
            })
            .collect()
    }
    (search_start..=lines.len().saturating_sub(pattern.len())).find(|&i| {
        pattern
            .iter()
            .enumerate()
            .all(|(p, pat)| normalise(&lines[i + p]) == normalise(pat))
    })
}

/// Result of applying update chunks to one file's contents.
#[derive(Debug)]
pub struct AppliedUpdate {
    pub new_contents: String,
    /// 1-based line (in the NEW contents) where the first replacement
    /// landed, with that replacement's new-line count — enough for a
    /// bounded verification echo. `None` when the chunks were a no-op.
    pub first_change: Option<(u64, u64)>,
}

/// Apply `chunks` to `original_contents`, returning the new contents.
/// Fails with a prescriptive message when a chunk's context cannot be
/// located (the caller maps this to C210; nothing is written).
pub fn derive_new_contents_from_chunks(
    original_contents: &str,
    path: &str,
    chunks: &[UpdateFileChunk],
) -> Result<AppliedUpdate, PatchError> {
    let mut original_lines: Vec<String> = original_contents.split('\n').map(String::from).collect();
    // Drop the trailing empty element from the final newline so line
    // counts match standard `diff` behaviour.
    if original_lines.last().is_some_and(String::is_empty) {
        original_lines.pop();
    }
    let replacements = compute_replacements(&original_lines, path, chunks)?;
    // Replacements apply in DESCENDING order, so the FIRST (lowest-index)
    // replacement's position is never shifted by the others — its index is
    // valid in the new contents as-is.
    let first_change = replacements
        .first()
        .map(|(idx, _, new)| (*idx as u64 + 1, new.len() as u64));
    let mut new_lines = apply_replacements(original_lines, &replacements);
    if !new_lines.last().is_some_and(String::is_empty) {
        new_lines.push(String::new());
    }
    Ok(AppliedUpdate {
        new_contents: new_lines.join("\n"),
        first_change,
    })
}

/// `(start_index, old_len, new_lines)` replacements, in ascending order.
pub(crate) fn compute_replacements(
    original_lines: &[String],
    path: &str,
    chunks: &[UpdateFileChunk],
) -> Result<Vec<(usize, usize, Vec<String>)>, PatchError> {
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut line_index: usize = 0;

    for chunk in chunks {
        if let Some(ctx_line) = &chunk.change_context {
            if let Some(idx) = seek_sequence(
                original_lines,
                std::slice::from_ref(ctx_line),
                line_index,
                false,
            ) {
                line_index = idx + 1;
            } else {
                return Err(PatchError(format!(
                    "failed to find context {ctx_line:?} in {path} — re-read \
                     the file and regenerate the patch against its current \
                     contents"
                )));
            }
        }

        if chunk.old_lines.is_empty() {
            // Pure addition: append at the end of the file.
            let insertion_idx = if original_lines.last().is_some_and(String::is_empty) {
                original_lines.len() - 1
            } else {
                original_lines.len()
            };
            replacements.push((insertion_idx, 0, chunk.new_lines.clone()));
            continue;
        }

        // A trailing empty old-line often represents the file's final
        // newline; retry without it when the direct search fails.
        let mut pattern: &[String] = &chunk.old_lines;
        let mut found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        let mut new_slice: &[String] = &chunk.new_lines;
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        }

        if let Some(start_idx) = found {
            replacements.push((start_idx, pattern.len(), new_slice.to_vec()));
            line_index = start_idx + pattern.len();
        } else {
            return Err(PatchError(format!(
                "failed to find expected lines in {}:\n{}\n— re-read the file \
                 and regenerate the patch against its current contents",
                path,
                chunk.old_lines.join("\n"),
            )));
        }
    }

    replacements.sort_by_key(|(index, _, _)| *index);
    Ok(replacements)
}

/// Apply replacements in descending order so earlier ones don't shift
/// later positions.
pub(crate) fn apply_replacements(
    mut lines: Vec<String>,
    replacements: &[(usize, usize, Vec<String>)],
) -> Vec<String> {
    for (start_idx, old_len, new_segment) in replacements.iter().rev() {
        let start_idx = *start_idx;
        for _ in 0..*old_len {
            if start_idx < lines.len() {
                lines.remove(start_idx);
            }
        }
        for (offset, new_line) in new_segment.iter().enumerate() {
            lines.insert(start_idx + offset, new_line.clone());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use Hunk::*;

    // -----------------------------------------------------------------
    // Parser cases pinned against codex-rs/apply-patch's reference tests.
    // -----------------------------------------------------------------

    #[test]
    fn rejects_missing_boundaries() {
        assert!(parse_patch("bad").unwrap_err().0.contains("Begin Patch"));
        assert!(parse_patch("*** Begin Patch\nbad")
            .unwrap_err()
            .0
            .contains("End Patch"));
    }

    #[test]
    fn empty_patch_yields_no_hunks() {
        assert_eq!(
            parse_patch("*** Begin Patch\n*** End Patch").unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn parses_all_three_hunk_kinds_with_move() {
        let hunks = parse_patch(
            "*** Begin Patch\n\
             *** Add File: path/add.py\n\
             +abc\n\
             +def\n\
             *** Delete File: path/delete.py\n\
             *** Update File: path/update.py\n\
             *** Move to: path/update2.py\n\
             @@ def f():\n\
             -    pass\n\
             +    return 123\n\
             *** End Patch",
        )
        .unwrap();
        assert_eq!(
            hunks,
            vec![
                AddFile {
                    path: PathBuf::from("path/add.py"),
                    contents: "abc\ndef\n".to_string()
                },
                DeleteFile {
                    path: PathBuf::from("path/delete.py")
                },
                UpdateFile {
                    path: PathBuf::from("path/update.py"),
                    move_path: Some(PathBuf::from("path/update2.py")),
                    chunks: vec![UpdateFileChunk {
                        change_context: Some("def f():".to_string()),
                        old_lines: vec!["    pass".to_string()],
                        new_lines: vec!["    return 123".to_string()],
                        is_end_of_file: false
                    }]
                }
            ]
        );
    }

    #[test]
    fn update_followed_by_add() {
        let hunks = parse_patch(
            "*** Begin Patch\n\
             *** Update File: file.py\n\
             @@\n\
             +line\n\
             *** Add File: other.py\n\
             +content\n\
             *** End Patch",
        )
        .unwrap();
        assert_eq!(
            hunks,
            vec![
                UpdateFile {
                    path: PathBuf::from("file.py"),
                    move_path: None,
                    chunks: vec![UpdateFileChunk {
                        change_context: None,
                        old_lines: vec![],
                        new_lines: vec!["line".to_string()],
                        is_end_of_file: false
                    }],
                },
                AddFile {
                    path: PathBuf::from("other.py"),
                    contents: "content\n".to_string()
                }
            ]
        );
    }

    #[test]
    fn update_without_context_header_parses() {
        let hunks = parse_patch(
            "*** Begin Patch\n*** Update File: file2.py\n import foo\n+bar\n*** End Patch",
        )
        .unwrap();
        assert_eq!(
            hunks,
            vec![UpdateFile {
                path: PathBuf::from("file2.py"),
                move_path: None,
                chunks: vec![UpdateFileChunk {
                    change_context: None,
                    old_lines: vec!["import foo".to_string()],
                    new_lines: vec!["import foo".to_string(), "bar".to_string()],
                    is_end_of_file: false,
                }],
            }]
        );
    }

    #[test]
    fn end_of_file_marker_is_preserved() {
        let hunks = parse_patch(
            "*** Begin Patch\n*** Update File: file.txt\n@@\n+quux\n*** End of File\n\n*** End Patch",
        )
        .unwrap();
        assert_eq!(
            hunks,
            vec![UpdateFile {
                path: PathBuf::from("file.txt"),
                move_path: None,
                chunks: vec![UpdateFileChunk {
                    change_context: None,
                    old_lines: Vec::new(),
                    new_lines: vec!["quux".to_string()],
                    is_end_of_file: true,
                }],
            }]
        );
    }

    #[test]
    fn empty_update_hunk_is_rejected() {
        let err =
            parse_patch("*** Begin Patch\n*** Update File: test.py\n*** End Patch").unwrap_err();
        assert!(err.0.contains("is empty"), "{err}");
    }

    #[test]
    fn environment_id_preamble_is_ignored() {
        let hunks = parse_patch(
            "*** Begin Patch\n\
             *** Environment ID: remote\n\
             *** Add File: hello.txt\n\
             +hello\n\
             *** End Patch",
        )
        .unwrap();
        assert_eq!(
            hunks,
            vec![AddFile {
                path: PathBuf::from("hello.txt"),
                contents: "hello\n".to_string(),
            }]
        );
    }

    #[test]
    fn multiple_chunks_in_one_update() {
        let hunks = parse_patch(
            "*** Begin Patch\n\
             *** Update File: f.py\n\
             @@ def a():\n\
             -    x = 1\n\
             +    x = 2\n\
             @@ def b():\n\
             -    y = 1\n\
             +    y = 2\n\
             *** End Patch",
        )
        .unwrap();
        match &hunks[0] {
            UpdateFile { chunks, .. } => assert_eq!(chunks.len(), 2),
            other => panic!("expected update, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Application semantics.
    // -----------------------------------------------------------------

    fn chunk(ctx: Option<&str>, old: &[&str], new: &[&str]) -> UpdateFileChunk {
        UpdateFileChunk {
            change_context: ctx.map(str::to_string),
            old_lines: old.iter().map(|s| s.to_string()).collect(),
            new_lines: new.iter().map(|s| s.to_string()).collect(),
            is_end_of_file: false,
        }
    }

    #[test]
    fn applies_simple_replacement() {
        let out =
            derive_new_contents_from_chunks("a\nb\nc\n", "f.txt", &[chunk(None, &["b"], &["B"])])
                .unwrap()
                .new_contents;
        assert_eq!(out, "a\nB\nc\n");
    }

    #[test]
    fn applies_with_context_narrowing() {
        let src = "def a():\n    pass\n\ndef b():\n    pass\n";
        let out = derive_new_contents_from_chunks(
            src,
            "f.py",
            &[chunk(Some("def b():"), &["    pass"], &["    return 1"])],
        )
        .unwrap()
        .new_contents;
        assert_eq!(out, "def a():\n    pass\n\ndef b():\n    return 1\n");
    }

    #[test]
    fn pure_addition_appends_at_end() {
        let out =
            derive_new_contents_from_chunks("a\n", "f.txt", &[chunk(None, &[], &["z"])]).unwrap();
        assert_eq!(out.new_contents, "a\nz\n");
    }

    #[test]
    fn missing_context_fails_with_reread_guidance() {
        let err =
            derive_new_contents_from_chunks("a\nb\n", "f.txt", &[chunk(None, &["nope"], &["x"])])
                .unwrap_err();
        assert!(err.0.contains("re-read the file"), "{err}");
    }

    #[test]
    fn fuzzy_match_tolerates_trailing_whitespace() {
        let out = derive_new_contents_from_chunks(
            "keep   \nold\n",
            "f.txt",
            &[chunk(None, &["keep", "old"], &["keep", "new"])],
        )
        .unwrap();
        assert_eq!(out.new_contents, "keep\nnew\n");
    }
}
