//! `coder::read-file` — return file content + metadata.
//!
//! **Single-path mode** (`path`): unchanged from T7 — full reads capped by
//! `max_read_bytes`; optional `line_from`/`line_to` switch to a windowed
//! streamed read. Non-accessible paths return C211.
//!
//! **Batch mode** (`paths[]`): read multiple files or windows in one call.
//! Each entry is a `ReadTarget`: a plain string (whole-file read) or an
//! object `{path, line_from?, line_to?}` (per-entry window). Entries are
//! processed in request order against a shared `batch_read_budget_bytes`
//! cap measured in BYTES OF RETURNED CONTENT (after UTF-8 sanitization —
//! invalid bytes expand to 3-byte U+FFFD replacements BEFORE they are
//! counted, so binary files can never deliver more than the budget). An
//! entry cut short by the remaining budget succeeds with `more_lines:
//! true`; an entry reached with zero budget gets a per-entry C213 (names
//! the config key + value, bytes consumed, and recovery guidance).
//! Per-entry resolution/glob/stat failures return per-entry C211; budget
//! is not consumed by failed entries.
//!
//! REDACTION INVARIANT (batch): error classification NEVER depends on
//! budget state — resolve + stat run BEFORE the zero-budget check, so a
//! missing path and a glob-denied path both return C211 (identical
//! wording, verbatim path echo) even after exhaustion. Only an existing,
//! accessible regular file may receive the budget C213.
//!
//! **XOR rule**: `path` XOR `paths` must be set; both or neither → C210.
//!
//! **S4 additions (v0.4.0)**: `stat: true` (single-path + per-entry)
//! returns metadata only — no content — with `total_lines`/`is_utf8`
//! counted via a bounded read (never more than `max_read_bytes`; null
//! beyond it, while size/mode/mtime still populate). `numbered: true`
//! prefixes each content line `N→` with its ABSOLUTE 1-based file line
//! number; prefix bytes are charged against every byte cap/budget.
//! Single-path FULL reads are additionally bounded by the
//! `max_output_bytes` config (per-call override clamped to
//! `max_read_bytes`); the C213 carries size + total_lines + the
//! corrective calls. REDACTION ORDERING everywhere: resolve → deny
//! (C211) → metadata syscalls → budget (C213) — classification must
//! never depend on budget state, and deny must precede any metadata
//! syscall so stat/budget can't become an existence or size oracle.
//!
//! LINE CONVENTION (shared with `coder::update-file`): a line is a
//! 0x0A-terminated or EOF-terminated byte segment. An empty file has 0
//! lines; a trailing newline does NOT create a phantom last line. This
//! is exactly `str::lines()` counting — the same convention
//! `update_file::split_file` uses for its 1-based line ops — so line
//! numbers reported here address the same lines `update-file` edits.
//! (The two must not drift: agents read a window, then edit those line
//! numbers.) Windowed content keeps each line's raw terminator bytes.

use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError, WireError};
use crate::code::path::PathResolver;

use super::read_window::{count_lines, lossy_utf8, number_lines, read_window, read_window_wire};

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

/// A single entry in a `paths[]` batch request. Pass either a bare file
/// path string (whole-file read, same cap as `max_read_bytes`) or an
/// object with optional per-entry `line_from`/`line_to` window parameters
/// (1-based, inclusive — same rules as the top-level `path` mode).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ReadTarget {
    /// Bare path string: read the whole file (within remaining batch budget
    /// and `max_read_bytes`).
    Path(String),
    /// Object form: path plus optional 1-based window parameters. Omit
    /// `line_from` to start from line 1; omit `line_to` to read to EOF.
    Window {
        /// File to read. Same jail rules as the top-level `path` field.
        path: String,
        /// First line of the window, 1-based inclusive (must be >= 1; 0
        /// is rejected with C210 for this entry). Defaults to 1 when
        /// only `line_to` is set.
        #[serde(default)]
        #[schemars(range(min = 1))]
        line_from: Option<u64>,
        /// Last line of the window, 1-based inclusive. Must be >=
        /// `line_from` (C210 for this entry otherwise). Omit to read from
        /// `line_from` to EOF.
        #[serde(default)]
        #[schemars(range(min = 1))]
        line_to: Option<u64>,
        /// Per-entry metadata probe: same semantics as the top-level
        /// `stat` field — size/mode/mtime always, `total_lines`/`is_utf8`
        /// when the file fits `max_read_bytes`, content null, no batch
        /// budget consumed. C210 when combined with this entry's
        /// `line_from`/`line_to` or `numbered`.
        #[serde(default)]
        stat: bool,
        /// Prefix this entry's content lines with their absolute 1-based
        /// file line numbers (`N→`) — same semantics as the top-level
        /// `numbered` field. Prefix bytes are charged against
        /// `batch_read_budget_bytes`.
        #[serde(default)]
        numbered: bool,
    },
}

impl ReadTarget {
    fn path(&self) -> &str {
        match self {
            ReadTarget::Path(p) => p,
            ReadTarget::Window { path, .. } => path,
        }
    }

    fn window_params(&self) -> (Option<u64>, Option<u64>) {
        match self {
            ReadTarget::Path(_) => (None, None),
            ReadTarget::Window {
                line_from, line_to, ..
            } => (*line_from, *line_to),
        }
    }

    /// `(stat, numbered)` for this entry; bare string targets carry
    /// neither flag.
    fn flags(&self) -> (bool, bool) {
        match self {
            ReadTarget::Path(_) => (false, false),
            ReadTarget::Window { stat, numbered, .. } => (*stat, *numbered),
        }
    }
}

// examples are wire-contract; goldens pin them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(
    example = "example_read_file_input",
    example = "example_read_file_batch"
)]
pub struct ReadFileInput {
    /// Single file to read. Relative to the primary allowed root, or an
    /// absolute path inside any allowed root. Call `coder::info` to see
    /// the allowed roots. Paths outside every allowed root are rejected —
    /// use the shell worker's `shell::fs::*` for host paths outside the
    /// jail. Mutually exclusive with `paths` (XOR): pass either `path` or
    /// `paths`, not both — C210 if both or neither is set.
    #[serde(default)]
    pub path: Option<String>,
    /// First line of the window, 1-based inclusive (must be >= 1; 0 is
    /// rejected with C210). Setting `line_from` and/or `line_to` switches
    /// to windowed mode: the file is streamed and only the requested
    /// lines are returned, so files larger than `max_read_bytes` stay
    /// readable slice by slice — the byte cap then applies to the
    /// returned window, never the file size. Defaults to 1 when only
    /// `line_to` is set. A window starting past EOF succeeds with empty
    /// content and reports the file's `total_lines`. Only valid in
    /// single-path mode (`path`); ignored when `paths` is set. Lines are
    /// 0x0A- or EOF-terminated segments; a trailing newline does not
    /// create a phantom line (same convention as `coder::update-file`).
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub line_from: Option<u64>,
    /// Last line of the window, 1-based inclusive. Must be >= `line_from`
    /// (C210 otherwise). Omit to read from `line_from` to end-of-file
    /// (still bounded by `max_read_bytes` on the returned bytes). Only
    /// valid in single-path mode (`path`); ignored when `paths` is set.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub line_to: Option<u64>,
    /// Metadata probe — the cheap "how big is it" call. When true the
    /// response carries size/mode/mtime plus `total_lines` and `is_utf8`
    /// (both null when the file exceeds `max_read_bytes` — size/mode/mtime
    /// still populate, so stat on a huge file SUCCEEDS); `content` is
    /// null, `lines_returned` 0, `more_lines` false. Probe BEFORE reading
    /// an unknown file, then fetch just the slice you need with
    /// `line_from`/`line_to`. Mutually exclusive with `line_from`,
    /// `line_to`, `numbered`, and `max_output_bytes` (C210 — stat returns
    /// no content for them to act on). Batch entries take a per-entry
    /// `stat` field instead; this top-level flag is ignored when `paths`
    /// is set.
    #[serde(default)]
    pub stat: bool,
    /// When true every returned content line is prefixed `N→`, where N is
    /// the line's ABSOLUTE 1-based number in the file — a window starting
    /// at `line_from: 40` is numbered from 40, not 1. Numbers match
    /// `coder::update-file`'s 1-based line ops exactly, so you can go
    /// from a numbered read straight to a line edit. Prefix bytes count
    /// toward all byte caps and budgets (no hidden bypass). C210 with
    /// `stat: true` (no content to number). Batch entries take a
    /// per-entry `numbered` field instead; this top-level flag is ignored
    /// when `paths` is set.
    #[serde(default)]
    pub numbered: bool,
    /// Per-call override of the `max_output_bytes` config (default
    /// 131072) that budgets single-path FULL reads, measured in returned
    /// content bytes after UTF-8 conversion (numbered prefixes included).
    /// Values above `max_read_bytes` are silently clamped to it. When the
    /// full content would exceed the effective budget the call fails with
    /// a C213 naming the file's size and `total_lines` — recover by
    /// windowing with `line_from`/`line_to`, probing with `stat: true`,
    /// or raising this field. Full reads only: combining it with
    /// `line_from`/`line_to` is C210 (windows are bounded by
    /// `max_read_bytes` instead); ignored when `paths` is set (batch mode
    /// is governed by `batch_read_budget_bytes`).
    #[serde(default)]
    pub max_output_bytes: Option<u64>,
    /// Batch of files (or windowed slices) to read in a single call.
    /// Each entry is either a plain path string (whole-file read) or an
    /// object `{path, line_from?, line_to?}` with per-entry window
    /// parameters. Entries are processed in request order against a
    /// shared `batch_read_budget_bytes` cap, measured in bytes of
    /// returned content (after UTF-8 sanitization) — see `coder::info`
    /// for the configured value. Results are returned in the `results`
    /// field; top-level fields are null. Mutually exclusive with `path`
    /// (XOR): pass either `path` or `paths`, not both — C210 if both or
    /// neither is set.
    #[serde(default)]
    pub paths: Option<Vec<ReadTarget>>,
    /// Optional per-call session working directory. When set, every relative
    /// path (the single `path` or each `paths[]` entry, in both the bare
    /// string and `{path,...}` object forms) anchors here instead of the
    /// primary allowed root, and every resolved path must stay inside it.
    /// `base_dir` itself must canonicalize inside an allowed root
    /// (`coder::info` lists them). Omit to resolve against the primary
    /// allowed root exactly as before.
    #[serde(default)]
    pub base_dir: Option<String>,
}

