//! Windowed streaming read primitives shared between single-path and batch
//! modes of `coder::read-file`.
//!
//! Extracted from `read_file.rs` (T8 size-pressure extraction) so
//! `read_file.rs` stays focused on the two handler modes.
//!
//! Two budget units live here, deliberately:
//! - [`read_window`] (single-path mode) budgets RAW file bytes — the T7
//!   contract for `max_read_bytes`.
//! - [`read_window_wire`] (batch mode) budgets CONVERTED WIRE BYTES —
//!   `content.len()` after lossy UTF-8 sanitization — because the batch
//!   budget exists to bound what the caller's context actually receives,
//!   and U+FFFD expansion (1 invalid byte → 3 wire bytes) would otherwise
//!   let binary files deliver up to 3x the configured budget.

use std::io::BufRead;

/// `N→` prefix for line `n` when numbering is on; empty when off. The
/// prefix is injected AT COLLECTION TIME so its bytes are charged against
/// the same budget as the line itself — numbering can never smuggle bytes
/// past a cap. (`→` is U+2192: 3 UTF-8 bytes.)
fn line_prefix(numbered: bool, n: u64) -> String {
    if numbered {
        format!("{n}\u{2192}")
    } else {
        String::new()
    }
}

/// Prefix every line of an already-converted body with its 1-based line
/// number (`N→`), numbering from `start`. Lines follow the shared
/// convention (0x0A- or EOF-terminated segments; empty input has none),
/// so numbering here matches `count_lines` and `coder::update-file`'s
/// line ops exactly. Used by the full-read path, where the whole body is
/// materialized before numbering.
pub fn number_lines(content: &str, start: u64) -> String {
    let mut out = String::with_capacity(content.len() + content.len() / 4);
    let mut n = start;
    for segment in content.split_inclusive('\n') {
        out.push_str(&line_prefix(true, n));
        out.push_str(segment);
        n += 1;
    }
    out
}

/// Outcome of the shared skip phase: either the stream reached the window
/// start, or EOF arrived first (in which case the file's full line count
/// is known for free — agents probe past EOF; not an error).
enum SkipOutcome {
    /// Lines `1..from` consumed; collection may begin. Carries the number
    /// of lines consumed so far (`from - 1`).
    Reached { consumed: u64 },
    /// EOF before the window starts; `total` is the file's line count.
    Eof { total: u64 },
}

/// Skip lines `1..from`, chunk-wise, buffering nothing: consume buffer
/// chunks counting 0x0A bytes. Shared by [`read_window`] and
/// [`read_window_wire`] so the two budget flavors can never drift in how
/// they locate the window start.
fn skip_to_window<R: BufRead>(reader: &mut R, from: u64) -> std::io::Result<SkipOutcome> {
    let mut consumed: u64 = 0;
    let mut in_partial_line = false;
    while consumed + 1 < from {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(SkipOutcome::Eof {
                total: consumed + u64::from(in_partial_line),
            });
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                reader.consume(idx + 1);
                consumed += 1;
                in_partial_line = false;
            }
            None => {
                let len = available.len();
                reader.consume(len);
                in_partial_line = true;
            }
        }
    }
    Ok(SkipOutcome::Reached { consumed })
}

/// Outcome of a streamed window read. `raw` holds the window's exact
/// bytes; lossy UTF-8 conversion happens ONCE on the whole collected
/// chunk, AFTER raw 0x0A line splitting, so an invalid multi-byte
/// sequence can never corrupt the line structure.
pub struct Window {
    pub raw: Vec<u8>,
    pub lines_returned: u64,
    pub total_lines: Option<u64>,
    pub more_lines: bool,
}

