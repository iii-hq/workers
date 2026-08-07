//! Editing `workers-dev.yaml` in place.
//!
//! The file is hand-written as often as it is tool-written, so writes are
//! line surgery on the original text rather than a serde round-trip: comments,
//! key order, and blank lines all survive because they are never re-serialized.
//! Anything the scanner cannot own confidently (inline `stacks: {…}`, a
//! duplicated key) is refused rather than rewritten.

use std::path::Path;

use anyhow::{bail, Context, Result};

/// Stack names land in YAML as plain scalars, so restrict them to characters
/// that can never need quoting or change the document's shape.
pub fn valid_stack_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Insert or replace `name`'s entry under `stacks:`, leaving every other byte
/// of the file alone.
pub fn upsert_stack(text: &str, name: &str, roots: &[String]) -> Result<String> {
    if !valid_stack_name(name) {
        bail!("invalid stack name {name:?} (use letters, digits, - and _)");
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(head) = top_level_key(&lines, "stacks")? else {
        let mut out = ensure_trailing_newline(text);
        out.push_str("stacks:\n");
        for line in render_entry("  ", name, roots) {
            out.push_str(&line);
            out.push('\n');
        }
        return Ok(out);
    };
    ensure_block_style(&lines, head)?;
    let block = block_range(&lines, head);
    let entry = render_entry(&block_indent(&lines, block), name, roots);
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match entry_range(&lines, block, name) {
        Some((start, end)) => out.splice(start..end, entry),
        None => out.splice(block.1..block.1, entry),
    };
    Ok(out.join("\n"))
}

/// Drop `name`'s entry. Removing the last entry drops the `stacks:` key too,
/// rather than leaving a dangling header.
pub fn remove_stack(text: &str, name: &str) -> Result<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let missing = || format!("stack {name} is not defined in this file");
    let Some(head) = top_level_key(&lines, "stacks")? else {
        bail!(missing());
    };
    ensure_block_style(&lines, head)?;
    let block = block_range(&lines, head);
    let Some((start, end)) = entry_range(&lines, block, name) else {
        bail!(missing());
    };
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.drain(start..end);

    let after: Vec<&str> = out.iter().map(String::as_str).collect();
    let rest = block_range(&after, head);
    let has_entry = (rest.0..rest.1).any(|i| entry_key(strip_cr(after[i])).is_some());
    if !has_entry {
        out.drain(head..rest.1.max(head + 1));
    }
    Ok(out.join("\n"))
}

/// Point `default_stack:` at `name`, replacing the existing key or appending it.
pub fn set_default_stack(text: &str, name: &str) -> Result<String> {
    if !valid_stack_name(name) {
        bail!("invalid stack name {name:?} (use letters, digits, - and _)");
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(i) = top_level_key(&lines, "default_stack")? else {
        return Ok(format!(
            "{}default_stack: {name}\n",
            ensure_trailing_newline(text)
        ));
    };
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let cr = if lines[i].ends_with('\r') { "\r" } else { "" };
    let comment = trailing_comment(strip_cr(lines[i]));
    out[i] = format!("default_stack: {name}{comment}{cr}");
    Ok(out.join("\n"))
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// The trailing `  # comment` on a `default_stack:` line, including its
/// leading whitespace so the user's alignment survives a rewrite — or `""`
/// if there's none. YAML only starts a comment at a `#` that sits at the
/// start of the line or is immediately preceded by whitespace; a `#` glued
/// to the value (`console#nospacecomment`) or inside a quoted scalar
/// (`"my#stack"`) is part of the VALUE, not a comment. This module's own
/// header says the file is hand-written as often as tool-written, so either
/// shape can already be sitting on this line — misreading it as a comment
/// would silently rewrite that value into garbage instead of discarding it
/// with the rest of the old line.
fn trailing_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let Some(hash) = (0..bytes.len())
        .find(|&i| bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()))
    else {
        return "";
    };
    let ws_start = line[..hash]
        .rfind(|c: char| !c.is_whitespace())
        .map_or(0, |i| i + 1);
    &line[ws_start..]
}

fn ensure_trailing_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}

/// Line index of a top-level `key:`. Errors when the key appears twice —
/// which one wins is a question for a human, not for line surgery.
fn top_level_key(lines: &[&str], key: &str) -> Result<Option<usize>> {
    let mut found = None;
    for (i, line) in lines.iter().enumerate() {
        let line = strip_cr(line);
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        if !rest.starts_with(':') {
            continue;
        }
        if found.is_some() {
            bail!("`{key}:` appears more than once in this file — fix it by hand");
        }
        found = Some(i);
    }
    Ok(found)
}