// examples are wire-contract; goldens pin them.
fn example_read_file_input() -> serde_json::Value {
    serde_json::json!({
        "path": "src/main.rs",
        "line_from": 10,
        "line_to": 50
    })
}

/// Batch form: mix bare path strings and {path, line_from, line_to} objects.
fn example_read_file_batch() -> serde_json::Value {
    serde_json::json!({
        "paths": [
            "src/lib.rs",
            { "path": "src/config.rs", "line_from": 1, "line_to": 30 }
        ]
    })
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// Per-entry result in a batch `paths[]` response.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadEntryResult {
    /// Canonical absolute path of the file (resolved through the jail).
    /// If resolution failed, this echoes the caller's input verbatim.
    pub path: String,
    /// `true` when the read succeeded (content/metadata fields are
    /// populated); `false` when an error occurred (only `error` is set).
    pub success: bool,
    /// File content as a UTF-8 string — the whole file or the requested
    /// window. Binary bytes are replaced by U+FFFD (`is_utf8: false`).
    /// `null` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Whether `content` survived UTF-8 conversion without losing bytes.
    /// `null` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_utf8: Option<bool>,
    /// Number of lines returned in `content`. `null` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_returned: Option<u64>,
    /// Total lines in the file; present when the stream reached EOF during
    /// this entry's read. `null` when not traversed or on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<u64>,
    /// `true` when the file has content beyond what `content` includes
    /// (window ended before EOF, or byte budget cut the window short).
    /// `null` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_lines: Option<bool>,
    /// Size of the FILE in bytes (from metadata). `null` on failure or
    /// when the entry budget was exhausted before the file was opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Unix permission bits (lower 9 bits of `st_mode`), e.g. 0o644.
    /// `null` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Last-modified time as a Unix epoch in seconds. `null` on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
    /// Structured error — present only when `success: false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadFileOutput {
    /// Canonical absolute path of the file read (resolved through the
    /// jail). **Single-path mode only; null when the request used
    /// `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// File content as a UTF-8 string — the whole file, or just the
    /// requested window when `line_from`/`line_to` was given (window
    /// lines keep their newline terminators). Binary content is returned
    /// with invalid bytes replaced by U+FFFD; use a future binary-aware
    /// function if exact bytes matter. **Single-path mode only; null when
    /// the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Whether `content` survived UTF-8 conversion without losing bytes.
    /// Reflects the RETURNED content only: a clean window inside an
    /// otherwise-binary file is still `true`. **Single-path mode only;
    /// null when the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_utf8: Option<bool>,
    /// Number of lines in `content`. For full reads this equals the
    /// file's total line count. **Single-path mode only; null when the
    /// request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_returned: Option<u64>,
    /// Total number of lines in the file. Present only when the read
    /// traversed the whole file: always for full reads; for windowed
    /// reads only when the stream naturally reached EOF within the byte
    /// cap. Never computed by forcing an extra full scan — absent means
    /// the file was not fully traversed. **Single-path mode only; null
    /// when the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<u64>,
    /// True when the file has content beyond what `content` includes:
    /// the window ended before EOF, or the byte budget cut the window
    /// short. Always false for full reads. **Single-path mode only; null
    /// when the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_lines: Option<bool>,
    /// Size of the FILE in bytes (from metadata) — not the size of
    /// `content`; in windowed mode the two differ. **Single-path mode
    /// only; null when the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Unix permission bits (lower 9 bits of `st_mode`), e.g. 0o644.
    /// **Single-path mode only; null when the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Last-modified time as a Unix epoch in seconds. **Single-path mode
    /// only; null when the request used `paths[]`.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
    /// Per-entry results for a batch `paths[]` request. **Present only
    /// when the request used `paths[]`; null in single-path mode.**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<ReadEntryResult>>,
}

// ---------------------------------------------------------------------------
// Public handler
// ---------------------------------------------------------------------------

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: ReadFileInput,
) -> Result<ReadFileOutput, String> {
    // Offload the synchronous read (incl. multi-file batch reads against the
    // shared byte budget) to a blocking thread so a large/batch read can't
    // stall the shared runtime (shell::exec/jobs/reload).
    tokio::task::spawn_blocking(move || inner(&resolver, &cfg, req).map_err(err_to_string))
        .await
        .map_err(|e| format!("read-file task join failed: {e}"))?
}