/// Stream lines `from..=to` (`to: None` = to EOF) out of `reader`
/// without ever materializing the full body:
///
/// - skip phase: consume buffer chunks counting 0x0A bytes, buffering
///   nothing;
/// - collect phase: buffer one line at a time, stopping when adding a
///   line would push the collected window over `max_bytes` — a partial
///   window is a SUCCESS with `more_lines: true`. A line that does not
///   fit is excluded entirely (the window never returns a torn line);
///   when even the first window line exceeds the budget, the window is
///   empty with `more_lines: true`. Per-line buffering is itself capped
///   at the remaining budget + 1 byte, so peak memory stays bounded by
///   ~2x `max_bytes` regardless of line length.
///
/// `total_lines` is reported only when the stream naturally reached EOF
/// (skip phase past EOF, collect phase hitting EOF, or an exact-`to`
/// window whose post-window peek shows EOF) — never via a forced scan.
///
/// With `numbered: true` each collected line is prefixed `N→` (N = the
/// line's absolute 1-based number in the FILE — `consumed + 1`, so a
/// window starting at line 40 is numbered from 40). Prefix bytes count
/// toward `max_bytes`: a line is excluded when prefix + raw line would
/// exceed the remaining budget.
pub fn read_window<R: BufRead>(
    reader: &mut R,
    from: u64,
    to: Option<u64>,
    max_bytes: u64,
    numbered: bool,
) -> std::io::Result<Window> {
    // Lines fully consumed from the stream so far (skipped + collected);
    // the next line to read is `consumed + 1`.
    let mut consumed = match skip_to_window(reader, from)? {
        SkipOutcome::Eof { total } => {
            return Ok(Window {
                raw: Vec::new(),
                lines_returned: 0,
                total_lines: Some(total),
                more_lines: false,
            })
        }
        SkipOutcome::Reached { consumed } => consumed,
    };

    // --- collect phase: lines from..=to (or EOF / byte budget) ----------
    let mut raw: Vec<u8> = Vec::new();
    let mut lines_returned: u64 = 0;
    let mut line_buf: Vec<u8> = Vec::new();
    loop {
        if to.is_some_and(|t| consumed >= t) {
            // Window complete. Peek (without consuming) to learn whether
            // anything follows — EOF here means the whole file was
            // traversed and the total line count is known for free.
            let at_eof = reader.fill_buf()?.is_empty();
            return Ok(Window {
                raw,
                lines_returned,
                total_lines: at_eof.then_some(consumed),
                more_lines: !at_eof,
            });
        }
        let budget_left = max_bytes.saturating_sub(raw.len() as u64);
        line_buf.clear();
        // Cap the line read at budget+1 bytes: enough to tell "fits"
        // from "does not fit" without buffering an arbitrarily long line.
        // UFCS pins `Self = &mut R` so the reader is reborrowed (not
        // moved) into the `Take` adapter.
        let n = std::io::Read::take(&mut *reader, budget_left.saturating_add(1))
            .read_until(b'\n', &mut line_buf)? as u64;
        if n == 0 {
            // Natural EOF at/under the byte cap: total known.
            return Ok(Window {
                raw,
                lines_returned,
                total_lines: Some(consumed),
                more_lines: false,
            });
        }
        // The `N→` prefix is charged against the SAME budget as the line:
        // a line that fits raw but not prefixed is excluded entirely.
        let prefix = line_prefix(numbered, consumed + 1);
        if n.saturating_add(prefix.len() as u64) > budget_left {
            // Byte budget exhausted mid-window: partial window, success.
            return Ok(Window {
                raw,
                lines_returned,
                total_lines: None,
                more_lines: true,
            });
        }
        raw.extend_from_slice(prefix.as_bytes());
        raw.extend_from_slice(&line_buf);
        lines_returned += 1;
        consumed += 1;
        if line_buf.last() != Some(&b'\n') {
            // EOF-terminated final segment: it IS a line (convention),
            // and the stream is exhausted.
            return Ok(Window {
                raw,
                lines_returned,
                total_lines: Some(consumed),
                more_lines: false,
            });
        }
    }
}

/// Outcome of a wire-budgeted window read (batch mode). The budget unit
/// is CONVERTED WIRE BYTES — `content.len()` after lossy UTF-8
/// sanitization — not raw file bytes. Each line is converted as it is
/// collected and counted in converted form, so binary input (whose
/// invalid bytes expand to 3-byte U+FFFD replacements) can never deliver
/// more than the budget. Raw 0x0A line splitting still happens BEFORE
/// conversion — 0x0A can never appear inside a multi-byte UTF-8
/// sequence, so per-line conversion concatenates to exactly the string a
/// whole-body conversion would produce.
pub struct WireWindow {
    pub content: String,
    pub is_utf8: bool,
    pub lines_returned: u64,
    pub total_lines: Option<u64>,
    pub more_lines: bool,
}

