//! `coder::search` — combined path + content search.
//!
//! Walks the resolved folder with `walkdir`, filtering by include/exclude
//! globs (matched against the path relative to its containing root) and
//! skipping non-accessible files entirely so the search can't reveal
//! their content. Noise paths matching `default_exclude_globs` are also
//! skipped by default — descent into matching directories is suppressed
//! and matching files are omitted; opt out per call with
//! `use_default_excludes: false`. That filter is hide-only and
//! independent of the `is_non_accessible` access control (REDACTION
//! INVARIANT) — never merge the two. Path matches and content matches are
//! reported in separate arrays of one response; result paths are
//! canonical-absolute.
//!
//! Each content match can carry `before`/`after` context lines (same
//! file only, capped at [`CONTEXT_LINES_CAP`]). The whole response is
//! bounded by `search_response_budget_bytes`, accounted in converted
//! wire bytes like `batch_read_budget_bytes`: when the next match would
//! exceed the budget, accumulation stops and `truncated` is set — the
//! search degrades, it never fails.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::path::PathResolver;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_search_input")]
pub struct SearchInput {
    /// Pattern to search for. Treated as a regex when `regex: true`,
    /// otherwise as a literal substring.
    pub query: String,
    /// Folder scoping the walk. Relative to the primary allowed root, or an
    /// absolute path inside any allowed root. Defaults to `.` (the primary
    /// root itself). Globs are matched relative to the containing root;
    /// result paths are absolute regardless of this value. Call
    /// `coder::info` to see the allowed roots. Paths outside every allowed
    /// root are rejected — use the shell worker's `shell::fs::*` for host
    /// paths outside the jail.
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub ignore_case: bool,
    /// Glob patterns (matched against the path relative to its containing
    /// root) that paths must match to be considered. Empty = include
    /// everything.
    #[serde(default)]
    pub include_globs: Vec<String>,
    /// Glob patterns (same relative-to-root matching) that exclude paths.
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    /// Optional explicit cap. Falls back to config when unset.
    #[serde(default)]
    pub max_matches: Option<u32>,
    /// Bytes per line to consider when scanning content; longer lines are
    /// truncated for the match snippet.
    #[serde(default)]
    pub max_line_bytes: Option<u32>,
    /// Lines of context to return BEFORE each content match (same file
    /// only, in file order), max 10 — larger values are rejected with
    /// C210. With context lines many edit tasks can go straight from
    /// search output to coder::update-file with no read in between.
    /// Context lines are truncated to `max_line_bytes` like the matched
    /// text and count toward the response byte budget. Unset = 0.
    #[serde(default)]
    pub context_lines_before: Option<u32>,
    /// Lines of context to return AFTER each content match. Same max
    /// (10), truncation, and budget rules as `context_lines_before`;
    /// unset = 0.
    #[serde(default)]
    pub context_lines_after: Option<u32>,
    /// Apply the worker's `default_exclude_globs` config (noise paths
    /// like .git, node_modules, target, dist — call coder::info for the
    /// active list): the walk does not descend into matching directories
    /// and matching files are omitted from BOTH content and path
    /// results. Pass `false` to search inside them.
    #[serde(default = "default_true")]
    pub use_default_excludes: bool,
    /// Search file contents (default true).
    #[serde(default = "default_true")]
    pub search_content: bool,
    /// Search file paths (default true).
    #[serde(default = "default_true")]
    pub search_paths: bool,
    /// Optional per-call session working directory. When set, a relative
    /// `path` anchors here instead of the primary allowed root, and the
    /// walk root must stay inside it. `base_dir` itself must canonicalize
    /// inside an allowed root (`coder::info` lists them). Result paths stay
    /// absolute. Omit to resolve against the primary allowed root exactly as
    /// before.
    #[serde(default)]
    pub base_dir: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_path() -> String {
    ".".to_string()
}

// examples are wire-contract; goldens pin them.
fn example_search_input() -> serde_json::Value {
    serde_json::json!({
        "query": "fn handle",
        "path": "src",
        "include_globs": ["**/*.rs"],
        "context_lines_before": 2,
        "context_lines_after": 2,
        "search_content": true,
        "search_paths": false
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ContentMatch {
    /// Absolute path under the canonical parent; symlinks at the entry
    /// itself are not resolved. Operations on it re-validate through the
    /// jail.
    pub path: String,
    pub line: u32,
    pub column: u32,
    /// Matched line; truncated to `max_line_bytes` and never spans newlines.
    pub text: String,
    /// Context lines immediately before the matched line — same file
    /// only, in file order, each truncated to `max_line_bytes`. Omitted
    /// when empty (no `context_lines_before` requested, or the match is
    /// at the start of the file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Vec<String>>,
    /// Context lines immediately after the matched line — same file
    /// only, in file order, each truncated to `max_line_bytes`. Omitted
    /// when empty (no `context_lines_after` requested, or the match is
    /// at the end of the file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PathMatch {
    /// Absolute path under the canonical parent; symlinks at the entry
    /// itself are not resolved. Operations on it re-validate through the
    /// jail.
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchOutput {
    pub content_matches: Vec<ContentMatch>,
    pub path_matches: Vec<PathMatch>,
    /// True if results were cut off — either match list hit the
    /// `max_matches` cap, or the response hit the
    /// `search_response_budget_bytes` byte budget. When true, refine the
    /// query or add include_globs rather than paginate.
    pub truncated: bool,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: SearchInput,
) -> Result<SearchOutput, String> {
    // Offload the synchronous recursive content/path scan to a blocking thread
    // so a large search can't stall the shared runtime (shell::exec/jobs/reload).
    tokio::task::spawn_blocking(move || inner(&resolver, &cfg, req).map_err(err_to_string))
        .await
        .map_err(|e| format!("search task join failed: {e}"))?
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: SearchInput,
) -> Result<SearchOutput, CoderError> {
    if req.query.is_empty() {
        return Err(CoderError::BadInput("query must not be empty".into()));
    }
    if !req.search_content && !req.search_paths {
        return Err(CoderError::BadInput(
            "at least one of search_content / search_paths must be true".into(),
        ));
    }
    let max_matches = req.max_matches.unwrap_or(cfg.search_default_max_matches) as usize;
    let max_line_bytes = req
        .max_line_bytes
        .unwrap_or(cfg.search_default_max_line_bytes) as usize;
    let ctx_before = validate_context_lines("context_lines_before", req.context_lines_before)?;
    let ctx_after = validate_context_lines("context_lines_after", req.context_lines_after)?;

    // Use `resolve` rather than `require_writable` so a search rooted at
    // a folder that *contains* non-accessible children still works; the
    // per-file `is_non_accessible` filter below still guards their bytes.
    let walk_root = resolver.resolve_opt(req.base_dir.as_deref(), &req.path)?;
    // NotFound is intercepted with the wire path in scope so the C211
    // message names the path the caller supplied (standardized wording —
    // REDACTION INVARIANT: identical to the glob-denied message).
    let md = std::fs::metadata(&walk_root).map_err(|e| CoderError::io_for_path(e, &req.path))?;
    if !md.is_dir() {
        return Err(CoderError::BadInput(format!(
            "not a directory: {}",
            req.path
        )));
    }

    let include = build_globset(&req.include_globs)?;
    let exclude = build_globset(&req.exclude_globs)?;

    let content_matcher = if req.search_content {
        Some(build_matcher(&req.query, req.regex, req.ignore_case)?)
    } else {
        None
    };
    let path_matcher = if req.search_paths {
        Some(build_matcher(&req.query, req.regex, req.ignore_case)?)
    } else {
        None
    };

    // Explicitly naming an excluded folder as the walk root expresses
    // the caller's intent to see inside it: the default-exclude filter
    // is disabled for that ENTIRE walk (mirrors coder::tree's behavior).
    let use_default_excludes =
        req.use_default_excludes && !resolver.is_default_excluded_dir(&walk_root);

    let mut content_matches: Vec<ContentMatch> = Vec::new();
    let mut path_matches: Vec<PathMatch> = Vec::new();
    let mut truncated = false;
    // CONVERTED WIRE BYTES accounting (same philosophy as
    // batch_read_budget_bytes in read_file.rs): charge the strings that
    // will actually be serialized. Monotone bounding of the payload
    // STRINGS only, not the JSON envelope — actual wire bytes can exceed
    // the budget by structural overhead (keys, quotes, commas, line/column
    // numbers) plus escape expansion. Exhaustion sets `truncated` — never
    // an error.
    let mut budget_remaining: u64 = cfg.search_response_budget_bytes;
    let mut budget_exhausted = false;

    let walker = walkdir::WalkDir::new(&walk_root).follow_links(false);
    // Suppress descent into default-excluded DIRECTORIES at the dir
    // boundary (dir-companion set; files merely NAMED like an excluded
    // directory are unaffected). Excluded FILES are skipped in the loop
    // body below.
    let entries = walker.into_iter().filter_entry(|e| {
        !(use_default_excludes
            && e.file_type().is_dir()
            && resolver.is_default_excluded_dir(e.path()))
    });
    for entry in entries.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let Some(rel) = resolver.relative(abs) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        // Access control (REDACTION INVARIANT): denied files are wholly
        // absent from both result lists. Independent of the hide-only
        // default-exclude filter below — keep the two checks separate.
        if resolver.is_non_accessible(abs) {
            continue;
        }
        // Noise hiding: default-excluded FILES (configured globs only —
        // the dir-boundary companions never apply to files).
        if use_default_excludes && resolver.is_default_excluded(abs) {
            continue;
        }
        if let Some(set) = &include {
            if !set.is_match(&rel) {
                continue;
            }
        }
        if let Some(set) = &exclude {
            if set.is_match(&rel) {
                continue;
            }
        }

        // Matching runs on the root-relative form; emitted paths are the
        // canonical absolute form (decision D2-eng).
        let abs_wire = abs.display().to_string();

        if let Some(matcher) = &path_matcher {
            if matcher.is_match(&rel) {
                if path_matches.len() >= max_matches {
                    truncated = true;
                } else if charge(&mut budget_remaining, abs_wire.len()) {
                    path_matches.push(PathMatch {
                        path: abs_wire.clone(),
                    });
                } else {
                    truncated = true;
                    budget_exhausted = true;
                }
            }
        }
        if budget_exhausted {
            break;
        }

        if let Some(matcher) = &content_matcher {
            // Skip files larger than max_read_bytes during a search — we
            // don't want to load multi-GB blobs into memory by accident.
            if let Ok(md) = std::fs::metadata(abs) {
                if md.len() > cfg.max_read_bytes {
                    continue;
                }
            }
            let bytes = match std::fs::read(abs) {
                Ok(b) => b,
                Err(_) => continue,
            };
            // Cheap binary heuristic: presence of any NUL byte. Skip
            // binary files so the response stays human-readable.
            if bytes.contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            // The file is fully in memory already; collecting the lines
            // lets context slices index into the same file (and ONLY the
            // same file — context never crosses file boundaries).
            let lines: Vec<&str> = text.lines().collect();
            for (line_idx, line) in lines.iter().enumerate() {
                let truncated_line = clip_line(line, max_line_bytes);
                // Only the FIRST match per line is reported (one content
                // match per matching line) — long-standing wire behavior.
                if let Some(m) = matcher.find(truncated_line) {
                    if content_matches.len() >= max_matches {
                        truncated = true;
                        break;
                    }
                    // Context slices over the collected lines, clipped at
                    // the file edges; overlap between adjacent matches is
                    // duplicated, not merged. Same per-line truncation as
                    // the matched text.
                    let before: Vec<String> = lines[line_idx.saturating_sub(ctx_before)..line_idx]
                        .iter()
                        .map(|l| clip_line(l, max_line_bytes).to_string())
                        .collect();
                    let after_end = (line_idx + 1).saturating_add(ctx_after).min(lines.len());
                    let after: Vec<String> = lines[line_idx + 1..after_end]
                        .iter()
                        .map(|l| clip_line(l, max_line_bytes).to_string())
                        .collect();
                    let cost = abs_wire.len()
                        + truncated_line.len()
                        + before.iter().map(String::len).sum::<usize>()
                        + after.iter().map(String::len).sum::<usize>();
                    if !charge(&mut budget_remaining, cost) {
                        truncated = true;
                        budget_exhausted = true;
                        break;
                    }
                    content_matches.push(ContentMatch {
                        path: abs_wire.clone(),
                        line: (line_idx as u32) + 1,
                        column: (m.start as u32) + 1,
                        text: truncated_line.to_string(),
                        // None when empty so the wire omits the field
                        // (schema-wire consistency: nullable, not
                        // required-but-sometimes-absent).
                        before: (!before.is_empty()).then_some(before),
                        after: (!after.is_empty()).then_some(after),
                    });
                }
            }
            if truncated {
                break;
            }
        }
    }

    Ok(SearchOutput {
        content_matches,
        path_matches,
        truncated,
    })
}

/// Per-request cap on `context_lines_before` / `context_lines_after`.
/// Larger windows belong to `coder::read-file` line windows, not search.
const CONTEXT_LINES_CAP: u32 = 10;

/// Validate one context-lines knob against [`CONTEXT_LINES_CAP`].
/// `None` means 0 (no context).
fn validate_context_lines(field: &str, value: Option<u32>) -> Result<usize, CoderError> {
    let v = value.unwrap_or(0);
    if v > CONTEXT_LINES_CAP {
        return Err(CoderError::BadInput(format!(
            "{field} is {v} but the maximum is {CONTEXT_LINES_CAP}. \
             Re-call with {field} <= {CONTEXT_LINES_CAP}; for a wider view \
             read the file with coder::read-file line_from/line_to."
        )));
    }
    Ok(v as usize)
}

/// Per-line truncation to `max_line_bytes` — one rule for the matched
/// line and its context lines. The clip point is floored to the nearest
/// UTF-8 char boundary (a mid-character byte slice panics), so a clipped
/// line can come out up to 3 bytes short of the cap — never over it.
/// (`str::floor_char_boundary` is nightly-only; walk back on stable.)
fn clip_line(line: &str, max_line_bytes: usize) -> &str {
    if line.len() <= max_line_bytes {
        return line;
    }
    let mut end = max_line_bytes;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// Charge `cost` wire bytes against the remaining response budget.
/// Returns false (charging nothing) when the cost would overdraw — the
/// caller stops accumulating and sets `truncated`; the search degrades,
/// it never errors.
fn charge(remaining: &mut u64, cost: usize) -> bool {
    let cost = cost as u64;
    if cost > *remaining {
        return false;
    }
    *remaining -= cost;
    true
}

fn build_globset(patterns: &[String]) -> Result<Option<globset::GlobSet>, CoderError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut b = globset::GlobSetBuilder::new();
    for p in patterns {
        let g = globset::Glob::new(p)
            .map_err(|e| CoderError::BadInput(format!("bad glob {p:?}: {e}")))?;
        b.add(g);
    }
    let set = b
        .build()
        .map_err(|e| CoderError::BadInput(format!("globset build failed: {e}")))?;
    Ok(Some(set))
}

/// Lightweight matcher abstraction — wraps either a `Regex` or a literal
/// substring search. Both report a 0-based column for the first match
/// position so the wire `column` can be 1-based.
enum Matcher {
    Regex(regex::Regex),
    Literal { needle: String, ignore_case: bool },
}

struct Match {
    start: usize,
}

impl Matcher {
    fn is_match(&self, hay: &str) -> bool {
        self.find(hay).is_some()
    }
    fn find(&self, hay: &str) -> Option<Match> {
        match self {
            Matcher::Regex(re) => re.find(hay).map(|m| Match { start: m.start() }),
            Matcher::Literal {
                needle,
                ignore_case,
            } => {
                if *ignore_case {
                    let hay_l = hay.to_lowercase();
                    let needle_l = needle.to_lowercase();
                    hay_l.find(&needle_l).map(|s| Match { start: s })
                } else {
                    hay.find(needle.as_str()).map(|s| Match { start: s })
                }
            }
        }
    }
}

fn build_matcher(query: &str, regex: bool, ignore_case: bool) -> Result<Matcher, CoderError> {
    if regex {
        let mut builder = regex::RegexBuilder::new(query);
        builder.case_insensitive(ignore_case);
        let re = builder
            .build()
            .map_err(|e| CoderError::BadInput(format!("bad regex {query:?}: {e}")))?;
        Ok(Matcher::Regex(re))
    } else {
        Ok(Matcher::Literal {
            needle: query.to_string(),
            ignore_case,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            search_default_max_matches: 1000,
            search_default_max_line_bytes: 4096,
            ..CoderConfig::default()
        };
        let cfg = Arc::new(cfg);
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    /// Expected wire path: responses carry canonical absolute paths.
    fn abs(tmp: &tempfile::TempDir, rel: &str) -> String {
        std::fs::canonicalize(tmp.path())
            .unwrap()
            .join(rel)
            .display()
            .to_string()
    }

    fn write(tmp: &tempfile::TempDir, rel: &str, body: &str) {
        let p = tmp.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[tokio::test]
    async fn literal_content_match_returns_line_column() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "alpha\nbeta needle here\ngamma\n");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        let m = &out.content_matches[0];
        assert_eq!(m.path, abs(&tmp, "a.txt"));
        assert_eq!(m.line, 2);
        assert_eq!(m.column, 6);
    }

    /// Pins the SKILL.md "poor-man's outline" recipe: matching is
    /// PER-LINE, so `^` anchors at each line start and a leading `\s*`
    /// catches indented declarations (impl/class methods). `path` scopes
    /// the walk to a folder; the root-relative include glob pins the one
    /// file. The pattern consumes the indentation, so `column` stays 1
    /// even for indented hits.
    #[tokio::test]
    async fn outline_recipe_returns_indented_declarations() {
        let (tmp, r, c) = setup();
        write(
            &tmp,
            "src/lib.rs",
            "pub struct Config {}\n\nimpl Config {\n    pub fn load() -> Self {\n        \
             Self {}\n    }\n\n    fn validate(&self) -> bool {\n        true\n    }\n}\n",
        );
        write(&tmp, "src/other.rs", "fn excluded_by_glob() {}\n");
        let out = handle(
            r,
            c,
            SearchInput {
                query: r"^\s*(pub |fn |class |def |func |impl |interface )".into(),
                path: "src".into(),
                regex: true,
                ignore_case: false,
                include_globs: vec!["src/lib.rs".into()],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        let hits: Vec<(u32, u32, &str)> = out
            .content_matches
            .iter()
            .map(|m| (m.line, m.column, m.text.as_str()))
            .collect();
        assert_eq!(
            hits,
            vec![
                (1, 1, "pub struct Config {}"),
                (3, 1, "impl Config {"),
                (4, 1, "    pub fn load() -> Self {"),
                (8, 1, "    fn validate(&self) -> bool {"),
            ],
            "outline must include the indented impl methods (lines 4 and 8)"
        );
        assert!(
            !out.content_matches
                .iter()
                .any(|m| m.path.ends_with("other.rs")),
            "include_globs must pin the single file"
        );
    }

    #[tokio::test]
    async fn regex_content_match() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "fn foo() {}\nfn Bar() {}\n");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "^fn [A-Z]".into(),
                path: ".".into(),
                regex: true,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(out.content_matches[0].line, 2);
    }

    #[tokio::test]
    async fn path_match_returns_paths() {
        let (tmp, r, c) = setup();
        write(&tmp, "src/foo.rs", "x");
        write(&tmp, "src/bar.ts", "x");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "foo".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: false,
                search_paths: true,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        let paths: Vec<_> = out.path_matches.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&abs(&tmp, "src/foo.rs").as_str()));
        assert!(!paths.contains(&abs(&tmp, "src/bar.ts").as_str()));
    }

    #[tokio::test]
    async fn non_accessible_files_skipped() {
        let (tmp, r, c) = setup();
        write(&tmp, ".env", "API_KEY=needle");
        write(&tmp, "a.txt", "needle here");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: true,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        // The .env file must not appear in either match list.
        let env_abs = abs(&tmp, ".env");
        for m in &out.content_matches {
            assert_ne!(m.path, env_abs, "non-accessible file leaked content");
        }
        for m in &out.path_matches {
            assert_ne!(m.path, env_abs, "non-accessible file leaked path");
        }
    }

    #[tokio::test]
    async fn include_and_exclude_globs() {
        let (tmp, r, c) = setup();
        write(&tmp, "src/a.rs", "needle");
        write(&tmp, "src/b.ts", "needle");
        write(&tmp, "build/c.rs", "needle");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec!["**/*.rs".into()],
                exclude_globs: vec!["build/**".into()],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        let paths: Vec<_> = out
            .content_matches
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        assert_eq!(paths, vec![abs(&tmp, "src/a.rs")]);
    }

    #[tokio::test]
    async fn truncation_flag_when_cap_hit() {
        let (tmp, r, _c) = setup();
        for i in 0..5 {
            write(&tmp, &format!("f{i}.txt"), "needle\n");
        }
        let cfg = Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            search_default_max_matches: 1000,
            max_read_bytes: 1024 * 1024,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            cfg,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: Some(2),
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 2);
        assert!(out.truncated);
    }

    #[tokio::test]
    async fn empty_query_rejected() {
        let (_tmp, r, c) = setup();
        let err = handle(
            r,
            c,
            SearchInput {
                query: "".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: true,
                base_dir: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_when_nothing_to_search() {
        // Disabling BOTH search_content and search_paths leaves nothing to
        // do; the guard must reject with C210 rather than silently return an
        // empty result. This branch was previously asserted only by the
        // now-dropped BDD suite.
        let (_tmp, r, c) = setup();
        let err = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: false,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    #[tokio::test]
    async fn ignore_case_literal() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "NEEDLE here");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: true,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
    }

    #[tokio::test]
    async fn skips_binary_files() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("blob.bin"), [0u8, 1, 2, 3, b'n', b'e']).unwrap();
        let out = handle(
            r,
            c,
            SearchInput {
                query: "ne".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        assert!(out.content_matches.is_empty());
    }

    #[tokio::test]
    async fn path_scopes_walk_to_subdir() {
        let (tmp, r, c) = setup();
        write(&tmp, "a/x.txt", "needle here");
        write(&tmp, "b/y.txt", "needle here");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: "a".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                context_lines_before: None,
                context_lines_after: None,
                use_default_excludes: true,
                search_content: true,
                search_paths: false,
                base_dir: None,
            },
        )
        .await
        .unwrap();
        let paths: Vec<_> = out
            .content_matches
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        assert_eq!(paths, vec![abs(&tmp, "a/x.txt")]);
    }

    // -----------------------------------------------------------------
    // S2 (v0.4.0): context lines, response byte budget, default excludes.
    // -----------------------------------------------------------------

    /// Request with serde defaults for everything but `query` — the same
    /// defaults a wire caller gets.
    fn base_input(query: &str) -> SearchInput {
        SearchInput {
            query: query.into(),
            path: ".".into(),
            regex: false,
            ignore_case: false,
            include_globs: vec![],
            exclude_globs: vec![],
            max_matches: None,
            max_line_bytes: None,
            context_lines_before: None,
            context_lines_after: None,
            use_default_excludes: true,
            search_content: true,
            search_paths: false,
            base_dir: None,
        }
    }

    /// Expected wire context: `Some` of owned lines (the wire omits the
    /// field entirely when no context exists — `None`, never `Some([])`).
    fn ctx(lines: &[&str]) -> Option<Vec<String>> {
        Some(lines.iter().map(|s| s.to_string()).collect())
    }

    /// Jail with an explicit `search_response_budget_bytes`.
    fn cfg_with_budget(tmp: &tempfile::TempDir, budget: u64) -> Arc<CoderConfig> {
        Arc::new(CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec![],
            max_read_bytes: 1024 * 1024,
            search_default_max_matches: 1000,
            search_default_max_line_bytes: 4096,
            search_response_budget_bytes: budget,
            ..CoderConfig::default()
        })
    }

    #[tokio::test]
    async fn context_lines_in_file_order() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "l1\nl2\nl3 needle\nl4\nl5\n");
        let out = handle(
            r,
            c,
            SearchInput {
                context_lines_before: Some(2),
                context_lines_after: Some(2),
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        let m = &out.content_matches[0];
        assert_eq!(m.before, ctx(&["l1", "l2"]));
        assert_eq!(m.after, ctx(&["l4", "l5"]));
    }

    #[tokio::test]
    async fn context_clipped_at_file_edges_and_duplicated_across_matches() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "needle first\nmid\nneedle last\n");
        let out = handle(
            r,
            c,
            SearchInput {
                context_lines_before: Some(5),
                context_lines_after: Some(5),
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 2);
        // First match: nothing before line 1; after clipped at EOF.
        assert!(out.content_matches[0].before.is_none());
        assert_eq!(out.content_matches[0].after, ctx(&["mid", "needle last"]));
        // Second match: overlapping context is duplicated, not merged.
        assert_eq!(out.content_matches[1].before, ctx(&["needle first", "mid"]));
        assert!(out.content_matches[1].after.is_none());
    }