// ---------------------------------------------------------------------------
// Internal dispatch
// ---------------------------------------------------------------------------

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: ReadFileInput,
) -> Result<ReadFileOutput, CoderError> {
    match (&req.path, &req.paths) {
        // XOR: both set
        (Some(_), Some(_)) => Err(CoderError::BadInput(
            "pass either path or paths, not both; the two modes are mutually exclusive \
             (C210). Use path for a single-file read, or paths[] for a batch read."
                .into(),
        )),
        // XOR: neither set
        (None, None) => Err(CoderError::BadInput(
            "either path or paths must be set (C210). \
             Use path for a single-file read, or paths[] for a batch read."
                .into(),
        )),
        // Single-path mode
        (Some(p), None) => {
            let p = p.clone();
            let single_req = SingleReadReq {
                path: &p,
                base_dir: req.base_dir.as_deref(),
                line_from: req.line_from,
                line_to: req.line_to,
                stat: req.stat,
                numbered: req.numbered,
                max_output_bytes: req.max_output_bytes,
            };
            single_read(resolver, cfg, single_req).map(|o| ReadFileOutput {
                path: Some(o.path),
                content: o.content,
                is_utf8: o.is_utf8,
                lines_returned: Some(o.lines_returned),
                total_lines: o.total_lines,
                more_lines: Some(o.more_lines),
                size: Some(o.size),
                mode: Some(o.mode),
                mtime: Some(o.mtime),
                results: None,
            })
        }
        // Batch mode
        (None, Some(targets)) => {
            let results = batch_read(resolver, cfg, req.base_dir.as_deref(), targets);
            Ok(ReadFileOutput {
                path: None,
                content: None,
                is_utf8: None,
                lines_returned: None,
                total_lines: None,
                more_lines: None,
                size: None,
                mode: None,
                mtime: None,
                results: Some(results),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Single-path mode (T7 + full reads)
// ---------------------------------------------------------------------------

struct SingleReadReq<'a> {
    path: &'a str,
    base_dir: Option<&'a str>,
    line_from: Option<u64>,
    line_to: Option<u64>,
    stat: bool,
    numbered: bool,
    max_output_bytes: Option<u64>,
}

/// Internal result for a single-path read before wrapping in
/// `ReadFileOutput`. `content`/`is_utf8` are `None` for stat probes.
struct SingleReadOut {
    path: String,
    content: Option<String>,
    is_utf8: Option<bool>,
    lines_returned: u64,
    total_lines: Option<u64>,
    more_lines: bool,
    size: u64,
    mode: u32,
    mtime: i64,
}

/// C210 for a field combined with `stat: true`. Prescriptive: stat
/// returns no content, so content-shaping fields cannot act.
fn stat_conflict(field: &str) -> CoderError {
    CoderError::BadInput(format!(
        "stat: true returns metadata only — no content — so {field} has \
         no effect (C210). Choose one: drop {field} to probe metadata, or \
         drop stat to read content."
    ))
}

fn single_read(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: SingleReadReq<'_>,
) -> Result<SingleReadOut, CoderError> {
    // Pure input validation first (C210), before any path is touched.
    let window = parse_window(req.line_from, req.line_to)?;
    if req.stat {
        if window.is_some() {
            return Err(stat_conflict("line_from/line_to"));
        }
        if req.numbered {
            return Err(stat_conflict("numbered"));
        }
        if req.max_output_bytes.is_some() {
            return Err(stat_conflict("max_output_bytes"));
        }
    }
    if req.max_output_bytes.is_some() && window.is_some() {
        return Err(CoderError::BadInput(
            "max_output_bytes budgets FULL reads only; a line_from/line_to \
             window is already bounded by max_read_bytes (C210). Drop \
             line_from/line_to to apply the budget, or drop \
             max_output_bytes to read the window."
                .into(),
        ));
    }
    // REDACTION ORDERING: resolve + deny-check (C211) BEFORE any metadata
    // syscall — stat on a denied path must be byte-identical to stat on a
    // missing path, and no budget may reclassify either.
    let abs = resolver.require_writable_opt(req.base_dir, req.path)?;
    let md = std::fs::metadata(&abs).map_err(|e| CoderError::io_for_path(e, req.path))?;
    if !md.is_file() {
        return Err(CoderError::BadInput(format!(
            "not a regular file: {}",
            req.path
        )));
    }
    if req.stat {
        return stat_read(&abs, req.path, cfg, &md);
    }
    match window {
        None => full_read(&abs, req.path, cfg, &md, req.numbered, req.max_output_bytes),
        Some((from, to)) => windowed_read(&abs, req.path, cfg, &md, from, to, req.numbered),
    }
}

/// Validate the window parameters. `Ok(None)` means full (non-windowed)
/// read; `Ok(Some((from, to)))` is the normalized 1-based inclusive
/// window (`from` defaults to 1, `to: None` means end-of-file).
fn parse_window(
    line_from: Option<u64>,
    line_to: Option<u64>,
) -> Result<Option<(u64, Option<u64>)>, CoderError> {
    if line_from.is_none() && line_to.is_none() {
        return Ok(None);
    }
    if line_from == Some(0) {
        return Err(CoderError::BadInput(
            "line_from must be >= 1 (line numbers are 1-based); got 0. \
             Use line_from=1 for the first line."
                .into(),
        ));
    }
    let from = line_from.unwrap_or(1);
    if let Some(to) = line_to {
        if to < from {
            return Err(CoderError::BadInput(format!(
                "line_to ({to}) must be >= line_from ({from}); the window \
                 is 1-based and inclusive. Swap or widen the bounds."
            )));
        }
    }
    Ok(Some((from, line_to)))
}

/// Full (non-windowed) read: the whole file, pre-checked against
/// `max_read_bytes`, then against the `max_output_bytes` context budget
/// (converted wire bytes, numbered prefixes included). Both C213s are
/// recovery tools: they name the actual sizes and the corrective calls.
///
/// ORDERING (REDACTION RULE): callers run resolve → deny → metadata
/// before reaching here, so by construction only an existing, accessible
/// regular file can ever receive either C213.
fn full_read(
    abs: &Path,
    wire_path: &str,
    cfg: &CoderConfig,
    md: &std::fs::Metadata,
    numbered: bool,
    max_output_override: Option<u64>,
) -> Result<SingleReadOut, CoderError> {
    if md.len() > cfg.max_read_bytes {
        return Err(CoderError::TooLarge(format!(
            "{} is {} bytes, which exceeds max_read_bytes ({}). \
             Read a smaller file, raise max_read_bytes in coder config, \
             or read a slice with line_from/line_to.",
            wire_path,
            md.len(),
            cfg.max_read_bytes
        )));
    }
    let bytes = std::fs::read(abs).map_err(|e| CoderError::io_for_path(e, wire_path))?;
    let lines = count_lines(&bytes);
    let (content, is_utf8) = lossy_utf8(bytes);
    let content = if numbered {
        number_lines(&content, 1)
    } else {
        content
    };
    // Per-call override clamps SILENTLY to max_read_bytes (documented on
    // the input field); without an override the config value applies.
    let budget = max_output_override
        .map(|v| v.min(cfg.max_read_bytes))
        .unwrap_or(cfg.max_output_bytes);
    if content.len() as u64 > budget {
        // The error IS the recovery tool: it carries the stat facts
        // (size, total_lines) plus every corrective call, so the agent's
        // next call can succeed from the message alone.
        return Err(CoderError::TooLarge(format!(
            "{wire_path}: a full read would return {} bytes of content \
             (file is {} bytes, {lines} lines), which exceeds \
             max_output_bytes ({budget}). To recover: read a slice with \
             line_from/line_to, probe metadata cheaply with stat: true, or \
             re-call with a higher per-call max_output_bytes (values above \
             max_read_bytes are clamped).",
            content.len(),
            md.len(),
        )));
    }
    Ok(SingleReadOut {
        path: abs.display().to_string(),
        content: Some(content),
        is_utf8: Some(is_utf8),
        lines_returned: lines,
        total_lines: Some(lines),
        more_lines: false,
        size: md.len(),
        mode: unix_mode(md),
        mtime: unix_mtime(md),
    })
}

/// Metadata-only probe (`stat: true`): size/mode/mtime from `md`, plus
/// `total_lines`/`is_utf8` from a bounded read — never more than
/// `max_read_bytes` of work. Shared by single-path and batch modes.
///
/// CALLERS MUST have run resolve → deny-check (C211) BEFORE calling —
/// the bounded read is a metadata syscall in the redaction-ordering
/// sense, and stat on a denied path must stay byte-identical to stat on
/// a missing one.
fn stat_read(
    abs: &Path,
    wire_path: &str,
    cfg: &CoderConfig,
    md: &std::fs::Metadata,
) -> Result<SingleReadOut, CoderError> {
    let (total_lines, is_utf8) = stat_counts(abs, wire_path, md, cfg.max_read_bytes)?;
    Ok(SingleReadOut {
        path: abs.display().to_string(),
        content: None,
        is_utf8,
        lines_returned: 0,
        total_lines,
        more_lines: false,
        size: md.len(),
        mode: unix_mode(md),
        mtime: unix_mtime(md),
    })
}

/// Count `total_lines` + whole-file UTF-8 validity for a stat probe,
/// reading at most `limit` bytes (the read is capped at limit+1 to
/// detect overflow — the same probe pattern `read_window` uses for
/// lines). `(None, None)` when the file exceeds `limit` (by metadata or
/// by an over-limit read after TOCTOU growth): stat on a big file still
/// SUCCEEDS for size/mode/mtime — that is its point. Line counting and
/// the UTF-8 verdict follow the full-read path exactly (`count_lines` +
/// strict validation before lossy conversion would kick in).
fn stat_counts(
    abs: &Path,
    wire_path: &str,
    md: &std::fs::Metadata,
    limit: u64,
) -> Result<(Option<u64>, Option<bool>), CoderError> {
    if md.len() > limit {
        return Ok((None, None));
    }
    let file = std::fs::File::open(abs).map_err(|e| CoderError::io_for_path(e, wire_path))?;
    let mut bytes = Vec::new();
    let mut bounded = std::io::Read::take(file, limit.saturating_add(1));
    std::io::Read::read_to_end(&mut bounded, &mut bytes)
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    if bytes.len() as u64 > limit {
        return Ok((None, None));
    }
    let total = count_lines(&bytes);
    let is_utf8 = std::str::from_utf8(&bytes).is_ok();
    Ok((Some(total), Some(is_utf8)))
}

/// Windowed read (single-path mode): stream lines `from..=to` via
/// `BufReader`. The `max_read_bytes` cap bounds the COLLECTED window's
/// RAW bytes — the T7 contract — never the file size (windowed mode
/// never returns C213 for an oversize file).
fn windowed_read(
    abs: &Path,
    wire_path: &str,
    cfg: &CoderConfig,
    md: &std::fs::Metadata,
    from: u64,
    to: Option<u64>,
    numbered: bool,
) -> Result<SingleReadOut, CoderError> {
    let file = std::fs::File::open(abs).map_err(|e| CoderError::io_for_path(e, wire_path))?;
    let mut reader = std::io::BufReader::new(file);
    let w = read_window(&mut reader, from, to, cfg.max_read_bytes, numbered)
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    let (content, is_utf8) = lossy_utf8(w.raw);
    Ok(SingleReadOut {
        path: abs.display().to_string(),
        content: Some(content),
        is_utf8: Some(is_utf8),
        lines_returned: w.lines_returned,
        total_lines: w.total_lines,
        more_lines: w.more_lines,
        size: md.len(),
        mode: unix_mode(md),
        mtime: unix_mtime(md),
    })
}

/// Windowed read (batch mode): identical streaming/no-torn-lines
/// machinery, but the budget is measured in CONVERTED WIRE BYTES
/// (`content.len()` after UTF-8 sanitization) — the unit
/// `batch_read_budget_bytes` is defined in, so the aggregate cap bounds
/// what the caller's context actually receives even for binary files
/// whose invalid bytes expand to 3-byte U+FFFD replacements.
fn wire_windowed_read(
    abs: &Path,
    wire_path: &str,
    wire_budget: u64,
    md: &std::fs::Metadata,
    from: u64,
    to: Option<u64>,
    numbered: bool,
) -> Result<SingleReadOut, CoderError> {
    let file = std::fs::File::open(abs).map_err(|e| CoderError::io_for_path(e, wire_path))?;
    let mut reader = std::io::BufReader::new(file);
    let w = read_window_wire(&mut reader, from, to, wire_budget, numbered)
        .map_err(|e| CoderError::io_for_path(e, wire_path))?;
    Ok(SingleReadOut {
        path: abs.display().to_string(),
        content: Some(w.content),
        is_utf8: Some(w.is_utf8),
        lines_returned: w.lines_returned,
        total_lines: w.total_lines,
        more_lines: w.more_lines,
        size: md.len(),
        mode: unix_mode(md),
        mtime: unix_mtime(md),
    })
}

// ---------------------------------------------------------------------------
// Batch mode
// ---------------------------------------------------------------------------

/// A failed batch entry: every content/metadata field null, `error` set.
fn entry_failure(path: String, error: WireError) -> ReadEntryResult {
    ReadEntryResult {
        path,
        success: false,
        content: None,
        is_utf8: None,
        lines_returned: None,
        total_lines: None,
        more_lines: None,
        size: None,
        mode: None,
        mtime: None,
        error: Some(error),
    }
}

/// Process `targets` in request order against the aggregate
/// `batch_read_budget_bytes` cap. The budget unit is CONVERTED WIRE
/// BYTES (each entry's `content.len()`), so what is accounted is exactly
/// what is delivered.
///
/// ORDERING IS LOAD-BEARING (REDACTION INVARIANT): resolve, stat, and the
/// regular-file check all run BEFORE the zero-budget check, so error
/// classification never depends on budget state. Otherwise an agent
/// could exhaust the budget and then distinguish a missing path (which
/// would hit the budget C213) from a glob-denied one (C211 at resolve).
fn batch_read(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    base_dir: Option<&str>,
    targets: &[ReadTarget],
) -> Vec<ReadEntryResult> {
    let mut remaining_budget: u64 = cfg.batch_read_budget_bytes;
    let mut results = Vec::with_capacity(targets.len());

    for target in targets {
        let wire_path = target.path();
        let (lf, lt) = target.window_params();
        let (stat, numbered) = target.flags();

        // Per-entry C210 for invalid window params before touching anything.
        let window = match parse_window(lf, lt) {
            Ok(w) => w,
            Err(e) => {
                results.push(entry_failure(wire_path.to_string(), e.to_wire_error()));
                continue;
            }
        };
        // Per-entry C210 for stat conflicts — same rules as single-path.
        if stat && window.is_some() {
            results.push(entry_failure(
                wire_path.to_string(),
                stat_conflict("line_from/line_to").to_wire_error(),
            ));
            continue;
        }
        if stat && numbered {
            results.push(entry_failure(
                wire_path.to_string(),
                stat_conflict("numbered").to_wire_error(),
            ));
            continue;
        }

        // Resolve + accessibility check; failures echo the caller's input
        // verbatim and do NOT consume budget.
        let abs = match resolver.require_writable_opt(base_dir, wire_path) {
            Ok(p) => p,
            Err(e) => {
                results.push(entry_failure(wire_path.to_string(), e.to_wire_error()));
                continue;
            }
        };

        // Stat BEFORE the zero-budget check (see fn docs).
        let md = match std::fs::metadata(&abs) {
            Ok(m) => m,
            Err(e) => {
                // NotFound folds to C211 and echoes the wire path VERBATIM —
                // byte-indistinguishable from the glob-denied arm above
                // (REDACTION INVARIANT). Other io errors (EIO, permission
                // TOCTOU) echo the canonical path: resolution succeeded, so
                // the canonical form is redaction-safe and more actionable.
                let echo = if e.kind() == std::io::ErrorKind::NotFound {
                    wire_path.to_string()
                } else {
                    abs.display().to_string()
                };
                let err = CoderError::io_for_path(e, wire_path);
                results.push(entry_failure(echo, err.to_wire_error()));
                continue;
            }
        };
        if !md.is_file() {
            results.push(entry_failure(
                abs.display().to_string(),
                CoderError::BadInput(format!("not a regular file: {wire_path}")).to_wire_error(),
            ));
            continue;
        }

        // Stat probe: metadata only, no content — consumes no budget and
        // is deliberately exempt from the zero-budget C213 below (stat is
        // the cheap probe; an exhausted batch can still size files).
        // Resolve + deny + metadata already ran, so classification stays
        // budget-independent (REDACTION INVARIANT).
        if stat {
            match stat_read(&abs, wire_path, cfg, &md) {
                Ok(r) => results.push(ReadEntryResult {
                    path: r.path,
                    success: true,
                    content: None,
                    is_utf8: r.is_utf8,
                    lines_returned: Some(0),
                    total_lines: r.total_lines,
                    more_lines: Some(false),
                    size: Some(r.size),
                    mode: Some(r.mode),
                    mtime: Some(r.mtime),
                    error: None,
                }),
                Err(e) => results.push(entry_failure(abs.display().to_string(), e.to_wire_error())),
            }
            continue;
        }

        // Accounted consumption so far, derived from the running budget.
        // Computed BEFORE the guard so it stays correct if the guard
        // condition ever changes; with today's `== 0` condition it is
        // tautologically the full budget.
        let consumed = cfg.batch_read_budget_bytes - remaining_budget;

        // Zero-budget check — only an existing, accessible regular file
        // can reach this point, so C213 leaks nothing about protected or
        // missing paths. The message reports the ACTUAL accounted
        // consumption.
        if remaining_budget == 0 {
            results.push(entry_failure(
                abs.display().to_string(),
                CoderError::TooLarge(format!(
                    "batch budget exhausted before reaching {wire_path}: \
                     batch_read_budget_bytes is {} and earlier entries already \
                     returned {consumed} bytes of content (after UTF-8 \
                     sanitization). To recover: request fewer or smaller entries, \
                     use per-entry line_from/line_to windows, or raise \
                     batch_read_budget_bytes in coder config.",
                    cfg.batch_read_budget_bytes,
                ))
                .to_wire_error(),
            ));
            continue;
        }

        // Effective per-entry budget: min(remaining, max_read_bytes),
        // applied to the entry's converted wire bytes. A string target is
        // a window from line 1 to EOF, so the no-torn-lines machinery
        // applies uniformly.
        let entry_budget = remaining_budget.min(cfg.max_read_bytes);
        let (from, to) = window.unwrap_or((1, None));

        match wire_windowed_read(&abs, wire_path, entry_budget, &md, from, to, numbered) {
            Ok(r) => {
                // Accounted consumption == delivered wire bytes (numbered
                // prefixes included): the collection above already counted
                // converted lengths.
                let delivered = r.content.as_ref().map_or(0, String::len) as u64;
                remaining_budget = remaining_budget.saturating_sub(delivered);
                results.push(ReadEntryResult {
                    path: r.path,
                    success: true,
                    content: r.content,
                    is_utf8: r.is_utf8,
                    lines_returned: Some(r.lines_returned),
                    total_lines: r.total_lines,
                    more_lines: Some(r.more_lines),
                    size: Some(r.size),
                    mode: Some(r.mode),
                    mtime: Some(r.mtime),
                    error: None,
                });
            }
            Err(e) => {
                // IO/other error — budget not consumed; resolution
                // succeeded, so the canonical echo is redaction-safe.
                results.push(entry_failure(abs.display().to_string(), e.to_wire_error()));
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Platform helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn unix_mode(md: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn unix_mode(_md: &std::fs::Metadata) -> u32 {
    0o644
}

fn unix_mtime(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        setup_with_cap(1024)
    }

    /// Jail with a custom `max_read_bytes` (window byte-budget tests).
    fn setup_with_cap(cap: u64) -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: cap,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    fn setup_with_batch_budget(
        batch_budget: u64,
    ) -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024, // 1 MiB per-entry cap — not the constraint
            batch_read_budget_bytes: batch_budget,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    fn full(path: &str) -> ReadFileInput {
        ReadFileInput {
            path: Some(path.into()),
            ..ReadFileInput::default()
        }
    }

    fn window_req(path: &str, from: Option<u64>, to: Option<u64>) -> ReadFileInput {
        ReadFileInput {
            path: Some(path.into()),
            line_from: from,
            line_to: to,
            ..ReadFileInput::default()
        }
    }

    fn batch(paths: Vec<ReadTarget>) -> ReadFileInput {
        ReadFileInput {
            paths: Some(paths),
            ..ReadFileInput::default()
        }
    }

    fn stat_req(path: &str) -> ReadFileInput {
        ReadFileInput {
            path: Some(path.into()),
            stat: true,
            ..ReadFileInput::default()
        }
    }

    /// Object-form batch target with only the window fields set.
    fn target_window(path: &str, from: Option<u64>, to: Option<u64>) -> ReadTarget {
        ReadTarget::Window {
            path: path.into(),
            line_from: from,
            line_to: to,
            stat: false,
            numbered: false,
        }
    }

    /// Object-form batch target with only the stat flag set.
    fn target_stat(path: &str) -> ReadTarget {
        ReadTarget::Window {
            path: path.into(),
            line_from: None,
            line_to: None,
            stat: true,
            numbered: false,
        }
    }

    /// Object-form batch target with only the numbered flag set.
    fn target_numbered(path: &str) -> ReadTarget {
        ReadTarget::Window {
            path: path.into(),
            line_from: None,
            line_to: None,
            stat: false,
            numbered: true,
        }
    }

    /// "L1\n" .. "L<n>\n" (trailing newline).
    fn numbered_lines(n: u64) -> String {
        (1..=n).map(|i| format!("L{i}\n")).collect()
    }

    fn unwrap_single(out: ReadFileOutput) -> (String, String, bool, u64, Option<u64>, bool) {
        (
            out.path.unwrap(),
            out.content.unwrap(),
            out.is_utf8.unwrap(),
            out.lines_returned.unwrap(),
            out.total_lines,
            out.more_lines.unwrap(),
        )
    }

    // -----------------------------------------------------------------------
    // XOR input validation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn both_path_and_paths_returns_c210() {
        let (_tmp, r, c) = setup();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            paths: Some(vec![ReadTarget::Path("f.txt".into())]),
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(
            err.contains("not both"),
            "error must name the XOR rule: {err}"
        );
    }

    #[tokio::test]
    async fn neither_path_nor_paths_returns_c210() {
        let (_tmp, r, c) = setup();
        let req = ReadFileInput::default();
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    // -----------------------------------------------------------------------
    // Single-path mode — regression (unchanged from T7)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reads_existing_file() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("hi.txt"), b"hello").unwrap();
        let out = handle(r, c, full("hi.txt")).await.unwrap();
        let (path, content, is_utf8, lines, total, more) = unwrap_single(out);
        assert_eq!(content, "hello");
        assert!(is_utf8);
        assert_eq!(lines, 1);
        assert_eq!(total, Some(1));
        assert!(!more);
        assert_eq!(
            path,
            std::fs::canonicalize(tmp.path())
                .unwrap()
                .join("hi.txt")
                .display()
                .to_string()
        );
    }

    #[tokio::test]
    async fn refuses_non_accessible() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), b"secret").unwrap();
        let err = handle(r, c, full(".env")).await.unwrap_err();
        assert!(err.contains("C211"), "got: {err}");
    }

    #[tokio::test]
    async fn refuses_file_above_max_read_bytes_and_hints_window_params() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("big.bin"), vec![0u8; 2048]).unwrap();
        let err = handle(r, c, full("big.bin")).await.unwrap_err();
        assert!(err.contains("C213"), "got: {err}");
        assert!(err.contains("line_from"), "got: {err}");
        assert!(err.contains("line_to"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_directory_with_bad_input() {
        let (tmp, r, c) = setup();
        std::fs::create_dir(tmp.path().join("d")).unwrap();
        let err = handle(r, c, full("d")).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    #[tokio::test]
    async fn missing_file_returns_c211() {
        let (_tmp, r, c) = setup();
        let err = handle(r, c, full("nope.txt")).await.unwrap_err();
        assert!(err.contains("C211"), "got: {err}");
    }

    // -----------------------------------------------------------------------
    // T7 — windowed reads (single-path, regression)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn window_in_range_returns_lines_and_counters() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), numbered_lines(10)).unwrap();
        let out = handle(r, c, window_req("f.txt", Some(3), Some(5)))
            .await
            .unwrap();
        let (_, content, is_utf8, lines, total, more) = unwrap_single(out);
        assert_eq!(content, "L3\nL4\nL5\n");
        assert_eq!(lines, 3);
        assert!(more);
        assert_eq!(total, None);
        assert!(is_utf8);
    }

    #[tokio::test]
    async fn window_from_only_reads_to_eof_and_knows_total() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), numbered_lines(10)).unwrap();
        let out = handle(r, c, window_req("f.txt", Some(8), None))
            .await
            .unwrap();
        let (_, content, _, lines, total, more) = unwrap_single(out);
        assert_eq!(content, "L8\nL9\nL10\n");
        assert_eq!(lines, 3);
        assert!(!more);
        assert_eq!(total, Some(10));
    }

    #[tokio::test]
    async fn window_to_only_reads_from_line_one() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), numbered_lines(10)).unwrap();
        let out = handle(r, c, window_req("f.txt", None, Some(2)))
            .await
            .unwrap();
        let (_, content, _, lines, total, more) = unwrap_single(out);
        assert_eq!(content, "L1\nL2\n");
        assert_eq!(lines, 2);
        assert!(more);
        assert_eq!(total, None);
    }

    #[tokio::test]
    async fn window_past_eof_is_success_with_total() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), numbered_lines(10)).unwrap();
        let out = handle(r, c, window_req("f.txt", Some(50), Some(60)))
            .await
            .unwrap();
        let (_, content, _, lines, total, more) = unwrap_single(out);
        assert_eq!(content, "");
        assert_eq!(lines, 0);
        assert!(!more);
        assert_eq!(total, Some(10));
    }

    #[tokio::test]
    async fn line_from_zero_rejected_with_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\n").unwrap();
        let err = handle(r, c, window_req("f.txt", Some(0), Some(3)))
            .await
            .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(err.contains("1-based"), "must name the rule: {err}");
    }

    #[tokio::test]
    async fn inverted_window_rejected_with_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let err = handle(r, c, window_req("f.txt", Some(5), Some(3)))
            .await
            .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(err.contains("line_to"), "must name the rule: {err}");
    }

    #[tokio::test]
    async fn to_only_zero_rejected_as_inverted() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\n").unwrap();
        let err = handle(r, c, window_req("f.txt", None, Some(0)))
            .await
            .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    #[tokio::test]
    async fn window_exceeding_byte_cap_returns_partial_with_more_lines() {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            max_read_bytes: 10,
            ..CoderConfig::default()
        });
        let r = Arc::new(PathResolver::new(&cfg).unwrap());
        std::fs::write(tmp.path().join("f.txt"), "aaaa\naaaa\naaaa\naaaa\n").unwrap();
        let out = handle(r, cfg, window_req("f.txt", Some(1), Some(4)))
            .await
            .unwrap();
        let (_, content, _, lines, total, more) = unwrap_single(out);
        assert_eq!(content, "aaaa\naaaa\n");
        assert_eq!(lines, 2);
        assert!(more, "byte-budget cut must set more_lines");
        assert_eq!(total, None);
    }

    #[tokio::test]
    async fn single_line_exceeding_byte_cap_returns_empty_partial() {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            max_read_bytes: 4,
            ..CoderConfig::default()
        });
        let r = Arc::new(PathResolver::new(&cfg).unwrap());
        std::fs::write(tmp.path().join("f.txt"), "aaaaaaaa\nb\n").unwrap();
        let out = handle(r, cfg, window_req("f.txt", Some(1), Some(2)))
            .await
            .unwrap();
        let (_, content, _, lines, _, more) = unwrap_single(out);
        assert_eq!(content, "");
        assert_eq!(lines, 0);
        assert!(more);
    }

    // -----------------------------------------------------------------------
    // Batch mode — ReadTarget parsing
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_string_target_reads_whole_file() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "hello\n").unwrap();
        let out = handle(r, c, batch(vec![ReadTarget::Path("a.txt".into())]))
            .await
            .unwrap();
        assert!(
            out.path.is_none(),
            "single-path field must be null in batch"
        );
        let results = out.results.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].content.as_deref(), Some("hello\n"));
    }

    #[tokio::test]
    async fn batch_object_target_with_window() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("b.txt"), numbered_lines(5)).unwrap();
        let out = handle(r, c, batch(vec![target_window("b.txt", Some(2), Some(3))]))
            .await
            .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert_eq!(results[0].content.as_deref(), Some("L2\nL3\n"));
        assert_eq!(results[0].lines_returned, Some(2));
    }

    // -----------------------------------------------------------------------
    // Batch mode — order preservation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_results_in_request_order() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("x.txt"), "X\n").unwrap();
        std::fs::write(tmp.path().join("y.txt"), "Y\n").unwrap();
        std::fs::write(tmp.path().join("z.txt"), "Z\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("x.txt".into()),
                ReadTarget::Path("y.txt".into()),
                ReadTarget::Path("z.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert_eq!(results[0].content.as_deref(), Some("X\n"));
        assert_eq!(results[1].content.as_deref(), Some("Y\n"));
        assert_eq!(results[2].content.as_deref(), Some("Z\n"));
    }

    // -----------------------------------------------------------------------
    // Batch mode — per-entry failures don't consume budget
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_per_entry_c211_missing_does_not_consume_budget() {
        let (tmp, r, c) = setup_with_batch_budget(20); // 20 bytes total
        std::fs::write(tmp.path().join("ok.txt"), "hello\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("missing.txt".into()), // fails — no budget consumed
                ReadTarget::Path("ok.txt".into()),      // succeeds
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(!results[0].success, "missing entry must fail");
        let wire = results[0].error.as_ref().unwrap();
        assert_eq!(wire.code, "C211");
        assert!(results[1].success, "next entry must succeed");
        assert_eq!(results[1].content.as_deref(), Some("hello\n"));
    }

    #[tokio::test]
    async fn batch_per_entry_c211_glob_denied_does_not_consume_budget() {
        let (tmp, r, c) = setup_with_batch_budget(20);
        std::fs::write(tmp.path().join(".env"), "secret").unwrap();
        std::fs::write(tmp.path().join("ok.txt"), "hello\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path(".env".into()),   // denied — no budget consumed
                ReadTarget::Path("ok.txt".into()), // succeeds
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(!results[0].success);
        assert_eq!(results[0].error.as_ref().unwrap().code, "C211");
        assert!(results[1].success);
    }

    // -----------------------------------------------------------------------
    // Batch mode — budget partial (more_lines mid-entry)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_budget_partial_entry_has_more_lines_true() {
        // Budget: 10 bytes. Each line is 5 bytes. First entry consumes 10
        // bytes (2 lines). Second entry has zero budget → C213.
        let (tmp, r, c) = setup_with_batch_budget(10);
        std::fs::write(tmp.path().join("a.txt"), "aaaa\nbbbb\ncccc\n").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "data\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("a.txt".into()),
                ReadTarget::Path("b.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        // First entry: 2 lines fit (10 bytes), 3rd line cut → more_lines=true
        assert!(results[0].success);
        assert_eq!(results[0].more_lines, Some(true));
        assert_eq!(results[0].lines_returned, Some(2));
        // Second entry: zero budget → C213
        assert!(!results[1].success);
        let wire = results[1].error.as_ref().unwrap();
        assert_eq!(wire.code, "C213");
        assert!(
            wire.message.contains("batch_read_budget_bytes"),
            "C213 must name the config key: {}",
            wire.message
        );
        assert!(
            wire.message.contains("10"),
            "C213 must name the budget value: {}",
            wire.message
        );
    }

    // -----------------------------------------------------------------------
    // Batch mode — zero-budget C213 details
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_zero_budget_entry_c213_names_key_and_value() {
        let (tmp, r, c) = setup_with_batch_budget(5);
        // First file: 5 bytes, consumes entire budget.
        std::fs::write(tmp.path().join("first.txt"), "abcde").unwrap();
        std::fs::write(tmp.path().join("second.txt"), "x").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("first.txt".into()),
                ReadTarget::Path("second.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert!(!results[1].success);
        let wire = results[1].error.as_ref().unwrap();
        assert_eq!(wire.code, "C213");
        assert!(wire.message.contains("batch_read_budget_bytes"));
        assert!(wire.message.contains('5'), "must name the value");
        // Recovery guidance
        assert!(
            wire.message.contains("line_from") || wire.message.contains("raise"),
            "must include recovery guidance: {}",
            wire.message
        );
    }

    // -----------------------------------------------------------------------
    // Batch mode — tiny budget (budget < first line → empty success)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_budget_smaller_than_first_line_succeeds_with_empty_more_lines() {
        // Budget of 2 bytes; first line is "aaaaaaaa\n" (9 bytes).
        // No-torn-lines: empty content, more_lines=true — NOT a C213 error.
        let (tmp, r, c) = setup_with_batch_budget(2);
        std::fs::write(tmp.path().join("f.txt"), "aaaaaaaa\nb\n").unwrap();
        let out = handle(r, c, batch(vec![ReadTarget::Path("f.txt".into())]))
            .await
            .unwrap();
        let results = out.results.unwrap();
        assert!(
            results[0].success,
            "budget < line length is still a success per no-torn-lines"
        );
        assert_eq!(results[0].content.as_deref(), Some(""));
        assert_eq!(results[0].lines_returned, Some(0));
        assert_eq!(results[0].more_lines, Some(true));
    }

    // -----------------------------------------------------------------------
    // Batch mode — budget unit is CONVERTED wire bytes (T8 review fix 1)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_budget_counts_wire_bytes_not_raw() {
        // REVIEWER REPRO: invalid bytes expand 3x under lossy conversion
        // (1x0xFF → one 3-byte U+FFFD), so a raw-byte budget would let
        // binary entries deliver up to 3x the configured cap. Two raw
        // lines of 3x0xFF + '\n' (4 raw bytes) each convert to 10 wire
        // bytes; budget 10 → exactly one converted line fits.
        let (tmp, r, c) = setup_with_batch_budget(10);
        std::fs::write(tmp.path().join("bin.dat"), b"\xFF\xFF\xFF\n\xFF\xFF\xFF\n").unwrap();
        std::fs::write(tmp.path().join("next.txt"), "x\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("bin.dat".into()),
                ReadTarget::Path("next.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();

        // Entry 0: one converted line (exactly 10 wire bytes) fits; the
        // second would exceed the budget → partial success per
        // no-torn-lines on the CONVERTED form.
        assert!(results[0].success);
        assert_eq!(results[0].content.as_ref().unwrap().len(), 10);
        assert_eq!(results[0].lines_returned, Some(1));
        assert_eq!(results[0].more_lines, Some(true));
        assert_eq!(results[0].is_utf8, Some(false));

        // Entry 1: zero wire budget remains → C213 reporting the ACTUAL
        // accounted consumption, not a hardcoded value.
        assert!(!results[1].success);
        let wire = results[1].error.as_ref().unwrap();
        assert_eq!(wire.code, "C213");
        assert!(
            wire.message.contains("batch_read_budget_bytes is 10"),
            "C213 must name the key + value: {}",
            wire.message
        );
        assert!(
            wire.message.contains("returned 10 bytes"),
            "C213 must report actual accounted consumption: {}",
            wire.message
        );

        // INVARIANT: total delivered wire bytes never exceed the budget.
        let total: usize = results
            .iter()
            .filter_map(|e| e.content.as_ref().map(String::len))
            .sum();
        assert!(total <= 10, "delivered {total} wire bytes > budget 10");
    }

    #[tokio::test]
    async fn batch_binary_line_over_wire_budget_is_empty_success_not_torn() {
        // 10 raw 0xFF bytes = one EOF-terminated line converting to 30
        // wire bytes. Budget 10: the converted line cannot fit → empty
        // SUCCESS with more_lines=true (no-torn-lines on the converted
        // form), zero consumed — the raw accounting bug delivered all 30
        // wire bytes here, 3x the budget.
        let (tmp, r, c) = setup_with_batch_budget(10);
        std::fs::write(tmp.path().join("bin.dat"), vec![0xFFu8; 10]).unwrap();
        std::fs::write(tmp.path().join("ok.txt"), "hi\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("bin.dat".into()),
                ReadTarget::Path("ok.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert_eq!(results[0].content.as_deref(), Some(""));
        assert_eq!(results[0].more_lines, Some(true));
        // Nothing delivered → nothing consumed: the next entry reads fine.
        assert!(results[1].success);
        assert_eq!(results[1].content.as_deref(), Some("hi\n"));
        let total: usize = results
            .iter()
            .filter_map(|e| e.content.as_ref().map(String::len))
            .sum();
        assert!(total <= 10, "delivered {total} wire bytes > budget 10");
    }

    // -----------------------------------------------------------------------
    // Batch mode — redaction invariant survives exhaustion (T8 review fix 2)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_post_exhaustion_missing_and_denied_indistinguishable() {
        // After the budget hits zero, a missing path and a glob-denied
        // path must BOTH return C211 with byte-identical message suffixes
        // and verbatim path echoes — C213 may only reach an existing,
        // accessible entry, or an agent could probe for protected files
        // by exhausting the budget first.
        let (tmp, r, c) = setup_with_batch_budget(5);
        std::fs::write(tmp.path().join("eat.txt"), "abcde").unwrap();
        std::fs::write(tmp.path().join(".env"), "secret").unwrap();
        std::fs::write(tmp.path().join("exists.txt"), "x").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("eat.txt".into()), // consumes the whole budget
                ReadTarget::Path("missing.txt".into()), // must be C211, NOT C213
                ReadTarget::Path(".env".into()),    // C211 (glob-denied)
                ReadTarget::Path("exists.txt".into()), // C213 — exists + accessible
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert_eq!(results[0].content.as_deref(), Some("abcde"));

        let missing = results[1].error.as_ref().unwrap();
        let denied = results[2].error.as_ref().unwrap();
        assert_eq!(
            missing.code, "C211",
            "missing after exhaustion: {missing:?}"
        );
        assert_eq!(denied.code, "C211", "denied after exhaustion: {denied:?}");
        // Byte-identical suffix after the caller-supplied path prefix
        // (T3 suffix-comparison pattern — REDACTION INVARIANT).
        let m_suffix = missing
            .message
            .strip_prefix("missing.txt: ")
            .expect("missing message starts with its wire path");
        let d_suffix = denied
            .message
            .strip_prefix(".env: ")
            .expect("denied message starts with its wire path");
        assert_eq!(
            m_suffix, d_suffix,
            "C211 missing vs glob-denied suffixes must be byte-identical"
        );
        // Both echo the caller's input verbatim (canonical echo for the
        // missing case would itself distinguish the two).
        assert_eq!(results[1].path, "missing.txt");
        assert_eq!(results[2].path, ".env");

        // Only the existing, accessible entry receives the budget C213.
        assert_eq!(results[3].error.as_ref().unwrap().code, "C213");
    }

    // -----------------------------------------------------------------------
    // Batch mode — per-entry window C210 propagates as per-entry error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_per_entry_window_c210_fails_that_entry_others_proceed() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), numbered_lines(5)).unwrap();
        std::fs::write(tmp.path().join("b.txt"), "ok\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                // line_to < line_from → C210
                target_window("a.txt", Some(5), Some(2)),
                ReadTarget::Path("b.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(!results[0].success);
        assert_eq!(results[0].error.as_ref().unwrap().code, "C210");
        assert!(results[1].success);
        assert_eq!(results[1].content.as_deref(), Some("ok\n"));
    }

    // -----------------------------------------------------------------------
    // Batch mode — empty batch succeeds with empty results
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_empty_paths_returns_empty_results() {
        let (_tmp, r, c) = setup();
        let out = handle(r, c, batch(vec![])).await.unwrap();
        assert!(out.results.unwrap().is_empty());
    }

    // -----------------------------------------------------------------------
    // S4 — stat probe (single-path)
    // -----------------------------------------------------------------------

    /// Jail with custom full-read output budget + read cap.
    fn setup_with_output_budget(
        max_output: u64,
        max_read: u64,
    ) -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: max_read,
            max_output_bytes: max_output,
            ..CoderConfig::default()
        });
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    /// Parse a top-level Err(String) wire JSON into (code, message).
    fn parse_wire(err: &str) -> (String, String) {
        let v: serde_json::Value = serde_json::from_str(err).expect("wire JSON");
        (
            v["code"].as_str().unwrap().to_string(),
            v["message"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn stat_single_returns_metadata_without_content() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("s.txt"), "hello\nworld\n").unwrap();
        let out = handle(r, c, stat_req("s.txt")).await.unwrap();
        assert!(out.content.is_none(), "stat must not return content");
        assert_eq!(out.lines_returned, Some(0));
        assert_eq!(out.more_lines, Some(false));
        assert_eq!(out.size, Some(12));
        assert_eq!(out.total_lines, Some(2));
        assert_eq!(out.is_utf8, Some(true));
        assert!(out.mode.is_some());
        assert!(out.mtime.is_some());
    }

    #[tokio::test]
    async fn stat_batch_entry_returns_metadata_without_content() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("s.txt"), "hello\nworld\n").unwrap();
        std::fs::write(tmp.path().join("t.txt"), "x\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![target_stat("s.txt"), ReadTarget::Path("t.txt".into())]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert!(results[0].content.is_none());
        assert_eq!(results[0].lines_returned, Some(0));
        assert_eq!(results[0].more_lines, Some(false));
        assert_eq!(results[0].size, Some(12));
        assert_eq!(results[0].total_lines, Some(2));
        assert_eq!(results[0].is_utf8, Some(true));
        // Next entry reads normally — stat consumed no budget.
        assert!(results[1].success);
        assert_eq!(results[1].content.as_deref(), Some("x\n"));
    }

    /// REDACTION ORDERING regression: stat resolves + deny-checks BEFORE
    /// any metadata syscall, so stat on a denied path is byte-identical
    /// to stat on a missing one.
    #[tokio::test]
    async fn stat_denied_byte_identical_to_missing_single() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), "secret-data").unwrap();
        let denied = handle(r.clone(), c.clone(), stat_req(".env"))
            .await
            .unwrap_err();
        let missing = handle(r, c, stat_req("missing.txt")).await.unwrap_err();
        let (d_code, d_msg) = parse_wire(&denied);
        let (m_code, m_msg) = parse_wire(&missing);
        assert_eq!(d_code, "C211");
        assert_eq!(m_code, "C211");
        let d_suffix = d_msg.strip_prefix(".env: ").expect("denied prefix");
        let m_suffix = m_msg.strip_prefix("missing.txt: ").expect("missing prefix");
        assert_eq!(
            d_suffix, m_suffix,
            "stat C211 suffixes must be byte-identical"
        );
    }

    #[tokio::test]
    async fn stat_denied_byte_identical_to_missing_batch_entries() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join(".env"), "secret-data").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![target_stat(".env"), target_stat("missing.txt")]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        let denied = results[0].error.as_ref().unwrap();
        let missing = results[1].error.as_ref().unwrap();
        assert_eq!(denied.code, "C211");
        assert_eq!(missing.code, "C211");
        let d_suffix = denied.message.strip_prefix(".env: ").unwrap();
        let m_suffix = missing.message.strip_prefix("missing.txt: ").unwrap();
        assert_eq!(d_suffix, m_suffix);
        // Verbatim path echoes — canonical echo would itself distinguish.
        assert_eq!(results[0].path, ".env");
        assert_eq!(results[1].path, "missing.txt");
    }

    #[tokio::test]
    async fn stat_big_file_returns_size_with_null_total_lines() {
        let (tmp, r, c) = setup_with_cap(1024);
        std::fs::write(tmp.path().join("big.bin"), vec![b'a'; 2048]).unwrap();
        let out = handle(r, c, stat_req("big.bin")).await.unwrap();
        assert_eq!(out.size, Some(2048), "stat on a big file SUCCEEDS");
        assert_eq!(out.total_lines, None, "not countable within max_read_bytes");
        assert_eq!(out.is_utf8, None, "not verifiable within max_read_bytes");
        assert!(out.content.is_none());
        assert!(out.mode.is_some());
        assert!(out.mtime.is_some());
    }

    #[tokio::test]
    async fn stat_with_window_is_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\n").unwrap();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            stat: true,
            line_from: Some(1),
            line_to: Some(2),
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(err.contains("stat"), "must name the conflict: {err}");
        assert!(err.contains("line_from"), "must name the field: {err}");
    }

    #[tokio::test]
    async fn stat_with_numbered_is_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\n").unwrap();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            stat: true,
            numbered: true,
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(err.contains("numbered"), "must name the field: {err}");
    }

    #[tokio::test]
    async fn stat_with_max_output_bytes_is_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\n").unwrap();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            stat: true,
            max_output_bytes: Some(64),
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(
            err.contains("max_output_bytes"),
            "must name the field: {err}"
        );
    }

    #[tokio::test]
    async fn batch_stat_entry_with_window_or_numbered_is_per_entry_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("a.txt"), "a\n").unwrap();
        std::fs::write(tmp.path().join("ok.txt"), "ok\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Window {
                    path: "a.txt".into(),
                    line_from: Some(1),
                    line_to: Some(1),
                    stat: true,
                    numbered: false,
                },
                ReadTarget::Window {
                    path: "a.txt".into(),
                    line_from: None,
                    line_to: None,
                    stat: true,
                    numbered: true,
                },
                ReadTarget::Path("ok.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert_eq!(results[0].error.as_ref().unwrap().code, "C210");
        assert_eq!(results[1].error.as_ref().unwrap().code, "C210");
        assert!(
            results[1]
                .error
                .as_ref()
                .unwrap()
                .message
                .contains("numbered"),
            "per-entry C210 must name the field"
        );
        assert!(results[2].success, "other entries proceed");
    }

    /// stat is the cheap probe: it stays available after the batch budget
    /// is exhausted (no content, no budget interaction) — while error
    /// classification for denied/missing paths stays C211 regardless.
    #[tokio::test]
    async fn batch_stat_entry_succeeds_after_budget_exhaustion() {
        let (tmp, r, c) = setup_with_batch_budget(5);
        std::fs::write(tmp.path().join("eat.txt"), "abcde").unwrap();
        std::fs::write(tmp.path().join("probe.txt"), "p1\np2\n").unwrap();
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("eat.txt".into()),   // consumes the whole budget
                target_stat("probe.txt"),             // still succeeds
                ReadTarget::Path("probe.txt".into()), // C213 — budget gone
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert!(results[1].success, "stat must not require budget");
        assert_eq!(results[1].total_lines, Some(2));
        assert_eq!(results[1].size, Some(6));
        assert!(results[1].content.is_none());
        assert_eq!(results[2].error.as_ref().unwrap().code, "C213");
    }

    // -----------------------------------------------------------------------
    // S4 — numbered reads
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn numbered_full_read_prefixes_absolute_lines() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            numbered: true,
            ..ReadFileInput::default()
        };
        let out = handle(r, c, req).await.unwrap();
        assert_eq!(
            out.content.as_deref(),
            Some("1\u{2192}a\n2\u{2192}b\n3\u{2192}c\n")
        );
        // Counters keep their meaning — numbering changes bytes, not lines.
        assert_eq!(out.lines_returned, Some(3));
        assert_eq!(out.total_lines, Some(3));
        assert_eq!(out.more_lines, Some(false));
    }

    #[tokio::test]
    async fn numbered_window_numbers_from_line_from() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), numbered_lines(10)).unwrap();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            line_from: Some(3),
            line_to: Some(5),
            numbered: true,
            ..ReadFileInput::default()
        };
        let out = handle(r, c, req).await.unwrap();
        assert_eq!(
            out.content.as_deref(),
            Some("3\u{2192}L3\n4\u{2192}L4\n5\u{2192}L5\n"),
            "numbering is ABSOLUTE: starts at line_from, not 1"
        );
        assert_eq!(out.lines_returned, Some(3));
    }

    #[tokio::test]
    async fn numbered_prefix_charged_to_batch_budget() {
        // a.txt prefixed = "1→aaaa\n2→bbbb\n" = 18 wire bytes, exactly the
        // budget; unprefixed it is only 10 and b.txt would still fit. The
        // prefix bytes must consume the budget → b.txt gets C213.
        let (tmp, r, c) = setup_with_batch_budget(18);
        std::fs::write(tmp.path().join("a.txt"), "aaaa\nbbbb\n").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "x\n").unwrap();
        let out = handle(
            r.clone(),
            c.clone(),
            batch(vec![
                target_numbered("a.txt"),
                ReadTarget::Path("b.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success);
        assert_eq!(
            results[0].content.as_deref(),
            Some("1\u{2192}aaaa\n2\u{2192}bbbb\n")
        );
        assert!(!results[1].success, "prefix bytes must consume budget");
        assert_eq!(results[1].error.as_ref().unwrap().code, "C213");

        // Control: the same batch unprefixed fits both entries.
        let out = handle(
            r,
            c,
            batch(vec![
                ReadTarget::Path("a.txt".into()),
                ReadTarget::Path("b.txt".into()),
            ]),
        )
        .await
        .unwrap();
        let results = out.results.unwrap();
        assert!(results[0].success && results[1].success);
    }

    #[tokio::test]
    async fn numbered_full_read_prefixes_converted_lossy_lines() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("bin.dat"), b"\xFF\xFF\nok\n").unwrap();
        let req = ReadFileInput {
            path: Some("bin.dat".into()),
            numbered: true,
            ..ReadFileInput::default()
        };
        let out = handle(r, c, req).await.unwrap();
        assert_eq!(
            out.content.as_deref(),
            Some("1\u{2192}\u{FFFD}\u{FFFD}\n2\u{2192}ok\n"),
            "prefix rides on the CONVERTED line"
        );
        assert_eq!(out.is_utf8, Some(false));
    }

    // -----------------------------------------------------------------------
    // S4 — full-read context budget (max_output_bytes)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn full_read_over_output_budget_returns_recovery_c213() {
        let (tmp, r, c) = setup_with_output_budget(10, 1024);
        std::fs::write(tmp.path().join("big.txt"), "aaaa\nbbbb\ncccc\ndddd\n").unwrap();
        let err = handle(r, c, full("big.txt")).await.unwrap_err();
        let (code, msg) = parse_wire(&err);
        assert_eq!(code, "C213");
        // The message is itself the recovery tool: size, total_lines, the
        // config key + per-call override, and every corrective call.
        assert!(msg.contains("20 bytes"), "must carry file size: {msg}");
        assert!(msg.contains("4 lines"), "must carry total_lines: {msg}");
        assert!(
            msg.matches("max_output_bytes").count() >= 2,
            "must name the config key AND the per-call override: {msg}"
        );
        assert!(msg.contains("line_from"), "window guidance: {msg}");
        assert!(msg.contains("stat: true"), "stat guidance: {msg}");
    }

    #[tokio::test]
    async fn full_read_at_default_budget_boundary() {
        // 128 KiB exactly passes; one byte more fails — under the DEFAULT
        // config (no custom budget), pinning the 131072 default.
        let (tmp, r, c) = setup_with_cap(10 * 1024 * 1024);
        std::fs::write(tmp.path().join("fits.txt"), vec![b'a'; 131_072]).unwrap();
        std::fs::write(tmp.path().join("over.txt"), vec![b'a'; 131_073]).unwrap();
        let out = handle(r.clone(), c.clone(), full("fits.txt"))
            .await
            .unwrap();
        assert_eq!(out.content.unwrap().len(), 131_072);
        let err = handle(r, c, full("over.txt")).await.unwrap_err();
        assert!(err.contains("C213"), "got: {err}");
        assert!(err.contains("max_output_bytes"), "got: {err}");
    }

    #[tokio::test]
    async fn per_call_max_output_bytes_admits_larger_read() {
        let (tmp, r, c) = setup_with_output_budget(10, 1024);
        std::fs::write(tmp.path().join("big.txt"), "aaaa\nbbbb\ncccc\ndddd\n").unwrap();
        let req = ReadFileInput {
            path: Some("big.txt".into()),
            max_output_bytes: Some(100),
            ..ReadFileInput::default()
        };
        let out = handle(r, c, req).await.unwrap();
        assert_eq!(out.content.unwrap().len(), 20);
        assert_eq!(out.total_lines, Some(4));
    }

    #[tokio::test]
    async fn per_call_max_output_bytes_clamps_to_max_read_bytes() {
        // File: 8 invalid bytes + '\n' = 9 raw bytes (under max_read_bytes
        // 15) but 25 CONVERTED wire bytes. Per-call budget 1000 silently
        // clamps to max_read_bytes (15) → 25 > 15 → C213. The config
        // budget (1000) alone would have admitted it — the clamp applies
        // to the per-call override.
        let (tmp, r, c) = setup_with_output_budget(1000, 15);
        let mut body = vec![0xFFu8; 8];
        body.push(b'\n');
        std::fs::write(tmp.path().join("bin.dat"), body).unwrap();
        let out = handle(r.clone(), c.clone(), full("bin.dat")).await.unwrap();
        assert_eq!(
            out.content.unwrap().len(),
            25,
            "config budget admits the read"
        );
        let req = ReadFileInput {
            path: Some("bin.dat".into()),
            max_output_bytes: Some(1000),
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C213"), "clamped per-call must refuse: {err}");
    }

    #[tokio::test]
    async fn windowed_read_unaffected_by_output_budget() {
        // The same file whose FULL read exceeds max_output_bytes streams
        // fine through a window — windows are governed by max_read_bytes.
        let (tmp, r, c) = setup_with_output_budget(10, 1024);
        std::fs::write(tmp.path().join("big.txt"), "aaaa\nbbbb\ncccc\ndddd\n").unwrap();
        let err = handle(r.clone(), c.clone(), full("big.txt"))
            .await
            .unwrap_err();
        assert!(err.contains("C213"));
        let out = handle(r, c, window_req("big.txt", Some(1), Some(4)))
            .await
            .unwrap();
        assert_eq!(out.content.as_deref(), Some("aaaa\nbbbb\ncccc\ndddd\n"));
    }

    #[tokio::test]
    async fn max_output_bytes_with_window_is_c210() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("f.txt"), "a\nb\n").unwrap();
        let req = ReadFileInput {
            path: Some("f.txt".into()),
            line_from: Some(1),
            line_to: Some(2),
            max_output_bytes: Some(64),
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(err.contains("max_output_bytes"), "got: {err}");
    }

    /// REDACTION ORDERING regression: resolve → deny (C211) → size → budget
    /// (C213). A denied or missing path must classify C211 no matter how
    /// the budget relates to the file.
    #[tokio::test]
    async fn denied_huge_file_is_c211_not_c213() {
        let (tmp, r, c) = setup_with_output_budget(10, 1024);
        std::fs::write(tmp.path().join(".env"), "aaaa\nbbbb\ncccc\ndddd\n").unwrap();
        let err = handle(r, c, full(".env")).await.unwrap_err();
        let (code, _) = parse_wire(&err);
        assert_eq!(code, "C211", "deny must precede budget: {err}");
    }

    #[tokio::test]
    async fn missing_file_with_budget_override_is_c211() {
        let (_tmp, r, c) = setup_with_output_budget(10, 1024);
        let req = ReadFileInput {
            path: Some("missing.txt".into()),
            max_output_bytes: Some(5),
            ..ReadFileInput::default()
        };
        let err = handle(r, c, req).await.unwrap_err();
        let (code, _) = parse_wire(&err);
        assert_eq!(code, "C211", "budget must never reclassify C211: {err}");
    }
}
