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
//! - `{ op: "replace", pattern: "...", replacement: "...", ignore_case?: bool,
//!   dot_matches_newline?: bool, expect_matches?: u64 }` — regex substitution
//!   on the full file body after line ops. `dot_matches_newline` lets `.`
//!   cross newlines; `expect_matches` fails the whole file (C210, nothing
//!   written) when the actual match count differs. Capture references in
//!   `replacement` are validated pre-write: a reference to a group the
//!   pattern does not define fails with C210 (write a literal `$` as `$$`).

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError, WireError};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_update_file_input")]
pub struct UpdateFileInput {
    pub files: Vec<UpdateFileSpec>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateFileSpec {
    /// Path relative to the primary allowed root, or an absolute path inside
    /// any allowed root. Call `coder::info` to see the allowed roots. Paths
    /// outside every allowed root are rejected — use the shell worker's
    /// `shell::fs::*` for host paths outside the jail.
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
        /// Substitution text for each match. Capture references expand:
        /// `$1`/`${1}` by index (`$0` is the whole match) and
        /// `$name`/`${name}` by name. A literal `$` MUST be written `$$`
        /// — JS/TS template literals in a replacement are the classic
        /// collision: write `Hello, $${name}!` to output `Hello, ${name}!`.
        /// Unbraced references consume the longest `[0-9A-Za-z_]` run,
        /// so `$1a` means a group named "1a", NOT group 1 then "a" (use
        /// `${1}a`). A reference to a group the pattern does not define
        /// fails pre-write with C210 — nothing is written. References
        /// are validated even when the replacement goes unused (e.g.
        /// `expect_matches: 0`): the replacement must be well-formed
        /// even when unused.
        replacement: String,
        #[serde(default)]
        ignore_case: bool,
        /// When true, `.` in `pattern` also matches `\n`, so a short
        /// anchored pattern like `"fn parse_config\\(.*?\\n\\}"` replaces a
        /// whole multi-line region without quoting it — prefer two short
        /// anchors joined by `.*?` over pasting the block into the pattern.
        /// Without this flag (the default), `.` does not cross newlines and
        /// a multi-line pattern silently matches nothing.
        #[serde(default)]
        dot_matches_newline: bool,
        /// Expected number of matches for this op. When set and the actual
        /// count differs, this FILE fails with C210 and nothing is written
        /// to it (other files in the batch still apply). Set
        /// `expect_matches: 1` to make a targeted read-free edit safe — a
        /// mismatch means the pattern is anchored too loosely or matches
        /// nothing. Set `expect_matches: 0` to assert ABSENCE: the op
        /// succeeds only when nothing matches (the replacement is unused).
        /// Omit to replace all matches unconditionally.
        #[serde(default)]
        expect_matches: Option<u64>,
    },
}

// examples are wire-contract; goldens pin them.
fn example_update_file_input() -> serde_json::Value {
    serde_json::json!({
        "files": [{
            "path": "src/lib.rs",
            "ops": [
                { "op": "insert", "at_line": 1, "content": "// generated by coder\n" },
                { "op": "update_lines", "from_line": 5, "to_line": 7,
                  "content": "pub fn hello() {\n    println!(\"hello\");\n}\n" },
                { "op": "replace", "pattern": "// BEGIN legacy.*?// END legacy",
                  "replacement": "// removed", "dot_matches_newline": true,
                  "expect_matches": 1 }
            ]
        }]
    })
}

/// Post-apply snapshot of the region affected by one op. Line ops echo the
/// affected region with ±2 context lines; regex `replace` ops emit one echo
/// per match site (up to 5, no context): the FIRST and LAST line of the
/// post-replace region, with `elided` set to the inner line count when the
/// region spans more than 2 lines (single-line replacements echo just that
/// line). Each site carries `total_replacements`. Provides just enough
/// context to verify the edit landed in the right place without flooding
/// the LLM context with the full file body.
#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct OpEcho {
    /// Index of the op in the request's ops array (0-based).
    pub op_index: u32,
    /// 1-based line number of the first echoed line (after all ops applied).
    pub from_line: u64,
    /// The echoed lines, post-apply. When the region is large, middle lines
    /// are elided and `elided` is set to indicate how many were skipped.
    pub lines: Vec<String>,
    /// Number of middle lines elided from a large region: for line ops,
    /// set when the affected region exceeded the per-echo cap; for
    /// replace sites, the count of inner lines between the region's
    /// echoed first and last line (set when the region spans >2 lines).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elided: Option<u64>,
    /// Total number of replacements the regex op made across the whole
    /// file (set only on replace-op site echoes, duplicated on each site).
    /// Sites are capped at 5 — when more matched, this count is the only
    /// record of the extras.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_replacements: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateFileOutput {
    pub results: Vec<UpdateFileResult>,
}

/// Maximum lines echoed per op before elision kicks in (first 8 + last 8).
const ECHO_MAX_LINES: usize = 20;
/// Number of context lines shown above and below a line op's region.
/// Regex match sites echo the first and last line of the post-replace
/// region (context 0, inner lines elided).
const ECHO_CONTEXT: i64 = 2;
/// Max echoed lines in the head/tail of a large region.
const ECHO_HEAD_TAIL: usize = 8;
/// Approximate per-file echo budget in bytes (~4 KiB).
const ECHO_BUDGET_BYTES: usize = 4 * 1024;
/// Maximum number of match sites echoed for a `replace` op.
const ECHO_MAX_SITES: usize = 5;

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateFileResult {
    /// Canonical absolute path (resolved through the jail); the caller's
    /// input verbatim when resolution failed.
    pub path: String,
    pub success: bool,
    /// Number of operations applied (only meaningful when `success`).
    pub applied: u32,
    /// Final line count after applying (only meaningful when `success`).
    pub new_line_count: u64,
    /// Per-op bounded post-apply echoes for edit verification; each applied
    /// op returns a snapshot of the affected region (±2 context lines) so
    /// the caller can confirm the edit landed at the right position without
    /// receiving the full file body. See `OpEcho` for field semantics.
    /// Empty on failure. Always present on the wire.
    pub echoes: Vec<OpEcho>,
    /// True when the total echo budget (~4 KiB) was exhausted before all
    /// op echoes could be emitted. Use `coder::read-file` to inspect the
    /// full result if needed. Always present on the wire.
    pub echoes_truncated: bool,
    /// Structured error for this entry. `code` is stable for programmatic
    /// branching (e.g. `"C211"` for not-found-or-denied; `"C210"` for bad
    /// input such as overlapping ops). `message` carries the corrective
    /// action an LLM agent needs to make a successful second call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
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
    let fs_scope = req.fs_scope.as_ref();
    let mut entries = Vec::with_capacity(req.files.len());
    for spec in req.files {
        match resolver.require_writable_scope(fs_scope, &spec.path) {
            Ok(abs) => entries.push((spec, Ok(abs))),
            Err(e) if is_jail_scope_error(&e) => return Err(err_to_string(e)),
            Err(e) => entries.push((spec, Err(e))),
        }
    }
    let results = entries
        .into_iter()
        .map(|(spec, resolved)| update_one(&cfg, spec, resolved))
        .collect();
    Ok(UpdateFileOutput { results })
}

fn update_one(
    cfg: &CoderConfig,
    spec: UpdateFileSpec,
    resolved: Result<std::path::PathBuf, CoderError>,
) -> UpdateFileResult {
    // Resolve up front: the edit pipeline operates ONLY on the
    // resolver-returned path, and the result echoes that canonical
    // absolute path. When resolution fails there is no canonical path,
    // so the caller's input is echoed verbatim.
    let abs = match resolved {
        Ok(abs) => abs,
        Err(e) => {
            return UpdateFileResult {
                path: spec.path,
                success: false,
                applied: 0,
                new_line_count: 0,
                echoes: vec![],
                echoes_truncated: false,
                error: Some((&e).into()),
            }
        }
    };
    let wire_path = abs.display().to_string();
    match try_update_one(cfg, &abs, spec) {
        Ok((applied, new_line_count, echoes, echoes_truncated)) => UpdateFileResult {
            path: wire_path,
            success: true,
            applied,
            new_line_count,
            echoes,
            echoes_truncated,
            error: None,
        },
        Err(e) => UpdateFileResult {
            path: wire_path,
            success: false,
            applied: 0,
            new_line_count: 0,
            echoes: vec![],
            echoes_truncated: false,
            error: Some((&e).into()),
        },
    }
}

fn is_jail_scope_error(e: &CoderError) -> bool {
    matches!(
        e,
        CoderError::OutsideBase(_) | CoderError::OutsideSession(_)
    )
}

fn try_update_one(
    cfg: &CoderConfig,
    abs: &Path,
    spec: UpdateFileSpec,
) -> Result<(u32, u64, Vec<OpEcho>, bool), CoderError> {
    // NotFound is intercepted with the wire path in scope so the C211
    // message names the path the caller supplied (standardized wording —
    // REDACTION INVARIANT: identical to the glob-denied message).
    let md = std::fs::metadata(abs).map_err(|e| CoderError::io_for_path(e, &spec.path))?;
    if !md.is_file() {
        return Err(CoderError::BadInput(format!(
            "not a regular file: {}",
            spec.path
        )));
    }
    if md.len() > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "{} current file size is {} bytes, which exceeds max_write_bytes \
             ({}). Split the content or raise max_write_bytes in coder config.",
            spec.path,
            md.len(),
            cfg.max_write_bytes
        )));
    }
    if spec.ops.is_empty() {
        return Err(CoderError::BadInput("ops must not be empty".into()));
    }

    let bytes = std::fs::read(abs).map_err(|e| CoderError::io_for_path(e, &spec.path))?;
    let (mut lines, ending, has_trailing) = split_file(&bytes);
    let original_len = lines.len();

    let line_ops: Vec<&UpdateOp> = spec.ops.iter().filter(|op| is_line_op(op)).collect();

    validate_line_ops(&line_ops, original_len)?;
    apply_line_ops(&mut lines, &line_ops)?;

    // Mutation-event timeline: every line-count-changing mutation is
    // recorded in true application order so echo anchors can be mapped
    // to FINAL-body coordinates (see `MutationEvent`/`map_through_events`).
    let mut events: Vec<MutationEvent> = Vec::new();
    let mut anchors: Vec<EchoAnchor> = Vec::new();
    record_line_op_events(&spec.ops, &mut events, &mut anchors);

    let mut new_bytes = join_lines(&lines, ending, has_trailing);
    new_bytes = apply_regex_ops(new_bytes, &spec.ops, &mut events, &mut anchors)?;

    let (final_lines, _, _) = split_file(&new_bytes);
    if (new_bytes.len() as u64) > cfg.max_write_bytes {
        return Err(CoderError::TooLarge(format!(
            "{} new file size after ops is {} bytes, which exceeds \
             max_write_bytes ({}). Split the content or raise \
             max_write_bytes in coder config.",
            spec.path,
            new_bytes.len(),
            cfg.max_write_bytes
        )));
    }
    atomic_write(abs, &new_bytes)?;

    // Build per-op echoes by mapping each anchor through the events that
    // applied after it, then extracting from the FINAL body.
    let (echoes, echoes_truncated) = build_echoes(&anchors, &events, &final_lines);

    Ok((
        spec.ops.len() as u32,
        final_lines.len() as u64,
        echoes,
        echoes_truncated,
    ))
}