/// Refuse a `stacks: {…}` written inline; only block style is ours to edit.
/// A trailing comment after the colon (`stacks: # my dev stacks`) is valid
/// block style, not inline content.
fn ensure_block_style(lines: &[&str], head: usize) -> Result<()> {
    let after = strip_cr(lines[head])
        .split_once(':')
        .map(|(_, rest)| rest.trim())
        .unwrap_or("");
    if !after.is_empty() && !after.starts_with('#') {
        bail!("`stacks:` is written inline in this file — edit it by hand");
    }
    Ok(())
}

/// True for a blank line or a comment line at any column — YAML comments
/// aren't bound to the surrounding block's indentation, so one test serves
/// both "does this end the block" and "does this belong to whatever line
/// follows it" (`block_range` and `leading_gap_run`). Keeping both callers
/// on this one function is deliberate: they drifted apart once already
/// (`block_range` went column-agnostic before `leading_gap_run` did, which
/// is exactly how a comment ended up silently deleted).
fn is_blank_or_comment(line: &str) -> bool {
    line.trim().is_empty() || line.trim_start().starts_with('#')
}

/// Half-open line range of the indented block under `head`. Blank lines and
/// comment lines (YAML allows comments at any column) don't end it; trailing
/// blanks are left outside.
fn block_range(lines: &[&str], head: usize) -> (usize, usize) {
    let start = head + 1;
    let mut end = start;
    for (i, line) in lines.iter().enumerate().skip(start) {
        let line = strip_cr(line);
        if is_blank_or_comment(line) {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            end = i + 1;
        } else {
            break;
        }
    }
    (start, end)
}

/// `("  ", "console")` for a `  console:` line; None for list items and blanks.
fn entry_key(line: &str) -> Option<(String, String)> {
    let indent: String = line.chars().take_while(|c| *c == ' ').collect();
    if indent.is_empty() || indent.len() == line.len() {
        return None;
    }
    let rest = &line[indent.len()..];
    let key = rest.split(':').next()?;
    if key.is_empty() || key.len() == rest.len() || !valid_stack_name(key) {
        return None;
    }
    Some((indent, key.to_string()))
}

/// Indentation the block's entries already use, or two spaces for a new block.
fn block_indent(lines: &[&str], block: (usize, usize)) -> String {
    (block.0..block.1)
        .find_map(|i| entry_key(strip_cr(lines[i])).map(|(indent, _)| indent))
        .unwrap_or_else(|| "  ".to_string())
}

/// Half-open line range of `name`'s entry: its key line through the line
/// before the next entry at the same indentation (or the end of the block).
fn entry_range(lines: &[&str], block: (usize, usize), name: &str) -> Option<(usize, usize)> {
    let mut start: Option<(String, usize)> = None;
    for i in block.0..block.1 {
        let Some((indent, key)) = entry_key(strip_cr(lines[i])) else {
            continue;
        };
        match &start {
            None if key == name => start = Some((indent, i)),
            None => {}
            Some((open_indent, open)) if indent.len() <= open_indent.len() => {
                return Some((*open, leading_gap_run(lines, i, open + 1)));
            }
            Some(_) => {}
        }
    }
    start.map(|(_, open)| (open, block.1))
}

/// Back `at` up over a run of blank lines and comment lines — at any column,
/// same rule as `block_range` — stopping no earlier than `floor` (never the
/// entry's own key line: callers pass `floor = open + 1`). A blank or
/// comment line immediately above a key reads as part of that key's own
/// entry, not trailing content of the entry before it, so both must stay
/// out of that entry's replace/remove range.
///
/// Deliberate, not an oversight: this can only ever push a comment onto the
/// *following* entry, never delete one. A comment that trails the entry
/// being replaced/removed (e.g. a note on `tiny`'s last line, right before
/// `console:`) ends up orphaned onto `console` instead — visible and still
/// valid YAML, unlike silently deleting the user's writing.
fn leading_gap_run(lines: &[&str], at: usize, floor: usize) -> usize {
    let mut at = at;
    while at > floor && is_blank_or_comment(strip_cr(lines[at - 1])) {
        at -= 1;
    }
    at
}

