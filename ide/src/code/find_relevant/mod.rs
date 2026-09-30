//! `coder::find-relevant` — judge-ranked code discovery, a Rust port of
//! dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit 82ef1fd.
//!
//! Asks `judge::evaluate` jevgrep's yes/no questions ([`prompts`]) level by
//! level ([`navigate`]), listing a folder ([`walk`]) only when it follows a
//! branch the judge admits. Every entry passes the jail's protections and
//! jevgrep's secret and content filters before any of it reaches the judge;
//! paths leave as root-relative, never the host layout. A missing or failing
//! judge is a typed `unavailable` result, not an error.
//!
//! Output order follows jevgrep's `apps/cli/src/render.ts`; the answer
//! cache ports `packages/core/src/cache.ts` (in memory only) and the
//! `AGENTS.md` lookup `repository-context.ts`.

pub mod navigate;
pub mod passes;
pub mod prompts;
pub mod select;
pub mod units;
pub mod walk;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use judge_contract::{Evaluation, Question};
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::judge::{self, Evaluator, JudgeError, Scores};
use crate::code::path::PathResolver;
use navigate::{Candidate, Run, Stop};

pub const MAX_QUERY_BYTES: usize = 4000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
/// Below the harness's 300 s `dispatch_timeout_ms`.
pub const MAX_TIMEOUT_MS: u64 = 280_000;
/// Smallest judge context window (tokens) whose states fit untruncated.
pub const MIN_WINDOW_TOKENS: u64 = 8_192;
/// Excerpt bytes one result carries (`coder::read-file`'s ceiling).
pub const MAX_SOURCE_BYTES: usize = 131_072;
/// A result's size as the harness counts it ([`result_bytes`]) stays under
/// this: harness/src/config.rs `default_max_result_bytes` (262_144), past
/// which `trigger::cap_result` swaps the whole result for a marker, less
/// headroom for its wrapping.
pub const MAX_RESULT_BYTES: usize = 250_000;

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
    /// Root-relative globs (not gitignore lines) to leave out; `gen/` or
    /// `gen/**` drops the folder itself. They only narrow.
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
    /// Partial coverage (deadline, judge token budget, a failed or
    /// oversized request, a resource limit): see `reason` and `issues`;
    /// narrow `path` and retry.
    Incomplete,
    /// No judge answered; use coder::search.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Excerpt {
    pub line_from: u32,
    pub line_to: u32,
    /// Verbatim source of the line range; with `partial`, only that byte
    /// span of it.
    pub text: String,
    /// Present when `text` is only a byte span of its lines (for example
    /// inside a line over 24000 bytes): do not rewrite whole lines from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial: Option<ByteSpan>,
}

/// UTF-8 byte offsets in the file: `[byte_from, byte_to)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
pub struct ByteSpan {
    pub byte_from: u32,
    pub byte_to: u32,
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
    let slots = cfg.find_relevant_judge_slots as usize;
    let evaluate = judge::evaluator(iii.clone(), provider.clone(), slots);
    let cache = Some(provider.clone().unwrap_or_default());
    run(
        resolver,
        cfg,
        req,
        |deadline| async move { judge::window(&iii, provider.as_deref(), deadline).await },
        evaluate,
        cache,
    )
    .await
    .map_err(err_to_string)
}