// ---------------------------------------------------------------------------
// Echo construction — mutation-event timeline
//
// Every line-count-changing mutation is recorded as an event in TRUE
// application order: line ops bottom-up first (exactly as `apply_line_ops`
// applies them), then each regex op's replacements sequentially in match
// order. Echo anchors captured at event k map to the final body by
// shifting through every later event (`map_through_events`); content is
// always extracted from the FINAL body, clamped so a coordinate bug can
// only produce a bounded wrong echo, never a panic.
// ---------------------------------------------------------------------------

/// One newline-count-changing mutation in true application order. `pos` is
/// the 1-based line of the mutation in ITS OWN input body (the body after
/// all earlier events applied); `delta` is the line-count change it caused.
/// Zero-delta mutations are not recorded — they cannot shift any position.
#[derive(Debug, Clone, Copy)]
struct MutationEvent {
    pos: i64,
    delta: i64,
}

/// A pending echo region captured when its mutation applied, expressed in
/// the body as it existed right after `events[..events_after]` ran. It is
/// mapped to the final body by shifting through every later event.
#[derive(Debug)]
struct EchoAnchor {
    op_index: u32,
    /// Inclusive 1-based region in the post-mutation body at capture time.
    first: i64,
    last: i64,
    /// `events.len()` at capture time: only later events shift this anchor.
    events_after: usize,
    /// Context lines around the region (±2 for line ops, 0 for regex sites).
    context: i64,
    /// Regex sites echo only the region's FIRST and LAST line (inner
    /// lines counted in `elided`); line ops echo the full region with
    /// head/tail elision.
    first_last_only: bool,
    /// Regex sites carry the op's total replacement count.
    total_replacements: Option<u64>,
}

/// Map a 1-based line position forward through `later` mutation events.
/// Every event at a position ≤ `pos` shifts it by the event's delta;
/// events strictly below never move content above them. Each event's
/// position is expressed in its own input frame — exactly the frame `pos`
/// occupies after shifting through all earlier events in the slice — so a
/// single left-to-right pass is correct.
fn map_through_events(pos: i64, later: &[MutationEvent]) -> i64 {
    let mut p = pos;
    for ev in later {
        if ev.pos <= p {
            p += ev.delta;
        }
    }
    p
}

fn count_newlines(bytes: &[u8]) -> i64 {
    bytes.iter().filter(|&&b| b == b'\n').count() as i64
}

/// Record one mutation event + echo anchor per line op, in true application
/// order (descending anchor — mirrors `apply_line_ops`). In each op's input
/// frame its region starts at its ORIGINAL anchor: ops already applied all
/// sit strictly above it (overlap-validated), so they never shift it.
fn record_line_op_events(
    ops: &[UpdateOp],
    events: &mut Vec<MutationEvent>,
    anchors: &mut Vec<EchoAnchor>,
) {
    let mut line_ops: Vec<(usize, &UpdateOp)> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| is_line_op(op))
        .collect();
    line_ops.sort_by_key(|b| std::cmp::Reverse(anchor(b.1)));

    for (op_index, op) in line_ops {
        let (pos, delta, first, last) = match op {
            UpdateOp::Insert { at_line, content } => {
                let m = split_content(content).len() as i64;
                let a = *at_line as i64;
                (a, m, a, a + m - 1)
            }
            UpdateOp::Remove { from_line, to_line } => {
                let a = *from_line as i64;
                let consumed = *to_line as i64 - a + 1;
                // The region is gone — echo the lines now surrounding the
                // removal point (context expands the window).
                (a, -consumed, a - 1, a - 1)
            }
            UpdateOp::UpdateLines {
                from_line,
                to_line,
                content,
            } => {
                let m = split_content(content).len() as i64;
                let a = *from_line as i64;
                let consumed = *to_line as i64 - a + 1;
                (a, m - consumed, a, a + m - 1)
            }
            UpdateOp::Replace { .. } => unreachable!("line ops only"),
        };
        if delta != 0 {
            events.push(MutationEvent { pos, delta });
        }
        anchors.push(EchoAnchor {
            op_index: op_index as u32,
            first,
            last,
            events_after: events.len(),
            context: ECHO_CONTEXT,
            first_last_only: false,
            total_replacements: None,
        });
    }
}

/// Apply every `replace` op in `ops` (in array order), each on the body
/// produced by all earlier ops, recording mutation events and up to
/// `ECHO_MAX_SITES` echo anchors per op. Match positions are located on
/// each op's OWN input body; earlier replacements within the same op shift
/// later sites through the event list like any other mutation.
fn apply_regex_ops(
    mut bytes: Vec<u8>,
    ops: &[UpdateOp],
    events: &mut Vec<MutationEvent>,
    anchors: &mut Vec<EchoAnchor>,
) -> Result<Vec<u8>, CoderError> {
    for (op_index, op) in ops.iter().enumerate() {
        let UpdateOp::Replace {
            pattern,
            replacement,
            ignore_case,
            dot_matches_newline,
            expect_matches,
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
        builder.dot_matches_new_line(*dot_matches_newline);
        let re = builder
            .build()
            .map_err(|e| CoderError::BadInput(format!("bad regex {pattern:?}: {e}")))?;
        // R1 (v0.4.1): pre-write guard — every `$` capture reference in
        // the replacement must name a group the pattern actually defines.
        // Runs right after compilation, before ANY replacement work, so
        // an undefined reference fails the entry (C210) with the file
        // byte-identical on disk.
        validate_replacement_refs(&re, pattern, replacement)?;

        let op_events_start = events.len();
        // (site first/last line in its own frame, events.len() at
        // capture); anchors are created after the op's total count is
        // known. `last` = first + newline count of the expanded
        // replacement: the full post-replace region of this site.
        let mut sites: Vec<(i64, i64, usize)> = Vec::new();
        let mut total: u64 = 0;
        // Incremental input-frame line counter for ascending match starts.
        let mut scan_pos: usize = 0;
        let mut scan_line: i64 = 0;

        let s = String::from_utf8_lossy(&bytes);
        let out = re
            .replace_all(&s, |caps: &regex::Captures<'_>| {
                let m = caps.get(0).expect("regex group 0 always present");
                scan_line += count_newlines(s[scan_pos..m.start()].as_bytes());
                scan_pos = m.start();
                let mut expanded = String::new();
                caps.expand(replacement, &mut expanded);
                let expanded_newlines = count_newlines(expanded.as_bytes());
                let delta = expanded_newlines - count_newlines(m.as_str().as_bytes());
                // Input-frame line -> this match's own frame (earlier
                // matches of this op may already have shifted it).
                // O(k) per match within this op (k = earlier delta-events);
                // O(M^2) total for M newline-changing matches. Acceptable:
                // max_write_bytes caps input and most replaces are delta-0.
                let pos = map_through_events(scan_line + 1, &events[op_events_start..]);
                if delta != 0 {
                    events.push(MutationEvent { pos, delta });
                }
                if sites.len() < ECHO_MAX_SITES {
                    sites.push((pos, pos + expanded_newlines, events.len()));
                }
                total += 1;
                expanded
            })
            .into_owned();
        // expect_matches guard: validated BEFORE any filesystem mutation —
        // `try_update_one` only reaches `atomic_write` after every op in
        // the file's pipeline succeeded, so erroring here leaves the file
        // byte-identical on disk (earlier ops' changes are in-memory only)
        // and the per-entry failure carries no echoes.
        // HAZARD: the failing op's MutationEvents are already pushed into
        // `events` — safety hinges on the WHOLE frame being discarded on
        // Err; a future resumable/per-op refactor must rewind them.
        if let Some(expected) = expect_matches {
            if total != *expected {
                return Err(expect_matches_mismatch(pattern, total, *expected));
            }
        }
        bytes = out.into_bytes();
        for (first, last, events_after) in sites {
            anchors.push(EchoAnchor {
                op_index: op_index as u32,
                first,
                last,
                events_after,
                context: 0,
                first_last_only: true,
                total_replacements: Some(total),
            });
        }
    }
    Ok(bytes)
}

/// Max pattern chars echoed in an `expect_matches` mismatch message —
/// enough to disambiguate WHICH replace op failed in a multi-op file
/// without flooding the error with a huge regex.
const PATTERN_SNIPPET_MAX_CHARS: usize = 60;

/// Quote a pattern for an error message, truncating to
/// [`PATTERN_SNIPPET_MAX_CHARS`] (char-boundary safe) with a `…` marker.
fn pattern_snippet(pattern: &str) -> String {
    if pattern.chars().count() <= PATTERN_SNIPPET_MAX_CHARS {
        format!("\"{pattern}\"")
    } else {
        let head: String = pattern.chars().take(PATTERN_SNIPPET_MAX_CHARS).collect();
        format!("\"{head}…\"")
    }
}