fn render_entry(indent: &str, name: &str, roots: &[String]) -> Vec<String> {
    let mut out = vec![format!("{indent}{name}:")];
    for root in roots {
        out.push(format!("{indent}{indent}- {root}"));
    }
    out
}

/// Replace `path`'s contents with `text`, but only after proving the result
/// still loads. Writes a sibling temp file and renames over the target, so a
/// crash mid-write cannot leave a half-written config behind.
pub fn write_verified(path: &Path, text: &str) -> Result<()> {
    crate::config::validate_config_text(text)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workers-dev.yaml".to_string());
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, text)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .with_context(|| format!("replace {} with {}", path.display(), tmp.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// Every text this module produces must still parse as YAML.
    fn parses(text: &str) -> bool {
        serde_yaml::from_str::<serde_yaml::Value>(text).is_ok()
    }

    #[test]
    fn creates_stacks_block_in_an_empty_file() {
        let out = upsert_stack("", "console", &roots(&["console", "session-manager"])).unwrap();
        assert_eq!(
            out,
            "stacks:\n  console:\n    - console\n    - session-manager\n"
        );
        assert!(parses(&out));
    }

    /// A file with other keys keeps them, and the block is appended.
    #[test]
    fn appends_block_to_a_file_without_stacks() {
        let src = "# my dev config\nengine_url: ws://127.0.0.1:49134\nrelease: false\n";
        let out = upsert_stack(src, "tiny", &roots(&["session-manager"])).unwrap();
        assert!(out.starts_with(src), "existing content must be untouched");
        assert!(out.ends_with("stacks:\n  tiny:\n    - session-manager\n"));
        assert!(parses(&out));
    }

    /// The whole point: comments around the edited entry survive.
    #[test]
    fn replaces_an_entry_and_preserves_comments() {
        let src = "\
stacks:
  # the console loop
  console:
    - console
  # everything else
  tiny:
    - session-manager
default_stack: tiny
";
        let out = upsert_stack(src, "console", &roots(&["console", "state"])).unwrap();
        assert!(out.contains("# the console loop"));
        assert!(out.contains("# everything else"));
        assert!(out.contains("  console:\n    - console\n    - state\n"));
        assert!(out.contains("  tiny:\n    - session-manager\n"));
        assert!(out.contains("default_stack: tiny"));
        assert!(parses(&out));
    }

    #[test]
    fn inserts_a_new_entry_at_the_end_of_the_block() {
        let src = "stacks:\n  tiny:\n    - session-manager\ncolor: auto\n";
        let out = upsert_stack(src, "console", &roots(&["console"])).unwrap();
        assert_eq!(
            out,
            "stacks:\n  tiny:\n    - session-manager\n  console:\n    - console\ncolor: auto\n"
        );
        assert!(parses(&out));
    }

    #[test]
    fn upsert_is_idempotent() {
        let src = "stacks:\n  tiny:\n    - session-manager\n";
        let once = upsert_stack(src, "console", &roots(&["console"])).unwrap();
        let twice = upsert_stack(&once, "console", &roots(&["console"])).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn removes_one_entry_and_keeps_the_others() {
        let src = "stacks:\n  tiny:\n    - session-manager\n  console:\n    - console\n";
        let out = remove_stack(src, "tiny").unwrap();
        assert_eq!(out, "stacks:\n  console:\n    - console\n");
        assert!(parses(&out));
    }

    /// Removing the last entry drops the now-empty `stacks:` key.
    #[test]
    fn removing_the_last_entry_drops_the_stacks_key() {
        let src = "engine_url: ws://x:1\nstacks:\n  tiny:\n    - session-manager\n";
        let out = remove_stack(src, "tiny").unwrap();
        assert_eq!(out, "engine_url: ws://x:1\n");
        assert!(parses(&out));
    }

    #[test]
    fn remove_reports_an_unknown_stack() {
        let src = "stacks:\n  tiny:\n    - session-manager\n";
        let err = remove_stack(src, "nope").unwrap_err();
        assert!(
            err.to_string().contains("not defined in this file"),
            "{err:#}"
        );
        let err = remove_stack("release: false\n", "tiny").unwrap_err();
        assert!(
            err.to_string().contains("not defined in this file"),
            "{err:#}"
        );
    }

    #[test]
    fn sets_default_in_place_or_appends_it() {
        let replaced = set_default_stack("default_stack: tiny\ncolor: auto\n", "console").unwrap();
        assert_eq!(replaced, "default_stack: console\ncolor: auto\n");
        let appended = set_default_stack("color: auto\n", "console").unwrap();
        assert_eq!(appended, "color: auto\ndefault_stack: console\n");
        assert!(parses(&replaced) && parses(&appended));
        // No comment on the line: unchanged behavior, covered above already.
    }

    /// A trailing comment on `default_stack:` — and the whitespace the user
    /// chose before it — must survive a value change, not just the block
    /// comments `upsert_stack`/`remove_stack` already protect.
    #[test]
    fn set_default_preserves_a_trailing_comment_and_its_spacing() {
        let out = set_default_stack("default_stack: tiny  # my usual loop\n", "console").unwrap();
        assert_eq!(out, "default_stack: console  # my usual loop\n");
        assert!(parses(&out));
    }

    /// Same as above, on a CRLF line: the comment must survive AND the line
    /// must still end with `\r`, not just one or the other.
    #[test]
    fn set_default_preserves_a_trailing_comment_on_a_crlf_line() {
        let out = set_default_stack(
            "default_stack: tiny  # my usual loop\r\ncolor: auto\r\n",
            "console",
        )
        .unwrap();
        assert_eq!(
            out,
            "default_stack: console  # my usual loop\r\ncolor: auto\r\n"
        );
        assert!(parses(&out));
    }

    /// A `#` glued directly to the value (no preceding whitespace) isn't a
    /// YAML comment — it's part of the plain scalar. The whole old value
    /// must be discarded with the rest of the line, not misread as a
    /// comment and re-emitted after the new name.
    #[test]
    fn set_default_does_not_mistake_a_glued_hash_for_a_comment() {
        let out = set_default_stack("default_stack: console#nospacecomment\n", "tiny").unwrap();
        assert_eq!(out, "default_stack: tiny\n");
        assert!(parses(&out));
    }

    /// Same failure mode, via a `#` inside a quoted value instead of glued
    /// to a plain one.
    #[test]
    fn set_default_does_not_mistake_a_hash_inside_a_quoted_value_for_a_comment() {
        let out = set_default_stack("default_stack: \"my#stack\"\n", "tiny").unwrap();
        assert_eq!(out, "default_stack: tiny\n");
        assert!(parses(&out));
    }

    /// Inline/flow style is not ours to edit — refuse instead of mangling it.
    #[test]
    fn refuses_inline_stacks_mapping() {
        let src = "stacks: {tiny: [session-manager]}\n";
        let err = upsert_stack(src, "console", &roots(&["console"])).unwrap_err();
        assert!(err.to_string().contains("inline"), "{err:#}");
    }

    #[test]
    fn refuses_a_duplicated_stacks_key() {
        let src = "stacks:\n  a:\n    - x\nstacks:\n  b:\n    - y\n";
        let err = upsert_stack(src, "c", &roots(&["z"])).unwrap_err();
        assert!(err.to_string().contains("more than once"), "{err:#}");
    }

    #[test]
    fn refuses_names_that_would_need_quoting() {
        let err = upsert_stack("", "my stack", &roots(&["x"])).unwrap_err();
        assert!(err.to_string().contains("invalid stack name"), "{err:#}");
        assert!(valid_stack_name("console-dev_2"));
        assert!(!valid_stack_name(""));
        assert!(!valid_stack_name("a:b"));
    }

    /// CRLF files stay CRLF on the lines we don't touch.
    #[test]
    fn preserves_crlf_line_endings() {
        let src = "color: auto\r\nstacks:\r\n  tiny:\r\n    - session-manager\r\n";
        let out = upsert_stack(src, "console", &roots(&["console"])).unwrap();
        assert!(out.starts_with("color: auto\r\n"));
        assert!(out.contains("  tiny:\r\n"));
        assert!(out.contains("  console:\n    - console\n"));
    }

    /// YAML comments are legal at any column. A column-0 comment between two
    /// entries must not truncate block detection, or an entry past it goes
    /// invisible: upsert would then append a duplicate key instead of
    /// replacing the existing one, and remove would report it missing.
    #[test]
    fn sees_past_a_column_zero_comment_inside_the_block() {
        let src = "stacks:\n  console:\n    - console\n# a stray column-0 comment\n  tiny:\n    - session-manager\n";

        let out = upsert_stack(src, "tiny", &roots(&["y"])).unwrap();
        assert_eq!(out.matches("tiny:").count(), 1, "{out:?}");
        assert!(out.contains("# a stray column-0 comment"));
        assert!(parses(&out));

        let out = remove_stack(src, "tiny").unwrap();
        assert!(!out.contains("tiny"), "{out:?}");
        assert!(out.contains("# a stray column-0 comment"));
        assert!(parses(&out));
    }

    /// A blank line separating two entries is layout, not part of either
    /// entry's content, on both the replace and the remove path.
    #[test]
    fn preserves_a_blank_line_between_entries() {
        let src = "stacks:\n  tiny:\n    - session-manager\n\n  console:\n    - console\n";

        let out = upsert_stack(src, "tiny", &roots(&["x"])).unwrap();
        assert!(out.contains("- x\n\n  console:"), "{out:?}");
        assert!(parses(&out));

        let out = remove_stack(src, "tiny").unwrap();
        assert!(out.contains("stacks:\n\n  console:"), "{out:?}");
        assert!(parses(&out));
    }

    /// A trailing comment after `stacks:` is still block style, not inline.
    #[test]
    fn accepts_a_trailing_comment_on_the_stacks_header() {
        let src = "stacks: # my dev stacks\n";
        let out = upsert_stack(src, "console", &roots(&["x"])).unwrap();
        assert!(out.starts_with("stacks: # my dev stacks\n"));
        assert!(out.contains("  console:\n    - x\n"));
        assert!(parses(&out));
    }

    /// Sibling of `sees_past_a_column_zero_comment_inside_the_block`: that one
    /// edits the entry AFTER the mismatched-indent comment, this one edits
    /// the entry BEFORE it. Both directions must leave the comment intact —
    /// it must never fall inside the range being replaced or removed just
    /// because its column doesn't match the entry that follows it.
    #[test]
    fn preserves_a_column_zero_comment_when_editing_the_entry_before_it() {
        let src = "stacks:\n  console:\n    - console\n# a stray column-0 comment\n  tiny:\n    - session-manager\n";

        let out = upsert_stack(src, "console", &roots(&["console", "state"])).unwrap();
        assert!(out.contains("# a stray column-0 comment"), "{out:?}");
        assert!(parses(&out));

        let out = remove_stack(src, "console").unwrap();
        assert!(out.contains("# a stray column-0 comment"), "{out:?}");
        assert!(parses(&out));
    }

    /// A run mixing blank lines and an oddly-indented comment between two
    /// entries must survive as a unit, in order, on both operations.
    #[test]
    fn preserves_mixed_blanks_and_comments_between_entries() {
        let src =
            "stacks:\n  tiny:\n    - session-manager\n\n# stray note\n  console:\n    - console\n";

        let out = upsert_stack(src, "tiny", &roots(&["x"])).unwrap();
        assert!(out.contains("- x\n\n# stray note\n  console:"), "{out:?}");
        assert!(parses(&out));

        let out = remove_stack(src, "tiny").unwrap();
        assert!(
            out.contains("stacks:\n\n# stray note\n  console:"),
            "{out:?}"
        );
        assert!(parses(&out));
    }

    #[test]
    fn write_verified_replaces_the_file_atomically() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::write(&path, "color: auto\n").unwrap();

        write_verified(&path, "color: auto\nstacks:\n  a:\n    - b\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "color: auto\nstacks:\n  a:\n    - b\n"
        );
        // No temp file left behind.
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file was not renamed away");
    }

    #[test]
    fn write_verified_refuses_text_the_loader_cannot_parse() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::write(&path, "color: auto\n").unwrap();

        let err = write_verified(&path, "stacks:\n  a:\n  - b\n  bad\n").unwrap_err();
        assert!(!err.to_string().is_empty());
        // The original file is untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "color: auto\n");
    }

    /// A digits-only (or `true`/`false`/`null`/...) stack name writes as valid
    /// YAML text — `upsert_stack` only checks ASCII alnum/-/_, which digits
    /// satisfy — but reads back as a non-string mapping key, which `load`'s
    /// `parse_stacks` then refuses. `write_verified` must catch that at write
    /// time (via `validate_config_text` running the same parse), not leave a
    /// file behind that only fails on the next launch.
    #[test]
    fn write_verified_refuses_a_digits_only_stack_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("workers-dev.yaml");
        std::fs::write(&path, "color: auto\n").unwrap();

        let err = write_verified(&path, "stacks:\n  123:\n    - console\n").unwrap_err();
        assert!(!err.to_string().is_empty());
        // The original file is untouched — byte-identical.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "color: auto\n");
    }
}