/// [`read_window`], but budgeted in CONVERTED wire bytes (see
/// [`WireWindow`]). Same skip phase and the same no-torn-lines rule,
/// applied to each line's CONVERTED form: a line whose converted length
/// exceeds the remaining budget is excluded entirely (`more_lines:
/// true`); when even the first line's converted form exceeds the budget,
/// the window is empty with `more_lines: true`.
///
/// Memory bound: lossy conversion never SHRINKS a byte sequence (valid
/// bytes map 1:1; each invalid maximal subpart of 1-3 bytes becomes one
/// 3-byte U+FFFD), so capping the RAW per-line read at remaining+1 bytes
/// both bounds peak memory (~2x budget) and detects "cannot fit" early:
/// raw overflow already implies converted overflow.
///
/// With `numbered: true` each line is prefixed `N→` (absolute 1-based
/// file line number); the prefix's bytes are charged against
/// `max_wire_bytes` together with the line's CONVERTED form.
pub fn read_window_wire<R: BufRead>(
    reader: &mut R,
    from: u64,
    to: Option<u64>,
    max_wire_bytes: u64,
    numbered: bool,
) -> std::io::Result<WireWindow> {
    let mut consumed = match skip_to_window(reader, from)? {
        SkipOutcome::Eof { total } => {
            return Ok(WireWindow {
                content: String::new(),
                is_utf8: true,
                lines_returned: 0,
                total_lines: Some(total),
                more_lines: false,
            })
        }
        SkipOutcome::Reached { consumed } => consumed,
    };

    let mut content = String::new();
    let mut is_utf8 = true;
    let mut lines_returned: u64 = 0;
    let mut line_buf: Vec<u8> = Vec::new();
    loop {
        if to.is_some_and(|t| consumed >= t) {
            let at_eof = reader.fill_buf()?.is_empty();
            return Ok(WireWindow {
                content,
                is_utf8,
                lines_returned,
                total_lines: at_eof.then_some(consumed),
                more_lines: !at_eof,
            });
        }
        let budget_left = max_wire_bytes.saturating_sub(content.len() as u64);
        line_buf.clear();
        let n = std::io::Read::take(&mut *reader, budget_left.saturating_add(1))
            .read_until(b'\n', &mut line_buf)? as u64;
        if n == 0 {
            // Natural EOF at/under the wire cap: total known.
            return Ok(WireWindow {
                content,
                is_utf8,
                lines_returned,
                total_lines: Some(consumed),
                more_lines: false,
            });
        }
        if n > budget_left {
            // The RAW form already exceeds the remaining wire budget;
            // the converted form can only be larger. Excluded entirely.
            return Ok(WireWindow {
                content,
                is_utf8,
                lines_returned,
                total_lines: None,
                more_lines: true,
            });
        }
        // Raw fits — convert this line and apply the fit test to the
        // CONVERTED length (the unit the budget is defined in), plus the
        // `N→` prefix when numbering: prefix bytes are budget bytes too.
        let ends_with_newline = line_buf.last() == Some(&b'\n');
        // mem::take re-allocates line_buf each iteration — intentional:
        // lossy_utf8 wants ownership, and the loop is budget-bounded, not hot.
        let (line, line_utf8) = lossy_utf8(std::mem::take(&mut line_buf));
        let prefix = line_prefix(numbered, consumed + 1);
        if (line.len() as u64).saturating_add(prefix.len() as u64) > budget_left {
            return Ok(WireWindow {
                content,
                is_utf8,
                lines_returned,
                total_lines: None,
                more_lines: true,
            });
        }
        content.push_str(&prefix);
        content.push_str(&line);
        is_utf8 &= line_utf8;
        lines_returned += 1;
        consumed += 1;
        if !ends_with_newline {
            // EOF-terminated final segment: it IS a line (convention),
            // and the stream is exhausted.
            return Ok(WireWindow {
                content,
                is_utf8,
                lines_returned,
                total_lines: Some(consumed),
                more_lines: false,
            });
        }
    }
}

/// Count lines per the shared convention (see `read_file` module docs):
/// 0x0A- or EOF-terminated segments; empty input has 0 lines; a trailing
/// newline does NOT add a phantom line. Matches `str::lines()` counting,
/// which `coder::update-file` uses for its 1-based line ops.
pub fn count_lines(bytes: &[u8]) -> u64 {
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    match bytes.last() {
        None => 0,
        Some(&b'\n') => newlines,
        Some(_) => newlines + 1,
    }
}