/// "1 time" / "N times" — mismatch messages read like prose, and the
/// assert-absent failure commonly reports exactly one match.
fn times(n: u64) -> String {
    if n == 1 {
        "1 time".to_string()
    } else {
        format!("{n} times")
    }
}

/// Prescriptive C210 for an `expect_matches` mismatch: names the failing
/// pattern (truncated) plus the actual and expected counts, and the
/// corrective next call. Nothing matched → re-check anchors with
/// `coder::search` / `dot_matches_newline`. Expected 0 (assert-absent)
/// but something matched → also route to `coder::search`; suggesting
/// `expect_matches: {actual}` here would invert the caller's
/// assert-absent intent. Otherwise → tighten anchors or accept the
/// observed count.
fn expect_matches_mismatch(pattern: &str, actual: u64, expected: u64) -> CoderError {
    let shown = pattern_snippet(pattern);
    if actual == 0 {
        CoderError::BadInput(format!(
            "replace pattern {shown} matched 0 times, expected {expected} — \
             re-check the anchors with coder::search (the pattern may need \
             dot_matches_newline: true or different anchor text)"
        ))
    } else if expected == 0 {
        CoderError::BadInput(format!(
            "replace pattern {shown} matched {}, expected 0 — \
             the pattern asserted absent still matches; re-check the \
             match sites with coder::search before replacing",
            times(actual)
        ))
    } else {
        CoderError::BadInput(format!(
            "replace pattern {shown} matched {}, expected {expected} — \
             anchor the regex more tightly (add surrounding context) or set \
             expect_matches: {actual}",
            times(actual)
        ))
    }
}

// ---------------------------------------------------------------------------
// R1 (v0.4.1) — pre-write validation of replacement capture references.
//
// The regex crate expands UNDEFINED `$` references to the EMPTY STRING
// (documented `Captures::expand` behavior). A replacement carrying
// literal `$…` text — JS/TS template literals being the production case
// (session q8x6g248: `Hello, ${name}!` silently became `Hello, !` with
// success: true) — therefore corrupts the file unless every reference is
// checked against the pattern's actual groups BEFORE any replacement
// work. The tokenizer below mirrors regex-automata's
// `util::interpolate::{string, find_cap_ref}` exactly; the parity test
// `r1_validator_matches_expand_semantics` pins both sides. A regex bump
// that breaks that test means interpolate semantics moved — re-mirror
// this tokenizer.
// ---------------------------------------------------------------------------

/// A capture reference parsed from a replacement string, plus the exact
/// text it was written as (`$1a`, `${name}`, …) for error messages.
struct ReplacementRef<'a> {
    /// The reference as written, including the leading `$`.
    written: &'a str,
    kind: RefKind<'a>,
}

enum RefKind<'a> {
    Index(usize),
    Named(&'a str),
}

/// Tokenize `replacement` EXACTLY like `Captures::expand`:
/// - `$$` is a literal-`$` escape (no reference);
/// - unbraced `$ref` consumes the longest run of `[0-9A-Za-z_]`;
/// - braced `${ref}` accepts ANY text up to the first `}` (`${}`
///   included — a reference to the empty name, which no pattern can
///   define);
/// - a `$` that cannot start a reference (`$ `, `$.`, trailing `$`,
///   unclosed `${`) is literal;
/// - refs that parse as `usize` are INDEX refs, everything else is
///   NAMED — so `$1a` names the group "1a", it is NOT `$1` then "a".
fn replacement_refs(replacement: &str) -> Vec<ReplacementRef<'_>> {
    let mut refs = Vec::new();
    let mut s = replacement;
    while let Some(i) = s.find('$') {
        s = &s[i..];
        if s.as_bytes().get(1) == Some(&b'$') {
            s = &s[2..]; // `$$` escape — literal `$`, no reference.
            continue;
        }
        match parse_cap_ref(s) {
            Some((kind, end)) => {
                refs.push(ReplacementRef {
                    written: &s[..end],
                    kind,
                });
                s = &s[end..];
            }
            None => s = &s[1..], // literal `$`
        }
    }
    refs
}

/// Parse one possible capture reference at the start of `s` (which
/// begins with `$`). Returns the reference and its end offset, or None
/// when the `$` cannot start a reference (it is then literal). Mirrors
/// regex-automata's `find_cap_ref`/`find_cap_ref_braced`.
fn parse_cap_ref(s: &str) -> Option<(RefKind<'_>, usize)> {
    let rep = s.as_bytes();
    if rep.len() <= 1 || rep[0] != b'$' {
        return None;
    }
    if rep[1] == b'{' {
        // Braced: anything up to the first `}`; unclosed brace → literal.
        let close = s[2..].find('}')? + 2;
        return Some((classify_cap_ref(&s[2..close]), close + 1));
    }
    let mut end = 1;
    while rep.get(end).copied().is_some_and(is_cap_letter) {
        end += 1;
    }
    if end == 1 {
        return None;
    }
    Some((classify_cap_ref(&s[1..end]), end))
}

/// usize-parseable refs are INDEX references; everything else is NAMED
/// (matching `find_cap_ref`'s `parse::<usize>()` fallback).
fn classify_cap_ref(name: &str) -> RefKind<'_> {
    match name.parse::<usize>() {
        Ok(i) => RefKind::Index(i),
        Err(_) => RefKind::Named(name),
    }
}

fn is_cap_letter(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_')
}

/// Validate every capture reference in `replacement` against the
/// compiled pattern's actual groups. First undefined reference →
/// prescriptive C210; nothing is written for the entry (the caller only
/// reaches `atomic_write` after every op succeeded).
fn validate_replacement_refs(
    re: &regex::Regex,
    pattern: &str,
    replacement: &str,
) -> Result<(), CoderError> {
    let group_count = re.captures_len(); // includes group 0 (whole match)
    for r in replacement_refs(replacement) {
        let defined = match r.kind {
            RefKind::Index(i) => i < group_count,
            RefKind::Named(name) => re.capture_names().flatten().any(|n| n == name),
        };
        if !defined {
            return Err(undefined_capture_ref(pattern, re, &r));
        }
    }
    Ok(())
}

/// Prescriptive C210 for an undefined capture reference: names the
/// offending reference as written, states what the pattern actually
/// defines, and gives the corrective rewrites (escape literal `$` as
/// `$$` — the JS/TS template-literal collision — or add the group).
fn undefined_capture_ref(pattern: &str, re: &regex::Regex, r: &ReplacementRef<'_>) -> CoderError {
    let shown = pattern_snippet(pattern);
    let explicit = re.captures_len() - 1;
    let named: Vec<String> = re
        .capture_names()
        .flatten()
        .map(|n| format!("`{n}`"))
        .collect();
    let plural = if explicit == 1 { "" } else { "s" };
    let defines = match (&r.kind, explicit) {
        (RefKind::Index(_), 0) => {
            "defines 0 capture groups (only $0, the whole match, is valid)".to_string()
        }
        (RefKind::Index(_), n) => {
            format!("defines {n} capture group{plural} (valid: $0, the whole match, through ${n})")
        }
        (RefKind::Named(name), 0) => {
            format!("defines 0 capture groups and no group named `{name}`")
        }
        (RefKind::Named(name), n) if named.is_empty() => {
            format!("defines {n} unnamed capture group{plural} and no group named `{name}`")
        }
        (RefKind::Named(name), n) => format!(
            "defines {n} capture group{plural} (named: {}) and no group named `{name}`",
            named.join(", ")
        ),
    };
    let written = r.written;
    CoderError::BadInput(format!(
        "replacement references capture group `{written}` but pattern {shown} \
         {defines} — the regex engine expands undefined references to the \
         EMPTY STRING, silently corrupting the file. Escape literal `$` as \
         `$$` (write `${written}` to output a literal `{written}` — common \
         when the replacement contains JS/TS template literals), or add the \
         capture group to the pattern"
    ))
}

/// Emit per-op echoes in op-index order (sites keep match order), mapping
/// each anchor to the final body and enforcing the per-file byte budget.
fn build_echoes(
    anchors: &[EchoAnchor],
    events: &[MutationEvent],
    final_lines: &[String],
) -> (Vec<OpEcho>, bool) {
    let mut order: Vec<usize> = (0..anchors.len()).collect();
    order.sort_by_key(|&i| anchors[i].op_index);

    let mut echoes: Vec<OpEcho> = Vec::new();
    let mut budget_bytes: usize = ECHO_BUDGET_BYTES;
    for &i in &order {
        let a = &anchors[i];
        let later = &events[a.events_after.min(events.len())..];
        let first = map_through_events(a.first, later);
        let last = map_through_events(a.last, later);
        let echo = if a.first_last_only {
            build_site_echo(a.op_index, first, last, final_lines, a.total_replacements)
        } else {
            build_line_echo(
                a.op_index,
                first,
                last,
                a.context,
                final_lines,
                a.total_replacements,
            )
        };
        if !append_echo(echo, &mut echoes, &mut budget_bytes) {
            return (echoes, true);
        }
    }
    (echoes, false)
}

/// Build a replace-site `OpEcho` for the post-replace region
/// `[post_first..=post_last]` (1-based): single- and two-line regions
/// echo every line; larger regions echo only the FIRST and LAST line
/// with `elided` set to the inner line count, so the tail of a
/// multi-line replacement is always visible without flooding the budget
/// (sites are capped at [`ECHO_MAX_SITES`] per op).
///
/// DEFENSIVE (panic guard): same clamping discipline as
/// [`build_line_echo`] — a wrong upstream coordinate degrades to a
/// bounded, possibly-off-position echo, never a panic.
fn build_site_echo(
    op_index: u32,
    post_first: i64,
    post_last: i64,
    final_lines: &[String],
    total_replacements: Option<u64>,
) -> OpEcho {
    let n = final_lines.len() as i64;
    if n == 0 {
        return OpEcho {
            op_index,
            from_line: 1,
            lines: vec![],
            elided: None,
            total_replacements,
        };
    }
    let start = post_first.saturating_sub(1).clamp(0, n - 1);
    let mut end = post_last.saturating_sub(1).clamp(0, n - 1);
    if end < start {
        // Inverted region (a collapse event pulled `last` above `first`):
        // degrade to a single-line echo, mirroring build_line_echo.
        end = start;
    }
    let (start, end) = (start as usize, end as usize);
    let region_len = end - start + 1; // end >= start by construction
    let from_line = (start + 1) as u64;
    if region_len <= 2 {
        OpEcho {
            op_index,
            from_line,
            lines: final_lines[start..=end].to_vec(),
            elided: None,
            total_replacements,
        }
    } else {
        OpEcho {
            op_index,
            from_line,
            lines: vec![final_lines[start].clone(), final_lines[end].clone()],
            elided: Some((region_len - 2) as u64),
            total_replacements,
        }
    }
}