    #[tokio::test]
    async fn context_never_crosses_file_boundaries() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "needle a\n");
        write(&tmp, "b.txt", "needle b\n");
        let out = handle(
            r,
            c,
            SearchInput {
                context_lines_before: Some(3),
                context_lines_after: Some(3),
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 2);
        for m in &out.content_matches {
            assert!(m.before.is_none(), "crossed file boundary: {:?}", m.before);
            assert!(m.after.is_none(), "crossed file boundary: {:?}", m.after);
        }
    }

    #[tokio::test]
    async fn context_lines_over_cap_rejected_c210() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "needle\n");
        let err = handle(
            r.clone(),
            c.clone(),
            SearchInput {
                context_lines_before: Some(11),
                ..base_input("needle")
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(
            err.contains("11") && err.contains("10"),
            "must name the actual value and the cap: {err}"
        );
        let err = handle(
            r,
            c,
            SearchInput {
                context_lines_after: Some(99),
                ..base_input("needle")
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
        assert!(
            err.contains("99") && err.contains("10"),
            "must name the actual value and the cap: {err}"
        );
    }

    #[tokio::test]
    async fn context_lines_subject_to_max_line_bytes() {
        let (tmp, r, c) = setup();
        let long = "x".repeat(100);
        write(&tmp, "a.txt", &format!("{long}\nneedle\n{long}\n"));
        let out = handle(
            r,
            c,
            SearchInput {
                max_line_bytes: Some(10),
                context_lines_before: Some(1),
                context_lines_after: Some(1),
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        let m = &out.content_matches[0];
        assert_eq!(m.before, Some(vec!["x".repeat(10)]));
        assert_eq!(m.after, Some(vec!["x".repeat(10)]));
    }

    #[tokio::test]
    async fn budget_truncates_without_error_deterministically() {
        let (tmp, r, _c) = setup();
        write(&tmp, "a.txt", "needle one\nneedle two\n");
        // Budget fits exactly the first match (path + text), not both.
        let cost1 = (abs(&tmp, "a.txt").len() + "needle one".len()) as u64;
        let out = handle(r, cfg_with_budget(&tmp, cost1), base_input("needle"))
            .await
            .unwrap();
        assert_eq!(
            out.content_matches.len(),
            1,
            "deterministic cutoff after the first match"
        );
        assert_eq!(out.content_matches[0].text, "needle one");
        assert!(out.truncated, "budget cutoff must set the truncated flag");
    }

    #[tokio::test]
    async fn budget_accounting_includes_context_lines() {
        let (tmp, r, _c) = setup();
        write(
            &tmp,
            "a.txt",
            "ctx before\nneedle one\nctx after\nfiller\nneedle two\n",
        );
        let path_len = abs(&tmp, "a.txt").len() as u64;
        // Without context this budget fits both matches exactly…
        let budget = 2 * (path_len + "needle one".len() as u64);
        let out = handle(
            r.clone(),
            cfg_with_budget(&tmp, budget),
            base_input("needle"),
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 2);
        assert!(!out.truncated);
        // …but with context lines charged, only the first match fits.
        let out = handle(
            r,
            cfg_with_budget(&tmp, budget),
            SearchInput {
                context_lines_before: Some(1),
                context_lines_after: Some(1),
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(
            out.content_matches.len(),
            1,
            "context lines must count toward the budget"
        );
        assert!(out.truncated);
    }

    #[tokio::test]
    async fn budget_applies_to_path_matches_too() {
        let (tmp, r, _c) = setup();
        write(&tmp, "needle1.txt", "x");
        write(&tmp, "needle2.txt", "x");
        // Both absolute paths have the same length; budget fits one.
        let p_len = abs(&tmp, "needle1.txt").len() as u64;
        let out = handle(
            r,
            cfg_with_budget(&tmp, p_len),
            SearchInput {
                search_content: false,
                search_paths: true,
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.path_matches.len(), 1);
        assert!(out.truncated);
    }

    #[tokio::test]
    async fn default_excludes_skip_node_modules_in_content_and_path_results() {
        let (tmp, r, c) = setup();
        write(&tmp, "node_modules/pkg/needle.js", "needle inside");
        write(&tmp, "src/needle.rs", "needle inside");
        let out = handle(
            r,
            c,
            SearchInput {
                search_paths: true,
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        // No names, counts, or placeholders for excluded entries anywhere.
        let serialized = serde_json::to_string(&out).unwrap();
        assert!(
            !serialized.contains("node_modules"),
            "excluded dir leaked: {serialized}"
        );
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(out.content_matches[0].path, abs(&tmp, "src/needle.rs"));
        assert_eq!(out.path_matches.len(), 1);
        assert_eq!(out.path_matches[0].path, abs(&tmp, "src/needle.rs"));
    }

    #[tokio::test]
    async fn use_default_excludes_false_searches_inside() {
        let (tmp, r, c) = setup();
        write(&tmp, "node_modules/pkg/dep.js", "needle inside");
        let out = handle(
            r,
            c,
            SearchInput {
                use_default_excludes: false,
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(
            out.content_matches[0].path,
            abs(&tmp, "node_modules/pkg/dep.js")
        );
    }

    #[tokio::test]
    async fn file_named_like_excluded_dir_still_searched() {
        let (tmp, r, c) = setup();
        // A FILE named `dist`: dir-boundary companions apply to
        // directories only, so this must still be scanned.
        write(&tmp, "dist", "needle in a file named dist");
        let out = handle(r, c, base_input("needle")).await.unwrap();
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(out.content_matches[0].path, abs(&tmp, "dist"));
    }

    #[tokio::test]
    async fn explicitly_excluded_walk_root_disables_filter() {
        let (tmp, r, c) = setup();
        write(&tmp, "node_modules/pkg/dep.js", "needle inside");
        // Naming the excluded folder as the walk root expresses intent to
        // see inside it (mirrors coder::tree's S1-reviewed behavior).
        let out = handle(
            r,
            c,
            SearchInput {
                path: "node_modules".into(),
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
    }

    // REDACTION INVARIANT regression: denied files stay wholly absent
    // from the whole serialized response even with the new options.
    #[tokio::test]
    async fn denied_files_wholly_absent_with_context_and_excludes_off() {
        let (tmp, r, c) = setup();
        write(&tmp, ".env", "API_KEY=needle secret");
        write(&tmp, "ok.txt", "needle here");
        let out = handle(
            r,
            c,
            SearchInput {
                search_paths: true,
                context_lines_before: Some(2),
                context_lines_after: Some(2),
                use_default_excludes: false,
                ..base_input("needle")
            },
        )
        .await
        .unwrap();
        let serialized = serde_json::to_string(&out).unwrap();
        assert!(
            !serialized.contains(".env"),
            "denied path leaked: {serialized}"
        );
        assert!(
            !serialized.contains("API_KEY"),
            "denied content leaked: {serialized}"
        );
        assert_eq!(out.content_matches.len(), 1);
    }

    // -----------------------------------------------------------------
    // clip_line UTF-8 boundary regressions: clipping must floor to a
    // char boundary instead of panicking mid-character (ship-blocker
    // found in S2 review). Three confirmed panic variants pinned below.
    // -----------------------------------------------------------------

    // a1: matched-text clip lands mid-'é' (2 bytes straddling the cap).
    #[tokio::test]
    async fn clip_mid_char_in_matched_line_does_not_panic() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "abétail\n");
        let out = handle(
            r,
            c,
            SearchInput {
                max_line_bytes: Some(3),
                ..base_input("ab")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        // Cap 3 falls inside 'é' (bytes 2..4); floor to the boundary at 2.
        assert_eq!(out.content_matches[0].text, "ab");
    }

    // a2: clean ASCII match poisoned by a multibyte CONTEXT neighbor —
    // the cap lands mid-'😀' (4 bytes) in the before-line.
    #[tokio::test]
    async fn clip_mid_char_in_context_line_does_not_panic() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "abc😀x\nhit42\n");
        let out = handle(
            r,
            c,
            SearchInput {
                max_line_bytes: Some(5),
                context_lines_before: Some(1),
                ..base_input("hit")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(out.content_matches[0].text, "hit42");
        // Cap 5 falls inside '😀' (bytes 3..7); floor to the boundary at 3.
        assert_eq!(out.content_matches[0].before, ctx(&["abc"]));
    }

    // a3 (worst): clip_line runs on EVERY scanned line before matching,
    // so a non-matching over-cap line with a multibyte char straddling
    // the DEFAULT 4096-byte cap killed the whole handler with zero
    // caller-supplied knobs — regardless of query.
    #[tokio::test]
    async fn clip_mid_char_in_non_matching_scanned_line_does_not_panic() {
        let (tmp, r, c) = setup();
        // '😀' occupies bytes 4095..4099: the default
        // search_default_max_line_bytes (4096) lands mid-character.
        let line = format!("{}😀tail", "a".repeat(4095));
        write(&tmp, "big.txt", &format!("{line}\n"));
        let out = handle(r, c, base_input("zzz-no-match")).await.unwrap();
        assert!(out.content_matches.is_empty());
        assert!(!out.truncated);
    }

    // Wire-shape pin: matches without context carry `None` (never
    // `Some([])`), so the fields are omitted entirely and context-free
    // responses keep the pre-S2 shape — matching the nullable,
    // non-required schema.
    #[tokio::test]
    async fn empty_context_omitted_from_wire() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "needle\n");
        let out = handle(r, c, base_input("needle")).await.unwrap();
        assert!(out.content_matches[0].before.is_none());
        assert!(out.content_matches[0].after.is_none());
        let serialized = serde_json::to_string(&out).unwrap();
        assert!(
            !serialized.contains("\"before\"") && !serialized.contains("\"after\""),
            "empty context must be skipped on the wire: {serialized}"
        );
    }

    // Clip-then-charge ordering pin: a line clipped at `max_line_bytes`
    // must be charged its POST-clip byte length. A budget sized to the
    // clipped text fits exactly; charging the raw line length would
    // overdraw and wrongly return zero matches.
    #[tokio::test]
    async fn budget_charges_post_clip_length() {
        let (tmp, r, _c) = setup();
        write(&tmp, "a.txt", &format!("ab{}\n", "é".repeat(50)));
        // max_line_bytes 3 falls inside the first 'é' (bytes 2..4); the
        // stored text floors to "ab" (2 bytes), not the 102-byte raw line.
        let budget = (abs(&tmp, "a.txt").len() + "ab".len()) as u64;
        let out = handle(
            r,
            cfg_with_budget(&tmp, budget),
            SearchInput {
                max_line_bytes: Some(3),
                ..base_input("ab")
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(out.content_matches[0].text, "ab");
        assert!(!out.truncated);
    }
}