/// UTF-8 conversion with the documented lossy semantics: valid input
/// passes through unchanged (`true`); invalid bytes become U+FFFD
/// (`false`).
pub fn lossy_utf8(bytes: Vec<u8>) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(s) => (s, true),
        Err(e) => {
            let bytes = e.into_bytes();
            (String::from_utf8_lossy(&bytes).into_owned(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `count_lines` must match `str::lines()` counting — the same
    /// convention `coder::update-file` uses for its 1-based line ops.
    #[test]
    fn count_lines_matches_update_file_convention() {
        let cases: &[&[u8]] = &[
            b"",
            b"\n",
            b"a",
            b"a\n",
            b"a\nb",
            b"a\nb\n",
            b"a\n\n\nb\n",
            b"\r\n",
            b"a\r\nb\r\n",
        ];
        for bytes in cases {
            let via_str_lines = String::from_utf8_lossy(bytes).lines().count() as u64;
            assert_eq!(
                count_lines(bytes),
                via_str_lines,
                "convention drift for {bytes:?}"
            );
        }
    }

    #[test]
    fn lossy_utf8_valid_is_true() {
        let (s, ok) = lossy_utf8(b"hello".to_vec());
        assert_eq!(s, "hello");
        assert!(ok);
    }

    #[test]
    fn lossy_utf8_invalid_is_false_with_replacement() {
        let (s, ok) = lossy_utf8(vec![0xFF, 0xFE]);
        assert!(!ok);
        assert!(s.contains('\u{FFFD}'));
    }

    #[test]
    fn read_window_full_file() {
        let data = b"line1\nline2\nline3\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window(&mut reader, 1, None, 1024, false).unwrap();
        assert_eq!(w.lines_returned, 3);
        assert_eq!(w.total_lines, Some(3));
        assert!(!w.more_lines);
        assert_eq!(w.raw, data);
    }

    #[test]
    fn read_window_subset() {
        let data = b"L1\nL2\nL3\nL4\nL5\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window(&mut reader, 2, Some(4), 1024, false).unwrap();
        assert_eq!(w.lines_returned, 3);
        assert_eq!(&w.raw, b"L2\nL3\nL4\n");
        assert!(w.more_lines);
        assert_eq!(w.total_lines, None);
    }

    #[test]
    fn read_window_budget_cuts_partial() {
        // Lines are 5 bytes each; budget=10 → only 2 lines fit.
        let data = b"aaaa\nbbbb\ncccc\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window(&mut reader, 1, None, 10, false).unwrap();
        assert_eq!(w.lines_returned, 2);
        assert!(w.more_lines);
        assert_eq!(w.total_lines, None);
    }

    #[test]
    fn read_window_past_eof_returns_empty_with_total() {
        let data = b"L1\nL2\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window(&mut reader, 10, Some(20), 1024, false).unwrap();
        assert_eq!(w.lines_returned, 0);
        assert!(!w.more_lines);
        assert_eq!(w.total_lines, Some(2));
    }

    // -----------------------------------------------------------------
    // read_window_wire — converted-wire-byte budget
    // -----------------------------------------------------------------

    #[test]
    fn wire_budget_counts_converted_not_raw_bytes() {
        // Two raw lines of 3x0xFF + '\n' (4 raw bytes each); each converts
        // to 3xU+FFFD + '\n' = 10 wire bytes. Wire budget 10: exactly one
        // converted line fits even though BOTH raw lines (8 bytes) would
        // have fit a raw budget of 10.
        let data: &[u8] = b"\xFF\xFF\xFF\n\xFF\xFF\xFF\n";
        let mut reader = std::io::BufReader::new(data);
        let w = read_window_wire(&mut reader, 1, None, 10, false).unwrap();
        assert_eq!(w.lines_returned, 1);
        assert_eq!(w.content.len(), 10, "delivered wire bytes == budget");
        assert_eq!(w.content, "\u{FFFD}\u{FFFD}\u{FFFD}\n");
        assert!(!w.is_utf8);
        assert!(w.more_lines);
        assert_eq!(w.total_lines, None);
    }

    #[test]
    fn wire_budget_first_converted_line_over_budget_is_empty_partial() {
        // 10 raw 0xFF bytes = one EOF-terminated line converting to 30
        // wire bytes. Budget 10: excluded entirely per no-torn-lines on
        // the CONVERTED form → empty success, more_lines=true. (The raw
        // accounting bug delivered all 30 wire bytes here.)
        let data = [0xFFu8; 10];
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window_wire(&mut reader, 1, None, 10, false).unwrap();
        assert_eq!(w.content, "");
        assert_eq!(w.lines_returned, 0);
        assert!(w.more_lines);
        assert_eq!(w.total_lines, None);
    }

    #[test]
    fn wire_budget_matches_raw_for_ascii() {
        // For valid UTF-8 the two budget units coincide: identical output.
        let data = b"aaaa\nbbbb\ncccc\n";
        let mut r1 = std::io::BufReader::new(&data[..]);
        let raw = read_window(&mut r1, 1, None, 10, false).unwrap();
        let mut r2 = std::io::BufReader::new(&data[..]);
        let wire = read_window_wire(&mut r2, 1, None, 10, false).unwrap();
        assert_eq!(wire.content.as_bytes(), &raw.raw[..]);
        assert_eq!(wire.lines_returned, raw.lines_returned);
        assert_eq!(wire.total_lines, raw.total_lines);
        assert_eq!(wire.more_lines, raw.more_lines);
        assert!(wire.is_utf8);
    }

    #[test]
    fn wire_window_past_eof_returns_empty_with_total() {
        let data = b"L1\nL2\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window_wire(&mut reader, 10, Some(20), 1024, false).unwrap();
        assert_eq!(w.lines_returned, 0);
        assert!(!w.more_lines);
        assert_eq!(w.total_lines, Some(2));
        assert!(w.is_utf8, "empty window is vacuously clean");
    }

    // -----------------------------------------------------------------
    // numbered — `N→` prefixes at collection time
    // -----------------------------------------------------------------

    #[test]
    fn number_lines_prefixes_each_segment_from_start() {
        assert_eq!(
            number_lines("a\nb\nc\n", 1),
            "1\u{2192}a\n2\u{2192}b\n3\u{2192}c\n"
        );
        // EOF-terminated final segment is a line too.
        assert_eq!(number_lines("a\nb", 5), "5\u{2192}a\n6\u{2192}b");
        // Empty body has zero lines — nothing to number.
        assert_eq!(number_lines("", 1), "");
    }

    #[test]
    fn numbered_window_uses_absolute_file_line_numbers() {
        let data = b"L1\nL2\nL3\nL4\nL5\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window(&mut reader, 3, Some(4), 1024, true).unwrap();
        assert_eq!(&w.raw, "3\u{2192}L3\n4\u{2192}L4\n".as_bytes());
        assert_eq!(w.lines_returned, 2);
    }

    #[test]
    fn numbered_prefix_counts_toward_raw_budget() {
        // Each raw line is 5 bytes; the "1→"/"2→" prefix adds 4 bytes
        // (digit + 3-byte arrow) → 9 per numbered line. Budget 10:
        // unnumbered fits 2 lines, numbered fits only 1.
        let data = b"aaaa\nbbbb\ncccc\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window(&mut reader, 1, None, 10, true).unwrap();
        assert_eq!(&w.raw, "1\u{2192}aaaa\n".as_bytes());
        assert_eq!(w.lines_returned, 1);
        assert!(w.more_lines, "prefix bytes must not bypass the cap");
    }

    #[test]
    fn numbered_wire_prefix_counts_toward_wire_budget() {
        let data = b"aaaa\nbbbb\ncccc\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let w = read_window_wire(&mut reader, 1, None, 10, true).unwrap();
        assert_eq!(w.content, "1\u{2192}aaaa\n");
        assert_eq!(w.content.len(), 9);
        assert_eq!(w.lines_returned, 1);
        assert!(w.more_lines);
    }

    #[test]
    fn numbered_wire_prefixes_converted_lossy_lines() {
        // Invalid bytes convert to U+FFFD; the prefix rides on the
        // CONVERTED line and the combined length is what the budget sees.
        let data: &[u8] = b"\xFF\xFF\n";
        let mut reader = std::io::BufReader::new(data);
        let w = read_window_wire(&mut reader, 1, None, 64, true).unwrap();
        assert_eq!(w.content, "1\u{2192}\u{FFFD}\u{FFFD}\n");
        assert!(!w.is_utf8);
        assert_eq!(w.total_lines, Some(1));
    }
}