/// One ask over any judge: `window` runs once with the ask deadline, after
/// the input is validated; `evaluate` answers every request the answer
/// cache (namespace `cache`, `None` = bypass) cannot.
pub async fn run<W: Future<Output = Result<Option<u64>, JudgeError>>>(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: FindRelevantInput,
    window: impl FnOnce(Instant) -> W,
    evaluate: Evaluator,
    cache: Option<String>,
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
    if walk::in_git_dir(&walk_root) {
        return Err(CoderError::BadInput(format!(
            "path is inside a .git directory, which is never searched: {}",
            req.path
        )));
    }
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
    let window = match window(deadline).await {
        Err(error) => return Ok(logged(unavailable(error.reason()))),
        Ok(Some(tokens)) if tokens < MIN_WINDOW_TOKENS => {
            return Ok(logged(unavailable("judge window too small".into())))
        }
        Ok(window) => window,
    };
    // A known window caps every request at twice its tokens.
    let cap = |jevgrep: usize| window.map_or(jevgrep, |tokens| jevgrep.min(2 * tokens as usize));

    let run = Arc::new(Run {
        query: req.query,
        tree: walk::Tree::new(&resolver, &walk_root, exclude, cfg.max_read_bytes),
        evaluate,
        deadline,
        cap: cap(navigate::MAX_REQUEST_BYTES),
        state_cap: cap(select::MAX_STATE_BYTES),
        window_cap: cap(usize::MAX),
        cache,
        slots: (cfg.find_relevant_judge_slots as usize).clamp(1, judge::MAX_SLOTS),
        token_budget: cfg.find_relevant_judge_token_budget,
        state: Mutex::new(Default::default()),
    });
    run.discover(vec![".".into()], None).await;
    run.relate().await;
    // The assessment reads only discovery previews, so it runs alongside.
    let (mut selected, mut assessments) =
        tokio::join!(select::select_evidence(&run), passes::assess_files(&run));
    passes::present(&run, &mut selected, &assessments).await;
    // retrieve.ts 722-723: the assessment may outlive the bytes it
    // classified; a file that changed keeps no roles, leads or source.
    let unchecked: Vec<Candidate> = run
        .admitted()
        .into_iter()
        .filter(|c| selected.get(&c.path).is_none_or(|s| !s.source_omitted))
        .collect();
    let checker = run.clone();
    let stale: Vec<String> = tokio::task::spawn_blocking(move || {
        unchecked
            .into_iter()
            .filter(|c| checker.unchanged(c).is_none())
            .map(|c| c.path)
            .collect()
    })
    .await
    .unwrap_or_default();
    for path in stale {
        selected.insert(path, select::Selected::omitted());
    }

    let root = run.tree.root.clone();
    let candidates = run.sorted_candidates();
    let (lookup, listed) = (run.clone(), candidates.clone());
    let agents_md = tokio::task::spawn_blocking(move || agents_md(&lookup, &listed))
        .await
        .unwrap_or_else(|_| {
            run.issue("agents_md_incomplete");
            Vec::new()
        });
    let state = std::mem::take(&mut *run.state());
    let admitted = !candidates.is_empty();
    let mut files: Vec<RelevantFile> = candidates
        .into_iter()
        .map(|candidate| {
            let found = selected.remove(&candidate.path).unwrap_or_default();
            // A file whose source changed keeps no assessment.
            let assessment = assessments
                .remove(&candidate.path)
                .filter(|_| !found.source_omitted);
            let (roles, priority) =
                assessment.map_or((Vec::new(), None), |a| (a.roles, Some(a.priority)));
            RelevantFile {
                path: root.join(&candidate.path).display().to_string(),
                score: candidate.score,
                priority,
                roles,
                excerpts: found.excerpts,
                leads: found.leads,
                call_leads: found.call_leads,
                source_omitted: found.source_omitted,
            }
        })
        .collect();
    sort_files(&mut files);
    let (status, reason) = match state.stop {
        // Nothing admitted yet: point the agent at coder::search.
        Some(Stop::Unavailable(reason)) if !admitted => (Status::Unavailable, Some(reason)),
        Some(Stop::Unavailable(reason)) => (Status::Incomplete, Some(reason)),
        Some(Stop::Deadline) => (Status::Incomplete, Some("deadline".into())),
        Some(Stop::Budget) => (Status::Incomplete, Some("token_budget".into())),
        None if state.issues.is_empty() => (Status::Complete, None),
        None => (Status::Incomplete, None),
    };
    let mut output = FindRelevantOutput {
        status,
        reason,
        files,
        agents_md,
        issues: state.issues,
        stats: Stats {
            judge_calls: state.judge_calls,
            questions: state.questions,
            input_tokens: state.input_tokens,
            cache_hits: state.cache_hits,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
    };
    spend_budget(&mut output, MAX_SOURCE_BYTES, MAX_RESULT_BYTES);
    Ok(logged(output))
}

/// render.ts: `priority ?? score` first, then score, then path.
fn sort_files(files: &mut [RelevantFile]) {
    files.sort_by(|a, b| {
        let rank = |f: &RelevantFile| f.priority.unwrap_or(f.score);
        rank(b)
            .total_cmp(&rank(a))
            .then(b.score.total_cmp(&a.score))
            .then_with(|| walk::locale_cmp(&a.path, &b.path))
    });
}

/// What harness/src/trigger.rs `cap_result` measures for `value`: its
/// compact JSON (`details`) plus that JSON again as one string (`normalize`
/// renders an object without `content` as one text block), so quotes and
/// backslashes count several times over.
fn result_bytes<T: Serialize + ?Sized>(value: &T) -> usize {
    let json = serde_json::to_string(value).unwrap_or_default();
    json.len() + walk::json_len(json.as_str())
}

/// Spends `result` bytes (as the harness counts them; jevgrep prints
/// everything) by value: the file list first, up to half of `result`; then
/// call leads and leads in output order, up to half of what is left; then
/// excerpts in output order while they also fit `source` bytes (a file that
/// loses one is `source_omitted`). Files and leads that do not fit are cut
/// from the tail, counted as a `resource_limit`.
// ponytail: fixed half split between leads and excerpts; weigh by score if
// agents keep re-reading files whose excerpts were dropped.
fn spend_budget(output: &mut FindRelevantOutput, mut source: usize, result: usize) {
    type Parts = (Vec<Excerpt>, Vec<CallLead>, Vec<Lead>);
    let parts: Vec<Parts> = output
        .files
        .iter_mut()
        .map(|file| {
            (
                std::mem::take(&mut file.excerpts),
                std::mem::take(&mut file.call_leads),
                std::mem::take(&mut file.leads),
            )
        })
        .collect();
    // An element costs at most its `result_bytes` (the 2 quote bytes pay
    // for its commas) and removing one saves at least that less 2, so
    // `used` never undercounts.
    let mut used = result_bytes(output);
    let mut trimmed = false;
    while used > result / 2 {
        let Some(file) = output.files.pop() else {
            break;
        };
        used -= result_bytes(&file).saturating_sub(2).min(used);
        trimmed = true;
    }
    let fits = |used: &mut usize, size: usize, cap: usize| {
        let ok = *used + size <= cap;
        if ok {
            *used += size;
        }
        ok
    };
    let lead_cap = used + result.saturating_sub(used) / 2;
    let mut excerpts: Vec<Vec<Excerpt>> = Vec::with_capacity(output.files.len());
    let mut leads_fit = true;
    for (file, (file_excerpts, call_leads, leads)) in output.files.iter_mut().zip(parts) {
        excerpts.push(file_excerpts);
        for call in call_leads {
            leads_fit = leads_fit && fits(&mut used, result_bytes(&call), lead_cap);
            if leads_fit {
                file.call_leads.push(call);
            }
        }
        for lead in leads {
            leads_fit = leads_fit && fits(&mut used, result_bytes(&lead), lead_cap);
            if leads_fit {
                file.leads.push(lead);
            }
        }
    }
    trimmed |= !leads_fit;
    for (file, file_excerpts) in output.files.iter_mut().zip(excerpts) {
        for excerpt in file_excerpts {
            if excerpt.text.len() <= source && fits(&mut used, result_bytes(&excerpt), result) {
                source -= excerpt.text.len();
                file.excerpts.push(excerpt);
            } else {
                file.source_omitted = true;
            }
        }
    }
    if trimmed {
        *output.issues.entry("resource_limit".into()).or_default() += 1;
        if output.status == Status::Complete {
            output.status = Status::Incomplete;
        }
    }
}

/// repository-context.ts: `AGENTS.md` at the walk root and in every folder
/// above a returned file, when a listing admits it (the jail's protections,
/// ignore rules and `exclude_globs` all apply); a folder too large or
/// unreadable to tell counts `agents_md_incomplete`. Absolute. Blocking.
fn agents_md(run: &Run, candidates: &[Candidate]) -> Vec<String> {
    let mut directories = vec![".".to_string()];
    for candidate in candidates {
        let mut path = candidate.path.as_str();
        while let Some((parent, _)) = path.rsplit_once('/') {
            if !directories.iter().any(|known| known == parent) {
                directories.push(parent.to_string());
            }
            path = parent;
        }
    }
    directories
        .iter()
        .filter(|directory| {
            let listing = run.tree.list(directory, walk::MAX_ENTRIES);
            let found = listing
                .entries
                .iter()
                .any(|entry| !entry.is_dir && entry.name == "AGENTS.md");
            if !found && (listing.truncated || listing.unreadable) {
                run.issue("agents_md_incomplete");
            }
            found
        })
        .map(|directory| {
            run.tree
                .root
                .join(walk::join(directory, "AGENTS.md"))
                .display()
                .to_string()
        })
        .collect()
}

/// cache.ts's `promptVersion`: bump when a prompt's wording changes.
const PROMPT_VERSION: &str = "unit-locators-1";
/// The parsers behind every unit: keep in step with `Cargo.lock`.
const PARSER_VERSION: &str =
    "tree-sitter-0.24.7-python-0.23.6-go-0.23.4-rust-0.23.3-typescript-0.23.2";
const CACHE_BYTES: usize = 64 << 20;
const CACHE_ENTRY_BYTES: usize = 1 << 20;
/// Resident cost of an entry beyond its answers (map slot, queue key, one
/// B-tree leaf) and of each answer (its node share and key `String`).
const CACHE_ENTRY_OVERHEAD: usize = 512;
const CACHE_ANSWER_OVERHEAD: usize = 96;

/// Answers shared by every ask in this process (cache.ts, minus the disk,
/// the 7-day TTL and the cache issue counts).
// ponytail: first-in first-out eviction under the byte cap (jevgrep trims
// in directory-scan order, not LRU either); make it LRU if hit rates on
// long-lived workers call for it.
static CACHE: Lazy<Mutex<AnswerCache>> =
    Lazy::new(|| Mutex::new(AnswerCache::new(CACHE_BYTES, CACHE_ENTRY_BYTES)));

struct AnswerCache {
    entries: HashMap<[u8; 32], (Scores, usize)>,
    order: VecDeque<[u8; 32]>,
    bytes: usize,
    max_bytes: usize,
    max_entry_bytes: usize,
}

impl AnswerCache {
    fn new(max_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_bytes,
            max_entry_bytes,
        }
    }

    /// The cached answers when they answer exactly `questions`, each in
    /// [0, 1].
    fn get(&self, key: &[u8; 32], questions: &BTreeMap<String, Question>) -> Option<Scores> {
        let (scores, _) = self.entries.get(key)?;
        let valid = scores.len() == questions.len()
            && questions
                .keys()
                .all(|id| scores.get(id).is_some_and(|p| (0.0..=1.0).contains(p)));
        valid.then(|| scores.clone())
    }

    /// Keep `scores` unless one is out of range or the entry is over its
    /// cap (its JSON, as on jevgrep's disk); evict the oldest entries past
    /// the total, which counts resident bytes.
    fn put(&mut self, key: [u8; 32], scores: &Scores) {
        let size = CACHE_ENTRY_OVERHEAD + scores.len() * CACHE_ANSWER_OVERHEAD;
        if self.entries.contains_key(&key)
            || walk::json_len(scores) > self.max_entry_bytes
            || !scores.values().all(|p| (0.0..=1.0).contains(p))
        {
            return;
        }
        while self.bytes + size > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                return;
            };
            if let Some((_, freed)) = self.entries.remove(&oldest) {
                self.bytes -= freed;
            }
        }
        self.bytes += size;
        self.order.push_back(key);
        self.entries.insert(key, (scores.clone(), size));
    }
}

