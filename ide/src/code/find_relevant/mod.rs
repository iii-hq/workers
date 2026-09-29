//! `coder::find-relevant` — judge-ranked code discovery, a Rust port of
//! dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit 82ef1fd.
//!
//! Walks the jailed folder once ([`walk`]), then asks `judge::evaluate`
//! jevgrep's yes/no questions ([`prompts`]) level by level ([`navigate`]),
//! following only the branches the judge admits. Every entry passes the
//! jail's protections and jevgrep's secret and content filters before any
//! of it reaches the judge; paths leave as root-relative, never the host
//! layout. A missing or failing judge is a typed `unavailable` result, not
//! an error.
//!
//! Output order follows jevgrep's `apps/cli/src/render.ts`.

pub mod navigate;
pub mod prompts;
pub mod walk;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::judge::{self, Evaluator, JudgeError};
use crate::code::path::PathResolver;
use navigate::{Run, Stop};

pub const MAX_QUERY_BYTES: usize = 4000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
/// Below the harness's 300 s `dispatch_timeout_ms`.
pub const MAX_TIMEOUT_MS: u64 = 280_000;
/// Smallest judge context window (tokens) whose states fit untruncated.
pub const MIN_WINDOW_TOKENS: u64 = 8_192;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_find_relevant_input")]
pub struct FindRelevantInput {
    /// Behavioural question in natural language: how, why or where
    /// something works (1..=4000 bytes).
    pub query: String,
    /// Folder to search (default `.`); result paths are absolute.
    #[serde(default = "default_path")]
    pub path: String,
    /// Root-relative glob patterns to leave out; they only narrow.
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    /// Deadline for the whole ask in ms; work left at the deadline makes
    /// the result `incomplete`.
    #[serde(default = "default_timeout_ms")]
    #[schemars(range(min = 1000, max = 280000))]
    pub timeout_ms: u64,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

fn default_path() -> String {
    ".".to_string()
}

fn default_timeout_ms() -> u64 {
    120_000
}

fn example_find_relevant_input() -> serde_json::Value {
    serde_json::json!({
        "query": "where does the harness stamp the filesystem scope on coder calls?",
        "path": "harness/src"
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Every admitted branch was explored.
    Complete,
    /// Partial coverage (deadline, a failed or oversized request, a
    /// resource limit): see `issues`; narrow `path` and retry.
    Incomplete,
    /// No judge answered; use coder::search.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Excerpt {
    pub line_from: u32,
    pub line_to: u32,
    /// Verbatim source of the line range.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Lead {
    /// Declaration name.
    pub name: String,
    pub line_from: u32,
    pub line_to: u32,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CallLead {
    /// The selected declaration making the call.
    pub caller: String,
    /// The called method.
    pub name: String,
    pub line_from: u32,
    pub line_to: u32,
    /// Earlier bases in the resolution order that were not inspected.
    pub unknown_earlier_bases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RelevantFile {
    /// Absolute path.
    pub path: String,
    /// Navigation score in [0, 1].
    pub score: f64,
    /// Judge's read-early priority in [0, 1], when assessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
    /// Estimated roles: implementation, caller, test, fixture, helper.
    pub roles: Vec<String>,
    pub excerpts: Vec<Excerpt>,
    /// Declarations worth reading whose source is not shown.
    pub leads: Vec<Lead>,
    /// Possible local calls from the shown source (not runtime-verified).
    pub call_leads: Vec<CallLead>,
    /// Some source was left out (output budget or the file changed); its
    /// locations remain.
    pub source_omitted: bool,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Stats {
    pub judge_calls: u64,
    pub questions: u64,
    pub input_tokens: u64,
    pub cache_hits: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FindRelevantOutput {
    pub status: Status,
    /// Why the result is not complete.
    pub reason: Option<String>,
    /// Best first.
    pub files: Vec<RelevantFile>,
    /// AGENTS.md files at the root and above returned files.
    pub agents_md: Vec<String>,
    /// Coverage issues by kind.
    pub issues: BTreeMap<String, u64>,
    pub stats: Stats,
}

/// The registered handler: reads the session's judge provider here, in the
/// handler task, and runs the ask over the bus.
pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    iii: iii_sdk::IIIClient,
    req: FindRelevantInput,
) -> Result<FindRelevantOutput, String> {
    let provider = judge::session_provider();
    let evaluate = judge::evaluator(iii.clone(), provider.clone());
    run(
        resolver,
        cfg,
        req,
        judge::window(&iii, provider.as_deref()),
        evaluate,
    )
    .await
    .map_err(err_to_string)
}

/// One ask over any judge: `window` is awaited once, after the input is
/// validated; `evaluate` answers every request.
pub async fn run(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: FindRelevantInput,
    window: impl Future<Output = Result<Option<u64>, JudgeError>>,
    evaluate: Evaluator,
) -> Result<FindRelevantOutput, CoderError> {
    let started = Instant::now();
    if req.query.trim().is_empty() {
        return Err(CoderError::BadInput("query must not be empty".into()));
    }
    if req.query.len() > MAX_QUERY_BYTES {
        return Err(CoderError::BadInput(format!(
            "query is {} bytes; at most {MAX_QUERY_BYTES}",
            req.query.len()
        )));
    }
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&req.timeout_ms) {
        return Err(CoderError::BadInput(format!(
            "timeout_ms must be within {MIN_TIMEOUT_MS}..={MAX_TIMEOUT_MS}"
        )));
    }
    let deadline = started + Duration::from_millis(req.timeout_ms);
    let walk_root = resolver.resolve_scope(req.fs_scope.as_ref(), &req.path)?;
    let md = std::fs::metadata(&walk_root).map_err(|e| CoderError::io_for_path(e, &req.path))?;
    if !md.is_dir() {
        return Err(CoderError::BadInput(format!(
            "not a directory: {}",
            req.path
        )));
    }
    let exclude = crate::code::functions::search::build_globset(&req.exclude_globs)?;

    let unavailable = |reason: String| FindRelevantOutput {
        status: Status::Unavailable,
        reason: Some(reason),
        files: Vec::new(),
        agents_md: Vec::new(),
        issues: BTreeMap::new(),
        stats: Stats {
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..Stats::default()
        },
    };
    let cap = match window.await {
        Err(error) => return Ok(logged(unavailable(reason_of(error)))),
        Ok(Some(tokens)) if tokens < MIN_WINDOW_TOKENS => {
            return Ok(logged(unavailable("judge window too small".into())))
        }
        Ok(Some(tokens)) => navigate::MAX_REQUEST_BYTES.min(2 * tokens as usize),
        Ok(None) => navigate::MAX_REQUEST_BYTES,
    };

    let max_read_bytes = cfg.max_read_bytes;
    let walk_resolver = resolver.clone();
    let tree = tokio::task::spawn_blocking(move || {
        walk::walk(&walk_resolver, &walk_root, exclude, max_read_bytes)
    })
    .await
    .map_err(|e| CoderError::Io(format!("find-relevant walk failed: {e}")))?;
    let truncated = tree.truncated;
    let run = Arc::new(Run {
        query: req.query,
        tree,
        evaluate,
        deadline,
        cap,
        state: Mutex::new(Default::default()),
    });
    if truncated {
        run.issue("resource_limit");
    }
    run.discover(vec![".".into()]).await;

    let root = run.tree.root.clone();
    let state = std::mem::take(&mut *run.state());
    let mut files: Vec<RelevantFile> = state
        .candidates
        .into_values()
        .map(|candidate| RelevantFile {
            path: root.join(&candidate.path).display().to_string(),
            score: candidate.score,
            priority: None,
            roles: Vec::new(),
            excerpts: Vec::new(),
            leads: Vec::new(),
            call_leads: Vec::new(),
            source_omitted: false,
        })
        .collect();
    sort_files(&mut files);
    let (status, reason) = match state.stop {
        Some(Stop::Unavailable(reason)) if state.answered == 0 => {
            (Status::Unavailable, Some(reason))
        }
        Some(Stop::Unavailable(reason)) => (Status::Incomplete, Some(reason)),
        Some(Stop::Deadline) => (Status::Incomplete, Some("deadline".into())),
        None if state.issues.is_empty() => (Status::Complete, None),
        None => (Status::Incomplete, None),
    };
    Ok(logged(FindRelevantOutput {
        status,
        reason,
        files,
        agents_md: Vec::new(),
        issues: state.issues,
        stats: Stats {
            judge_calls: state.judge_calls,
            questions: state.questions,
            input_tokens: state.input_tokens,
            cache_hits: 0,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
    }))
}

fn reason_of(error: JudgeError) -> String {
    match error {
        JudgeError::Unavailable(reason) | JudgeError::Rejected(reason) => reason,
        other => other.to_string(),
    }
}

/// render.ts: `priority ?? score` first, then score, then path.
fn sort_files(files: &mut [RelevantFile]) {
    files.sort_by(|a, b| {
        let rank = |f: &RelevantFile| f.priority.unwrap_or(f.score);
        rank(b)
            .total_cmp(&rank(a))
            .then(b.score.total_cmp(&a.score))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// One line per ask; never the query or any path.
fn logged(output: FindRelevantOutput) -> FindRelevantOutput {
    tracing::info!(
        status = ?output.status,
        reason = output.reason.as_deref().unwrap_or(""),
        judge_calls = output.stats.judge_calls,
        questions = output.stats.questions,
        input_tokens = output.stats.input_tokens,
        elapsed_ms = output.stats.elapsed_ms,
        files = output.files.len(),
        "coder::find-relevant"
    );
    output
}

#[cfg(test)]
mod tests;