/// Append an echo if it fits in the budget. Returns false when the budget
/// is exhausted (the first echo is always admitted).
fn append_echo(echo: OpEcho, echoes: &mut Vec<OpEcho>, budget_bytes: &mut usize) -> bool {
    let estimate = echo.lines.iter().map(|l| l.len() + 1).sum::<usize>() + 64; // overhead
    if *budget_bytes < estimate && !echoes.is_empty() {
        return false;
    }
    *budget_bytes = budget_bytes.saturating_sub(estimate);
    echoes.push(echo);
    true
}

/// Build an `OpEcho` for the post-apply region `[post_first..=post_last]`
/// (1-based), expanded by `context` lines on each side, eliding the middle
/// of large regions.
///
/// DEFENSIVE (panic guard): all coordinates are saturating/clamped to the
/// final body, so a wrong upstream coordinate degrades to a bounded,
/// possibly-off-position echo — never an arithmetic or slice panic.
fn build_line_echo(
    op_index: u32,
    post_first: i64,
    post_last: i64,
    context: i64,
    final_lines: &[String],
    total_replacements: Option<u64>,
) -> OpEcho {
    let n = final_lines.len() as i64;
    if n == 0 {
        return OpEcho {
            op_index,
            from_line: 1,
            lines: vec![],
            elided: None,
            total_replacements,
        };
    }

    let start = post_first
        .saturating_sub(1)
        .saturating_sub(context)
        .clamp(0, n - 1);
    let mut end = post_last
        .saturating_sub(1)
        .saturating_add(context)
        .clamp(0, n - 1);
    if end < start {
        // Inverted region (e.g. empty insert, or a collapse event pulled
        // `last` above `first`): degrade to a single-line echo.
        // Known cosmetic (ADV-1): the echoed line may be off by one
        // from the ideal collapse point; accepted as harmless.
        end = start;
    }
    let (start, end) = (start as usize, end as usize);
    let region_len = end - start + 1; // end >= start by construction
    let from_line = (start + 1) as u64;

    if region_len <= ECHO_MAX_LINES {
        OpEcho {
            op_index,
            from_line,
            lines: final_lines[start..=end].to_vec(),
            elided: None,
            total_replacements,
        }
    } else {
        // Large region: first ECHO_HEAD_TAIL + last ECHO_HEAD_TAIL lines.
        let mut lines = final_lines[start..start + ECHO_HEAD_TAIL].to_vec();
        let tail_start = (end + 1).saturating_sub(ECHO_HEAD_TAIL);
        lines.extend_from_slice(&final_lines[tail_start..=end]);
        OpEcho {
            op_index,
            from_line,
            lines,
            elided: Some((region_len - ECHO_HEAD_TAIL * 2) as u64),
            total_replacements,
        }
    }
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

// Keep the old name for the unit tests that exercise regex semantics
// without caring about echo traces.
#[cfg(test)]
fn apply_regex_replaces(bytes: Vec<u8>, ops: &[&UpdateOp]) -> Result<Vec<u8>, CoderError> {
    let owned: Vec<UpdateOp> = ops.iter().map(|op| (*op).clone()).collect();
    apply_regex_ops(bytes, &owned, &mut Vec::new(), &mut Vec::new())
}

/// Apply line ops only — used by unit tests that exercise line semantics
/// without the full file pipeline.
#[cfg(test)]
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
                dot_matches_newline: false,
                expect_matches: None,
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
                dot_matches_newline: false,
                expect_matches: None,
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
                dot_matches_newline: false,
                expect_matches: None,
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
                dot_matches_newline: false,
                expect_matches: None,
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
                dot_matches_newline: false,
                expect_matches: None,
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
                dot_matches_newline: false,
                expect_matches: None,
            }],
        )
        .unwrap();
        assert_eq!(out, b"hello");
    }

    // -----------------------------------------------------------------------
    // S3 (v0.4.0): dot_matches_newline + expect_matches regex semantics.
    // -----------------------------------------------------------------------

    #[test]
    fn regex_dot_does_not_cross_newlines_by_default() {
        let out = apply_regex_replaces(
            b"start\nmiddle\nend\n".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "start.*?end".into(),
                replacement: "X".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: None,
            }],
        )
        .unwrap();
        assert_eq!(out, b"start\nmiddle\nend\n");
    }

    #[test]
    fn regex_dot_matches_newline_crosses_lines() {
        let out = apply_regex_replaces(
            b"start\nmiddle\nend\n".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "start.*?end".into(),
                replacement: "X".into(),
                ignore_case: false,
                dot_matches_newline: true,
                expect_matches: None,
            }],
        )
        .unwrap();
        assert_eq!(out, b"X\n");
    }

    #[test]
    fn regex_expect_matches_exact_count_ok() {
        let out = apply_regex_replaces(
            b"foo bar".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "foo".into(),
                replacement: "baz".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: Some(1),
            }],
        )
        .unwrap();
        assert_eq!(out, b"baz bar");
    }

    #[test]
    fn regex_expect_matches_mismatch_is_c210() {
        let err = apply_regex_replaces(
            b"foo foo foo".to_vec(),
            &[&UpdateOp::Replace {
                pattern: "foo".into(),
                replacement: "bar".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: Some(1),
            }],
        )
        .unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    // ---------------------------------------------------------------------------
    // Echo primitive unit tests (the scenario coverage lives in
    // handler_tests, driven through the real pipeline)
    // ---------------------------------------------------------------------------

    #[test]
    fn map_through_events_shifts_only_positions_at_or_below() {
        let events = [
            MutationEvent { pos: 2, delta: 1 },
            MutationEvent { pos: 10, delta: -3 },
        ];
        // Above both events (after the first shifts it past the second).
        assert_eq!(map_through_events(12, &events), 10);
        // Between: only the first event is at/below.
        assert_eq!(map_through_events(5, &events), 6);
        // Below both: untouched.
        assert_eq!(map_through_events(1, &events), 1);
        // Exactly at an event position: shifted (<= rule).
        assert_eq!(map_through_events(2, &events), 3);
    }

    #[test]
    fn map_through_events_sequential_frames() {
        // Three line-collapsing replacements all at (current-frame) line 1:
        // a position at line 4 must end up at line 1.
        let events = [
            MutationEvent { pos: 1, delta: -1 },
            MutationEvent { pos: 1, delta: -1 },
            MutationEvent { pos: 1, delta: -1 },
        ];
        assert_eq!(map_through_events(4, &events), 1);
    }

    /// A1b panic guard: out-of-range and inverted regions must degrade to
    /// bounded echoes, never panic (debug overflow or release slice OOB).
    #[test]
    fn build_line_echo_clamps_out_of_range_regions() {
        let final_lines: Vec<String> = vec!["a".into(), "b".into()];
        // Region far beyond EOF.
        let e = build_line_echo(0, 9, 9, 2, &final_lines, None);
        assert!(e.lines.len() <= 2);
        assert_eq!(e.from_line, 2);
        // Inverted region (last << first).
        let e = build_line_echo(0, 5, -3, 2, &final_lines, None);
        assert!(e.lines.len() <= 2);
        // Negative positions.
        let e = build_line_echo(0, -10, -10, 2, &final_lines, None);
        assert_eq!(e.from_line, 1);
        // Empty final body.
        let e = build_line_echo(0, 1, 1, 2, &[], None);
        assert!(e.lines.is_empty());
        // i64 extremes must not overflow the saturating math.
        let e = build_line_echo(0, i64::MIN, i64::MAX, 2, &final_lines, None);
        assert!(e.lines.len() <= 2);
    }

    /// R4 panic guard (mirrors the A1b test above): `build_site_echo`
    /// claims the same clamping discipline as `build_line_echo` — pin it
    /// at unit level: out-of-range and inverted regions degrade to
    /// bounded echoes, never panic.
    #[test]
    fn build_site_echo_clamps_out_of_range_regions() {
        let final_lines: Vec<String> = vec!["a".into(), "b".into()];
        // Region far beyond EOF.
        let e = build_site_echo(0, 9, 9, &final_lines, None);
        assert!(e.lines.len() <= 2);
        assert_eq!(e.from_line, 2);
        // Inverted region (last << first): degrades to a single line.
        let e = build_site_echo(0, 5, -3, &final_lines, None);
        assert_eq!(e.lines, vec!["b"]);
        assert_eq!(e.elided, None);
        // Negative positions.
        let e = build_site_echo(0, -10, -10, &final_lines, None);
        assert_eq!(e.from_line, 1);
        // Empty final body.
        let e = build_site_echo(0, 1, 1, &[], None);
        assert!(e.lines.is_empty());
        // i64 extremes must not overflow the saturating math; the huge
        // clamped region still echoes only first + last with elision.
        let e = build_site_echo(0, i64::MIN, i64::MAX, &final_lines, None);
        assert!(e.lines.len() <= 2);
    }

    #[test]
    fn build_line_echo_elides_large_regions() {
        let final_lines: Vec<String> = (1..=100).map(|i| format!("L{i}")).collect();
        let e = build_line_echo(0, 1, 100, 2, &final_lines, None);
        assert_eq!(e.lines.len(), ECHO_HEAD_TAIL * 2);
        assert_eq!(e.elided, Some(100 - ECHO_HEAD_TAIL as u64 * 2));
        assert_eq!(e.from_line, 1);
        assert_eq!(e.lines[0], "L1");
        assert_eq!(e.lines[ECHO_HEAD_TAIL * 2 - 1], "L100");
    }

    // -----------------------------------------------------------------------
    // R1 (v0.4.1): validator-vs-expand parity.
    // -----------------------------------------------------------------------

    /// R1 — the validator must tokenize `replacement` EXACTLY like
    /// `Captures::expand` (regex-automata `util::interpolate`). Each row
    /// pins BOTH (a) the validator's verdict and (b) the actual
    /// `expand()` output, so any divergence between our tokenizer and the
    /// regex crate's interpolation shows up here. Every `valid: false`
    /// row's expansion demonstrates the hazard the validator guards: the
    /// undefined reference silently expands to the EMPTY STRING.
    #[test]
    fn r1_validator_matches_expand_semantics() {
        struct Case {
            pattern: &'static str,
            haystack: &'static str,
            replacement: &'static str,
            valid: bool,
            expanded: &'static str,
        }
        let cases = [
            // Index and named references to defined groups: valid.
            Case {
                pattern: r"(\w+)=(\d+)",
                haystack: "HOST=8080",
                replacement: "$1: $2",
                valid: true,
                expanded: "HOST: 8080",
            },
            Case {
                pattern: r"(\w+)",
                haystack: "abc",
                replacement: "[$0]",
                valid: true,
                expanded: "[abc]",
            },
            Case {
                pattern: r"(?P<num>\d+)",
                haystack: "42",
                replacement: "n=$num",
                valid: true,
                expanded: "n=42",
            },
            // `$$` is the literal-$ escape: no reference, valid.
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "a $$ b",
                valid: true,
                expanded: "a $ b",
            },
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "$${name}",
                valid: true,
                expanded: "${name}",
            },
            // The production hazard: undefined named reference (JS/TS
            // template literal) expands to the EMPTY STRING.
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "`Hello, ${name}!`",
                valid: false,
                expanded: "`Hello, !`",
            },
            // Unbraced refs consume the longest [0-9A-Za-z_] run: `$1a`
            // names group "1a" (NOT group 1 followed by "a").
            Case {
                pattern: "(x)",
                haystack: "x",
                replacement: "$1a",
                valid: false,
                expanded: "",
            },
            Case {
                pattern: "(x)",
                haystack: "x",
                replacement: "${1}a",
                valid: true,
                expanded: "xa",
            },
            // `$1_` is also one named run ("1_"), undefined here; the
            // following `$0` is a separate, defined reference.
            Case {
                pattern: "(x)",
                haystack: "x",
                replacement: "$1_$0",
                valid: false,
                expanded: "x",
            },
            // Index out of range.
            Case {
                pattern: "(x)",
                haystack: "x",
                replacement: "$2",
                valid: false,
                expanded: "",
            },
            // Leading zeros still parse as an index.
            Case {
                pattern: "(x)",
                haystack: "x",
                replacement: "$01",
                valid: true,
                expanded: "x",
            },
            // A `$` that cannot start a reference is literal.
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "$ price",
                valid: true,
                expanded: "$ price",
            },
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "100$",
                valid: true,
                expanded: "100$",
            },
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "$.x",
                valid: true,
                expanded: "$.x",
            },
            // Unclosed brace: literal, valid.
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "${unclosed",
                valid: true,
                expanded: "${unclosed",
            },
            // Empty braces ARE a reference (to the empty name) — a group
            // can never be named "", so expand drops it: invalid.
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "a${}b",
                valid: false,
                expanded: "ab",
            },
            // `$` followed by non-ASCII: `é` is not a cap letter, so the
            // `$` is literal (UTF-8 boundary handling in the tokenizer
            // must not split the multi-byte char).
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "$é",
                valid: true,
                expanded: "$é",
            },
            // A numeric run too large for usize (2^64) falls back to a
            // NAMED reference — undefined, expands to the empty string.
            Case {
                pattern: "foo",
                haystack: "foo",
                replacement: "$18446744073709551616",
                valid: false,
                expanded: "",
            },
            // Defined-but-non-participating groups are VALID (they may be
            // empty at runtime; that is not an undefined reference).
            Case {
                pattern: "(a)|(b)",
                haystack: "b",
                replacement: "<$1$2>",
                valid: true,
                expanded: "<b>",
            },
        ];
        for c in &cases {
            let re = regex::Regex::new(c.pattern).unwrap();
            let verdict = validate_replacement_refs(&re, c.pattern, c.replacement);
            assert_eq!(
                verdict.is_ok(),
                c.valid,
                "validator verdict for replacement {:?} against pattern {:?} \
                 (got: {verdict:?})",
                c.replacement,
                c.pattern
            );
            let caps = re.captures(c.haystack).expect("haystack must match");
            let mut out = String::new();
            caps.expand(c.replacement, &mut out);
            assert_eq!(
                out, c.expanded,
                "expand() output for replacement {:?} against pattern {:?}",
                c.replacement, c.pattern
            );
        }
    }
}