/// cache.ts `key`: sha256 of `[1, namespace, state, questions]`; `None`
/// (bypass the cache) if the request cannot be serialized.
// ponytail: the key names the session's provider but not the hub's model,
// nor, for `""`, the hub's default provider; a swap of either serves old
// answers until restart. Resolve both in `judge::window` if swaps happen.
fn cache_key(provider: &str, request: &Evaluation) -> Option<[u8; 32]> {
    let namespace = serde_json::json!({
        "provider": provider,
        "promptVersion": PROMPT_VERSION,
        "parserVersion": PARSER_VERSION,
    });
    let key = (1, namespace, &request.state, &request.questions);
    let bytes = serde_json::to_vec(&key).ok()?;
    Some(Sha256::digest(bytes).into())
}

fn cached(key: &[u8; 32], questions: &BTreeMap<String, Question>) -> Option<Scores> {
    CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(key, questions)
}

fn remember(key: [u8; 32], scores: &Scores) {
    CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .put(key, scores);
}

/// One line per ask; never the query or any path.
fn logged(output: FindRelevantOutput) -> FindRelevantOutput {
    tracing::info!(
        status = ?output.status,
        reason = output.reason.as_deref().unwrap_or(""),
        judge_calls = output.stats.judge_calls,
        questions = output.stats.questions,
        input_tokens = output.stats.input_tokens,
        cache_hits = output.stats.cache_hits,
        elapsed_ms = output.stats.elapsed_ms,
        files = output.files.len(),
        "coder::find-relevant"
    );
    output
}

#[cfg(test)]
mod tests;