#[cfg(test)]
mod handler_tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            max_write_bytes: 1024 * 1024,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    #[tokio::test]
    async fn jail_escape_aborts_batch_with_top_level_c215() {
        // A path that escapes the jail must fail resolution before any file
        // I/O and surface as a top-level C215 for the grant hook.
        let (_tmp, r, c) = setup();
        let err = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "../escape.txt".into(),
                    ops: vec![UpdateOp::Insert {
                        at_line: 1,
                        content: "x".into(),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("\"code\":\"C215\""));
        assert!(err.contains("../escape.txt"));
        assert!(err.contains("filesystem_access_request="));
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
                fs_scope: None,
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
                        dot_matches_newline: false,
                        expect_matches: None,
                    }],
                }],
                fs_scope: None,
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
                            dot_matches_newline: false,
                            expect_matches: None,
                        },
                    ],
                }],
                fs_scope: None,
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
                        dot_matches_newline: false,
                        expect_matches: None,
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success);
        assert_eq!(out.results[0].new_line_count, 2);
        // No before/after fields; verify via file content and echo.
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "a=\nb\n"
        );
    }

    #[tokio::test]
    async fn success_includes_echo_not_full_body() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "old\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::UpdateLines {
                        from_line: 1,
                        to_line: 1,
                        content: "new".into(),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success);
        // No full-body fields; echoes present.
        assert!(!r0.echoes.is_empty(), "echo should be present on success");
        assert!(r0.echoes[0].lines.contains(&"new".to_string()));
    }

    #[tokio::test]
    async fn echo_insert_appears_in_handler_result() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "1\n2\n3\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Insert {
                        at_line: 2,
                        content: "inserted".into(),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success);
        assert!(!r0.echoes.is_empty());
        let e = &r0.echoes[0];
        assert!(
            e.lines.contains(&"inserted".to_string()),
            "echo should contain inserted: {e:?}"
        );
    }

    #[tokio::test]
    async fn echo_regex_replace_site_appears_in_result() {
        let (tmp, r, c) = setup();
        std::fs::write(
            tmp.path().join("a.txt"),
            "hello world\nfoo bar\nhello again\n",
        )
        .unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "hello".into(),
                        replacement: "HI".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: None,
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success);
        // Should have echo(es) showing match sites.
        let all_lines: Vec<&str> = r0
            .echoes
            .iter()
            .flat_map(|e| e.lines.iter().map(|s| s.as_str()))
            .collect();
        assert!(
            all_lines.iter().any(|l| l.contains("HI")),
            "echo should contain replaced text; echoes: {:?}",
            r0.echoes
        );
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
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.results.len(), 2);
        assert!(!out.results[0].success);
        assert_eq!(out.results[0].error.as_ref().unwrap().code, "C211");
        assert!(out.results[1].success);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "X\n1\n"
        );
    }

    // C211 IDENTICAL-WORDING (REDACTION INVARIANT), tested end-to-end
    // through the REAL handler: a genuinely missing file and a glob-denied
    // file must produce byte-identical message suffixes after the
    // caller-supplied path prefix, so callers cannot distinguish the two.
    #[tokio::test]
    async fn c211_wording_identical_for_missing_and_glob_denied() {
        let (tmp, r, c) = setup();
        // ".env" exists on disk but matches the non-accessible glob;
        // "missing.txt" does not exist at all.
        std::fs::write(tmp.path().join(".env"), "secret").unwrap();
        let spec = |path: &str| UpdateFileSpec {
            path: path.into(),
            ops: vec![UpdateOp::Insert {
                at_line: 1,
                content: "x".into(),
            }],
        };
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![spec("missing.txt"), spec(".env")],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let missing_err = out.results[0].error.as_ref().expect("missing errors");
        let denied_err = out.results[1].error.as_ref().expect("denied errors");

        assert_eq!(missing_err.code, "C211", "missing code wrong");
        assert_eq!(denied_err.code, "C211", "denied code wrong");

        // Strip the "<path>: " prefix; the remainder must be byte-identical.
        let m_msg = &missing_err.message;
        let d_msg = &denied_err.message;
        let m_suffix = m_msg
            .strip_prefix("missing.txt: ")
            .expect("missing message starts with its wire path");
        let d_suffix = d_msg
            .strip_prefix(".env: ")
            .expect("denied message starts with its wire path");
        assert_eq!(
            m_suffix, d_suffix,
            "C211 wording must not distinguish missing from denied; \
             missing: {m_msg}; denied: {d_msg}"
        );
        // Neither message may carry raw OS error detail.
        assert!(
            !m_msg.contains("os error") && !d_msg.contains("os error"),
            "raw OS text leaked; missing: {m_msg}; denied: {d_msg}"
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
                fs_scope: None,
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
        let err = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![],
                fs_scope: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"));
    }

    // -----------------------------------------------------------------------
    // Echo scenarios, end-to-end through the real handler.
    // -----------------------------------------------------------------------

    /// Write `content`, run `ops` against it, return the single result.
    async fn run_ops(content: &str, ops: Vec<UpdateOp>) -> UpdateFileResult {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), content).unwrap();
        let mut out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "f.txt".into(),
                    ops,
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        out.results.remove(0)
    }

    #[tokio::test]
    async fn echo_insert_correct_post_apply_region() {
        // a..e, insert "X\nY" before line 3 → a,b,X,Y,c,d,e.
        // Region [3,4] ±2 context → from_line 1, lines a..d.
        let r = run_ops(
            "a\nb\nc\nd\ne\n",
            vec![UpdateOp::Insert {
                at_line: 3,
                content: "X\nY".into(),
            }],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        assert!(!r.echoes_truncated);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.op_index, 0);
        assert_eq!(e.from_line, 1);
        assert_eq!(e.lines, vec!["a", "b", "X", "Y", "c", "d"]);
        assert_eq!(e.elided, None);
        assert_eq!(e.total_replacements, None);
    }

    #[tokio::test]
    async fn echo_remove_shows_context_around_gap() {
        // a..e, remove line 3 → a,b,d,e. Gap anchor at line 2 ±2 → all 4.
        let r = run_ops(
            "a\nb\nc\nd\ne\n",
            vec![UpdateOp::Remove {
                from_line: 3,
                to_line: 3,
            }],
        )
        .await;
        assert!(r.success);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.from_line, 1);
        assert_eq!(e.lines, vec!["a", "b", "d", "e"]);
    }

    #[tokio::test]
    async fn echo_update_lines_correct_content_and_position() {
        // a,b,c,d → update 2..3 with X,Y,Z → a,X,Y,Z,d.
        let r = run_ops(
            "a\nb\nc\nd\n",
            vec![UpdateOp::UpdateLines {
                from_line: 2,
                to_line: 3,
                content: "X\nY\nZ".into(),
            }],
        )
        .await;
        assert!(r.success);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.from_line, 1);
        assert_eq!(e.lines, vec!["a", "X", "Y", "Z", "d"]);
    }

    #[tokio::test]
    async fn echo_two_line_ops_offset_correctness() {
        // 1..10; op 0 inserts "X" before line 2 (delta +1), op 1 updates
        // original line 5 → its post-apply position must be 6.
        let content = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let r = run_ops(
            content,
            vec![
                UpdateOp::Insert {
                    at_line: 2,
                    content: "X".into(),
                },
                UpdateOp::UpdateLines {
                    from_line: 5,
                    to_line: 5,
                    content: "FIVE".into(),
                },
            ],
        )
        .await;
        assert!(r.success);
        assert_eq!(r.echoes.len(), 2, "echoes: {:?}", r.echoes);

        let e0 = r.echoes.iter().find(|e| e.op_index == 0).unwrap();
        assert!(e0.lines.contains(&"X".to_string()), "echo0: {e0:?}");
        assert_eq!(e0.from_line, 1);

        // FIVE sits at post-apply line 6; ±2 context → window starts at 4.
        let e1 = r.echoes.iter().find(|e| e.op_index == 1).unwrap();
        assert_eq!(e1.from_line, 4, "echo1: {e1:?}");
        assert_eq!(e1.lines, vec!["3", "4", "FIVE", "6", "7"]);
    }

    #[tokio::test]
    async fn echo_pathological_1000_line_op_stays_bounded() {
        let big: Vec<String> = (0..1000).map(|i| format!("L{i}")).collect();
        let r = run_ops(
            "old\n",
            vec![UpdateOp::UpdateLines {
                from_line: 1,
                to_line: 1,
                content: big.join("\n"),
            }],
        )
        .await;
        assert!(r.success);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.lines.len(), ECHO_HEAD_TAIL * 2);
        assert_eq!(e.elided, Some(1000 - ECHO_HEAD_TAIL as u64 * 2));
    }

    #[tokio::test]
    async fn echo_budget_truncation_sets_flag() {
        // 100 single-line update ops over ~205-byte lines: each echo is
        // ~1 KiB (5 context lines), so the ~4 KiB budget exhausts after a
        // few echoes and the flag MUST be set.
        let long = "X".repeat(200);
        let content: String = (0..200).map(|i| format!("{i}:{long}\n")).collect();
        let ops: Vec<UpdateOp> = (0..100)
            .map(|i| UpdateOp::UpdateLines {
                from_line: i + 1,
                to_line: i + 1,
                content: format!("{i}:{long}"),
            })
            .collect();
        let r = run_ops(&content, ops).await;
        assert!(r.success);
        assert!(
            r.echoes_truncated,
            "echoes_truncated must be set when the budget is exhausted; \
             emitted {} echoes",
            r.echoes.len()
        );
        assert!(!r.echoes.is_empty(), "first echo is always admitted");
        assert!(r.echoes.len() < 100, "remaining echoes must be dropped");
    }

    /// Wire contract: `echoes` and `echoes_truncated` are ALWAYS serialized,
    /// matching the schema's required[] — including on failure results.
    #[tokio::test]
    async fn wire_always_emits_echo_fields() {
        let r = run_ops(
            "a\n",
            vec![UpdateOp::Remove {
                from_line: 5,
                to_line: 9,
            }],
        )
        .await;
        assert!(!r.success);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["echoes"], serde_json::json!([]));
        assert_eq!(v["echoes_truncated"], serde_json::json!(false));
    }

    // -----------------------------------------------------------------------
    // T6 review regressions (A1–A4): permanent reviewer repros.
    // -----------------------------------------------------------------------

    /// A1: a regex whose matches span `\n` collapses many lines; the old
    /// site indices pointed beyond the final body and panicked in the echo
    /// arithmetic. Must never panic; echoes stay bounded and truthful.
    #[tokio::test]
    async fn a1_multiline_collapsing_regex_does_not_panic() {
        let content = "k1\nk2\nk3\nk4\nk5\nk6\nk7\nk8\nk9\nz\n";
        let r = run_ops(
            content,
            vec![UpdateOp::Replace {
                pattern: r"k\d\n".into(),
                replacement: "".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: None,
            }],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        assert_eq!(r.new_line_count, 1);
        // Sites capped at 5, all collapsed onto the surviving line.
        assert_eq!(r.echoes.len(), ECHO_MAX_SITES);
        for e in &r.echoes {
            assert_eq!(e.op_index, 0);
            assert_eq!(e.from_line, 1);
            assert_eq!(e.lines, vec!["z"]);
            assert_eq!(e.total_replacements, Some(9));
        }

        // Same regex deleting the ENTIRE body: bounded empty echoes.
        let r = run_ops(
            "k1\nk2\n",
            vec![UpdateOp::Replace {
                pattern: r"k\d\n".into(),
                replacement: "".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: None,
            }],
        )
        .await;
        assert!(r.success);
        assert_eq!(r.new_line_count, 0);
        for e in &r.echoes {
            assert!(e.lines.is_empty());
            assert_eq!(e.total_replacements, Some(2));
        }
    }

    /// A2: replacements that ADD newlines shift later sites; each site must
    /// report its POST-APPLY position (1/12/23), not its pre-replace line.
    /// R4 (v0.4.1): each 11-line region echoes its FIRST and LAST line
    /// with the 9 inner lines elided.
    #[tokio::test]
    async fn a2_regex_sites_report_post_apply_positions() {
        // Replace each "X" with 11 lines (delta +10 per match).
        let repl: Vec<String> = (1..=11).map(|i| format!("L{i}")).collect();
        let r = run_ops(
            "X\nX\nX\n",
            vec![UpdateOp::Replace {
                pattern: "X".into(),
                replacement: repl.join("\n"),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: None,
            }],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        assert_eq!(r.new_line_count, 33);
        assert_eq!(r.echoes.len(), 3);
        let from_lines: Vec<u64> = r.echoes.iter().map(|e| e.from_line).collect();
        assert_eq!(from_lines, vec![1, 12, 23], "echoes: {:?}", r.echoes);
        for e in &r.echoes {
            assert_eq!(e.op_index, 0);
            assert_eq!(
                e.lines,
                vec!["L1", "L11"],
                "site shows the region's first AND last line"
            );
            assert_eq!(e.elided, Some(9), "inner lines elided");
            assert_eq!(e.total_replacements, Some(3));
        }
    }

    /// A3: each regex op matches on its OWN input body. The second op's
    /// pattern only exists AFTER the first op ran, so it must still find
    /// (and echo) its site — the old single-pass site scan found nothing.
    #[tokio::test]
    async fn a3_sequential_regex_ops_attribute_sites_to_own_input() {
        let r = run_ops(
            "alpha\nmiddle\n",
            vec![
                UpdateOp::Replace {
                    pattern: "alpha".into(),
                    replacement: "beta".into(),
                    ignore_case: false,
                    dot_matches_newline: false,
                    expect_matches: None,
                },
                UpdateOp::Replace {
                    pattern: "beta".into(),
                    replacement: "gamma".into(),
                    ignore_case: false,
                    dot_matches_newline: false,
                    expect_matches: None,
                },
            ],
        )
        .await;
        assert!(r.success);
        let e0 = r
            .echoes
            .iter()
            .find(|e| e.op_index == 0)
            .expect("op 0 echo");
        let e1 = r.echoes.iter().find(|e| e.op_index == 1).expect(
            "op 1 must get an echo: its pattern matches on its OWN input \
             body (which contains beta), not the pre-replace body",
        );
        // Both sites are line 1; content comes from the FINAL body, so both
        // truthfully show what the site looks like now.
        assert_eq!(e1.from_line, 1);
        assert_eq!(e1.lines, vec!["gamma"]);
        assert_eq!(e1.total_replacements, Some(1));
        assert_eq!(e0.from_line, 1);
        assert_eq!(e0.lines, vec!["gamma"]);
        assert_eq!(e0.total_replacements, Some(1));
    }

    /// A4: line-op echoes must shift through regex deltas too. An insert at
    /// line 4 followed by a regex collapsing the 3 lines above it must be
    /// echoed at its TRUE final position (line 1).
    #[tokio::test]
    async fn a4_insert_echo_shifts_through_regex_deltas() {
        let r = run_ops(
            "k1\nk2\nk3\nz\n",
            vec![
                UpdateOp::Insert {
                    at_line: 4,
                    content: "INSERTED".into(),
                },
                UpdateOp::Replace {
                    pattern: r"k\d\n".into(),
                    replacement: "".into(),
                    ignore_case: false,
                    dot_matches_newline: false,
                    expect_matches: None,
                },
            ],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        // Final body: INSERTED\nz\n
        assert_eq!(r.new_line_count, 2);
        let e0 = r
            .echoes
            .iter()
            .find(|e| e.op_index == 0)
            .expect("insert echo");
        assert_eq!(
            e0.from_line, 1,
            "insert region must map through the -3 regex delta"
        );
        assert_eq!(e0.lines[0], "INSERTED", "echo: {e0:?}");
    }

    // -----------------------------------------------------------------------
    // S3 (v0.4.0): replace-op hardening — dot_matches_newline + expect_matches.
    // -----------------------------------------------------------------------

    /// Default `dot_matches_newline: false` keeps current behavior: `.`
    /// does not cross newlines, the multi-line pattern matches nothing,
    /// and 0 matches with no expectation is still a successful no-op
    /// (no sites → no echoes, file byte-identical).
    #[tokio::test]
    async fn s3_dot_default_false_multiline_pattern_matches_nothing() {
        let (tmp, r, c) = setup();
        let original = "start\nmiddle\nend\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "start.*?end".into(),
                        replacement: "X".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: None,
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success, "0 matches without expectation: {:?}", r0.error);
        assert!(
            r0.echoes.is_empty(),
            "no sites → no echoes: {:?}",
            r0.echoes
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            original
        );
    }

    /// `dot_matches_newline: true` lets `start.*?end` span lines. The
    /// replace collapses 4 lines into 1 (delta -3); both the regex site
    /// echo and a line-op echo captured below it must land at FINAL-body
    /// coordinates (extends the a1–a4 echo math to a multi-line-delta
    /// replace driven by the new flag).
    #[tokio::test]
    async fn s3_dot_matches_newline_replaces_region_with_correct_echoes() {
        let (tmp, r, c) = setup();
        std::fs::write(
            tmp.path().join("a.txt"),
            "before\nstart\nmid1\nmid2\nend\nafter\n",
        )
        .unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![
                        UpdateOp::Insert {
                            at_line: 7,
                            content: "TAIL".into(),
                        },
                        UpdateOp::Replace {
                            pattern: "start.*?end".into(),
                            replacement: "ONE".into(),
                            ignore_case: false,
                            dot_matches_newline: true,
                            expect_matches: None,
                        },
                    ],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success, "got: {:?}", r0.error);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "before\nONE\nafter\nTAIL\n"
        );
        assert_eq!(r0.new_line_count, 4);
        // Regex site: matched at line 2 of its own input, context 0.
        let site = r0
            .echoes
            .iter()
            .find(|e| e.op_index == 1)
            .expect("replace site echo");
        assert_eq!(site.from_line, 2, "site: {site:?}");
        assert_eq!(site.lines, vec!["ONE"]);
        assert_eq!(site.total_replacements, Some(1));
        // Insert echo captured at line 7 must shift through the -3 delta
        // of the multi-line replace to final line 4 (±2 context → 2).
        let ins = r0
            .echoes
            .iter()
            .find(|e| e.op_index == 0)
            .expect("insert echo");
        assert!(ins.lines.contains(&"TAIL".to_string()), "echo: {ins:?}");
        assert_eq!(ins.from_line, 2, "echo: {ins:?}");
    }

    /// `expect_matches: 1` with exactly 1 actual match applies normally.
    #[tokio::test]
    async fn s3_expect_matches_exact_count_applies() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "foo bar\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "foo".into(),
                        replacement: "X".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: Some(1),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success, "got: {:?}", r0.error);
        assert_eq!(r0.echoes[0].total_replacements, Some(1));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "X bar\n"
        );
    }

    /// `expect_matches: 1` against 3 actual matches: the FILE fails with a
    /// per-entry C210 naming both counts, nothing is written for it (bytes
    /// identical), no echoes are emitted for it — and the OTHER file in
    /// the same batch still applies with normal echoes.
    #[tokio::test]
    async fn s3_expect_matches_mismatch_fails_file_other_file_applies() {
        let (tmp, r, c) = setup();
        let original = "foo\nfoo\nfoo\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "foo\n").unwrap();
        let replace = |expect: Option<u64>| UpdateOp::Replace {
            pattern: "foo".into(),
            replacement: "bar".into(),
            ignore_case: false,
            dot_matches_newline: false,
            expect_matches: expect,
        };
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![
                    UpdateFileSpec {
                        path: "a.txt".into(),
                        ops: vec![replace(Some(1))],
                    },
                    UpdateFileSpec {
                        path: "b.txt".into(),
                        ops: vec![replace(None)],
                    },
                ],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let ra = &out.results[0];
        assert!(!ra.success);
        let err = ra.error.as_ref().expect("a.txt errors");
        assert_eq!(err.code, "C210");
        assert!(
            err.message.contains("\"foo\" matched 3 times, expected 1"),
            "message must name the failing pattern plus actual and expected: {}",
            err.message
        );
        assert!(
            err.message.contains("expect_matches: 3"),
            "message must offer the corrective value: {}",
            err.message
        );
        assert!(ra.echoes.is_empty(), "failed file emits no echoes");
        assert_eq!(
            std::fs::read(tmp.path().join("a.txt")).unwrap(),
            original.as_bytes(),
            "mismatching file must be byte-identical on disk"
        );
        let rb = &out.results[1];
        assert!(rb.success, "per-entry isolation: {:?}", rb.error);
        assert_eq!(rb.echoes[0].total_replacements, Some(1));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("b.txt")).unwrap(),
            "bar\n"
        );
    }

    /// `expect_matches: 2` against 0 actual matches: C210 routes the
    /// agent to coder::search / dot_matches_newline; file unchanged.
    #[tokio::test]
    async fn s3_expect_matches_zero_actual_routes_to_search() {
        let (tmp, r, c) = setup();
        let original = "hello\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "missing".into(),
                        replacement: "x".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: Some(2),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(!r0.success);
        let err = r0.error.as_ref().expect("errors");
        assert_eq!(err.code, "C210");
        assert!(
            err.message
                .contains("\"missing\" matched 0 times, expected 2"),
            "message must name the failing pattern plus the counts: {}",
            err.message
        );
        assert!(
            err.message.contains("coder::search"),
            "0-match mismatch must route to coder::search: {}",
            err.message
        );
        assert!(
            err.message.contains("dot_matches_newline"),
            "0-match mismatch must hint at dot_matches_newline: {}",
            err.message
        );
        assert_eq!(
            std::fs::read(tmp.path().join("a.txt")).unwrap(),
            original.as_bytes()
        );
    }

    /// `expect_matches: 0` (assert-absent) against 1 actual match: C210
    /// routes to coder::search and must NOT suggest `expect_matches: 1` —
    /// that would invert the caller's assert-absent intent. File unchanged.
    #[tokio::test]
    async fn s3_expect_matches_assert_absent_mismatch_routes_to_search() {
        let (tmp, r, c) = setup();
        let original = "legacy_api()\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "legacy_api".into(),
                        replacement: "x".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: Some(0),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(!r0.success);
        let err = r0.error.as_ref().expect("errors");
        assert_eq!(err.code, "C210");
        assert!(
            err.message
                .contains("\"legacy_api\" matched 1 time, expected 0"),
            "message must name the pattern and counts: {}",
            err.message
        );
        assert!(
            err.message.contains("coder::search"),
            "assert-absent mismatch must route to coder::search: {}",
            err.message
        );
        assert!(
            !err.message.contains("expect_matches:"),
            "must not suggest setting expect_matches to the observed \
             count — that inverts assert-absent intent: {}",
            err.message
        );
        assert_eq!(
            std::fs::read(tmp.path().join("a.txt")).unwrap(),
            original.as_bytes()
        );
    }

    /// Long patterns are truncated in the mismatch message (first 60 chars
    /// + `…`) so multi-op files stay disambiguated without flooding.
    #[test]
    fn pattern_snippet_truncates_long_patterns() {
        let short = "a".repeat(PATTERN_SNIPPET_MAX_CHARS);
        assert_eq!(pattern_snippet(&short), format!("\"{short}\""));
        let long = "b".repeat(PATTERN_SNIPPET_MAX_CHARS + 10);
        let shown = pattern_snippet(&long);
        assert_eq!(
            shown,
            format!("\"{}…\"", "b".repeat(PATTERN_SNIPPET_MAX_CHARS))
        );
        // Char-boundary safe on multi-byte input.
        let emoji = "é".repeat(PATTERN_SNIPPET_MAX_CHARS + 1);
        assert_eq!(
            pattern_snippet(&emoji),
            format!("\"{}…\"", "é".repeat(PATTERN_SNIPPET_MAX_CHARS))
        );
    }

    /// Mismatch messages pluralize the match count correctly.
    #[test]
    fn times_pluralizes_match_counts() {
        assert_eq!(times(0), "0 times");
        assert_eq!(times(1), "1 time");
        assert_eq!(times(3), "3 times");
    }

    /// Multiple ops in one file: op 0 would apply cleanly, but op 1's
    /// expect_matches mismatch fails the WHOLE file — op 0's would-be
    /// change must not reach disk (all-or-nothing per file).
    #[tokio::test]
    async fn s3_second_op_expect_mismatch_rolls_back_whole_file() {
        let (tmp, r, c) = setup();
        let original = "alpha\nkeep\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![
                        UpdateOp::Replace {
                            pattern: "alpha".into(),
                            replacement: "beta".into(),
                            ignore_case: false,
                            dot_matches_newline: false,
                            expect_matches: Some(1),
                        },
                        UpdateOp::Replace {
                            pattern: "nope".into(),
                            replacement: "x".into(),
                            ignore_case: false,
                            dot_matches_newline: false,
                            expect_matches: Some(1),
                        },
                    ],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(!r0.success, "second op's mismatch must fail the file");
        assert_eq!(r0.error.as_ref().unwrap().code, "C210");
        assert!(r0.echoes.is_empty());
        assert_eq!(
            std::fs::read(tmp.path().join("a.txt")).unwrap(),
            original.as_bytes(),
            "op 0's would-be change must not land when op 1 fails"
        );
    }

    // -----------------------------------------------------------------------
    // R1 (v0.4.1): pre-write validation of replacement capture references.
    // -----------------------------------------------------------------------

    /// R1 — the EXACT production repro (live session q8x6g248): a harness
    /// agent rewrote a JS handler with a replacement containing the
    /// template literal `Hello, ${name}!`. `Captures::expand` treated
    /// `${name}` as a capture reference, the pattern defines no group
    /// `name`, and the regex crate expands undefined references to the
    /// EMPTY STRING — the file got `Hello, !` with success: true (silent
    /// corruption, twice in one session). The reference must now fail
    /// pre-write with a C210 naming `name`; the file stays byte-identical
    /// on disk and other files in the batch still apply.
    #[tokio::test]
    async fn r1_production_repro_template_literal_ref_rejected_pre_write() {
        let (tmp, r, c) = setup();
        let original = "import { iii } from 'iii';\n\n\
                        iii.registerFunction({\n  \
                        handler: () => ({ body: { message: 'hi' } }),\n});\n";
        std::fs::write(tmp.path().join("handler.js"), original).unwrap();
        std::fs::write(tmp.path().join("other.txt"), "foo\n").unwrap();
        let replacement = "iii.registerFunction({\n  \
                           handler: (req) => {\n    \
                           const name = req.query.name ?? 'world';\n    \
                           return { status: 200, body: { message: `Hello, ${name}!` } };\n  \
                           },\n});\n";
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![
                    UpdateFileSpec {
                        path: "handler.js".into(),
                        ops: vec![UpdateOp::Replace {
                            pattern: r"iii\.registerFunction\(.*".into(),
                            replacement: replacement.into(),
                            ignore_case: false,
                            dot_matches_newline: true,
                            expect_matches: Some(1),
                        }],
                    },
                    UpdateFileSpec {
                        path: "other.txt".into(),
                        ops: vec![UpdateOp::Replace {
                            pattern: "foo".into(),
                            replacement: "bar".into(),
                            ignore_case: false,
                            dot_matches_newline: false,
                            expect_matches: Some(1),
                        }],
                    },
                ],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(!r0.success, "undefined ${{name}} must fail pre-write");
        let err = r0.error.as_ref().expect("entry must carry an error");
        assert_eq!(err.code, "C210");
        assert!(
            err.message.contains("`${name}`"),
            "message must name the offending reference: {}",
            err.message
        );
        assert!(
            err.message.contains("0 capture groups"),
            "message must state what the pattern defines: {}",
            err.message
        );
        assert!(
            err.message.contains("$$"),
            "message must teach the $$ escape: {}",
            err.message
        );
        assert!(
            err.message.contains("template literal"),
            "message must name the JS/TS template-literal collision: {}",
            err.message
        );
        assert!(r0.echoes.is_empty(), "failed file emits no echoes");
        assert_eq!(
            std::fs::read(tmp.path().join("handler.js")).unwrap(),
            original.as_bytes(),
            "file must be byte-identical on disk — NOTHING written"
        );
        // Per-entry isolation: the other file in the batch still applies.
        let r1 = &out.results[1];
        assert!(r1.success, "batch isolation: {:?}", r1.error);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("other.txt")).unwrap(),
            "bar\n"
        );
    }

    /// R1 — `$$` escape: the corrective rewrite from the C210 message
    /// (`$${name}`) must write a literal `${name}` to disk.
    #[tokio::test]
    async fn r1_dollar_dollar_escape_writes_literal_dollar() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.js"), "MSG\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.js".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: "MSG".into(),
                        replacement: "`Hello, $${name}!`".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: Some(1),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success, "got: {:?}", r0.error);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.js")).unwrap(),
            "`Hello, ${name}!`\n"
        );
    }

    /// R1 — the unbraced longest-run gotcha: `$1a` is a reference to a
    /// group NAMED "1a" (NOT group 1 followed by "a"); with only group 1
    /// defined it must fail C210 naming `1a`. The braced disambiguation
    /// `${1}a` applies fine.
    #[tokio::test]
    async fn r1_unbraced_longest_run_gotcha_dollar_1a() {
        let (tmp, r, c) = setup();
        let original = "x\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let op = |replacement: &str| UpdateOp::Replace {
            pattern: "(x)".into(),
            replacement: replacement.into(),
            ignore_case: false,
            dot_matches_newline: false,
            expect_matches: Some(1),
        };
        let out = handle(
            r.clone(),
            c.clone(),
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![op("$1a")],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(!r0.success, "$1a names group `1a`, which does not exist");
        let err = r0.error.as_ref().unwrap();
        assert_eq!(err.code, "C210");
        assert!(
            err.message.contains("`$1a`") && err.message.contains("`1a`"),
            "message must name the `1a` reference: {}",
            err.message
        );
        assert_eq!(
            std::fs::read(tmp.path().join("a.txt")).unwrap(),
            original.as_bytes()
        );
        // The braced disambiguation works.
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![op("${1}a")],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        assert!(out.results[0].success, "got: {:?}", out.results[0].error);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "xa\n"
        );
    }

    /// R1 — valid rewrites keep working unchanged: `$0`, `$1` with a
    /// group, `$name` with a named group.
    #[tokio::test]
    async fn r1_valid_capture_rewrites_still_apply() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "HOST=8080\n").unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        pattern: r"(?P<key>\w+)=(\d+)".into(),
                        replacement: "$key: $2 (was $0)".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: Some(1),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(r0.success, "got: {:?}", r0.error);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
            "HOST: 8080 (was HOST=8080)\n"
        );
    }

    /// R1 — undefined-ref validation precedes the `expect_matches`
    /// guard: even with `expect_matches: 0` (assert-absence, where the
    /// replacement goes unused on success) an undefined reference fails
    /// with the capture-ref C210 — the replacement must be well-formed
    /// even when unused.
    #[tokio::test]
    async fn r1_undefined_ref_rejected_even_with_expect_matches_zero() {
        let (tmp, r, c) = setup();
        let original = "plain\n";
        std::fs::write(tmp.path().join("a.txt"), original).unwrap();
        let out = handle(
            r,
            c,
            UpdateFileInput {
                files: vec![UpdateFileSpec {
                    path: "a.txt".into(),
                    ops: vec![UpdateOp::Replace {
                        // Matches nothing: expect_matches: 0 alone would
                        // succeed without ever using the replacement.
                        pattern: "missing".into(),
                        replacement: "`Hi, ${name}!`".into(),
                        ignore_case: false,
                        dot_matches_newline: false,
                        expect_matches: Some(0),
                    }],
                }],
                fs_scope: None,
            },
        )
        .await
        .unwrap();
        let r0 = &out.results[0];
        assert!(!r0.success, "undefined ref must fail even when unused");
        let err = r0.error.as_ref().expect("entry must carry an error");
        assert_eq!(err.code, "C210");
        assert!(
            err.message.contains("references capture group `${name}`"),
            "the capture-ref C210 (not the mismatch wording) must fire: {}",
            err.message
        );
        assert_eq!(
            std::fs::read(tmp.path().join("a.txt")).unwrap(),
            original.as_bytes()
        );
    }

    // -----------------------------------------------------------------------
    // R4 (v0.4.1): multi-line replace site echoes show first AND last line.
    // -----------------------------------------------------------------------

    /// R4 — a replace whose post-replace region spans >2 lines echoes the
    /// FIRST and LAST line of the region with `elided` set to the inner
    /// line count. The old matched-first-line-only echo hid everything
    /// below the region's first line: in production (session q8x6g248)
    /// the corrupted "Hello, !" line sat in the tail of the replaced
    /// region and stayed invisible until a full read-file — with this
    /// echo shape the region's tail is visible in the mutation response.
    #[tokio::test]
    async fn r4_multiline_replace_echo_shows_first_and_last_lines() {
        let r = run_ops(
            "before\nstart\nmid\nend\nafter\n",
            vec![UpdateOp::Replace {
                pattern: "start.*?end".into(),
                replacement: "NEW_HEAD\ninner1\ninner2\nNEW_TAIL".into(),
                ignore_case: false,
                dot_matches_newline: true,
                expect_matches: Some(1),
            }],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        assert_eq!(r.new_line_count, 6);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.op_index, 0);
        assert_eq!(e.from_line, 2);
        assert_eq!(
            e.lines,
            vec!["NEW_HEAD", "NEW_TAIL"],
            "echo must show the region's first AND last line: {e:?}"
        );
        assert_eq!(e.elided, Some(2), "inner line count elided");
        assert_eq!(e.total_replacements, Some(1));
    }

    /// R4 — a two-line region echoes both lines with no elision.
    #[tokio::test]
    async fn r4_two_line_replace_echo_shows_both_lines_no_elision() {
        let r = run_ops(
            "a\nTARGET\nz\n",
            vec![UpdateOp::Replace {
                pattern: "TARGET".into(),
                replacement: "first\nlast".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: Some(1),
            }],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.from_line, 2);
        assert_eq!(e.lines, vec!["first", "last"]);
        assert_eq!(e.elided, None);
    }

    /// R4 — single-line replacements are unchanged: one line, no elision.
    #[tokio::test]
    async fn r4_single_line_replace_echo_unchanged() {
        let r = run_ops(
            "a\nTARGET tail\nz\n",
            vec![UpdateOp::Replace {
                pattern: "TARGET".into(),
                replacement: "DONE".into(),
                ignore_case: false,
                dot_matches_newline: false,
                expect_matches: Some(1),
            }],
        )
        .await;
        assert!(r.success, "got: {:?}", r.error);
        assert_eq!(r.echoes.len(), 1);
        let e = &r.echoes[0];
        assert_eq!(e.from_line, 2);
        assert_eq!(e.lines, vec!["DONE tail"]);
        assert_eq!(e.elided, None);
    }
}
