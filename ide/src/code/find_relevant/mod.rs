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
//!
//! jevgrep's contextual follow-up, relationship pass and Python test-body
//! selection are deliberately not ported. Measured in MOT-4965 against
//! blind gold labels, the follow-up spent up to 93% of an ask's judge
//! tokens for small coverage gains, the relationship pass never fired on
//! scoped asks, and test-body selection cut the key lines of questions
//! about tests.

pub mod navigate;
pub mod passes;
pub mod prompts;
pub mod select;
pub mod units;
pub mod walk;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use judge_contract::{Evaluation, Question};
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::judge::{self, Evaluator, JudgeError, Listing, Scores};
use crate::code::path::PathResolver;
use navigate::{Candidate, Run, Stop};

pub const MAX_QUERY_BYTES: usize = 4000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
/// Below the harness's 300 s `dispatch_timeout_ms`.
pub const MAX_TIMEOUT_MS: u64 = 280_000;
/// Smallest judge context window (tokens) whose states fit untruncated.
pub const MIN_WINDOW_TOKENS: u64 = 8_192;
/// Excerpt bytes one result carries at most; a lower
/// `code.max_output_bytes` (`coder::read-file`'s ceiling) lowers it.
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
    /// Folder of the component the question is about (default `.`, the
    /// whole root: on a large repo that ends `incomplete` with reason
    /// `token_budget`); result paths are absolute.
    #[serde(default = "default_path")]
    pub path: String,
    /// Globs (not gitignore lines) to leave out, relative to the session
    /// root like coder::search's, NOT to `path`: with path `ade`, write
    /// `ade/gen/**` or `**/gen/**`. `gen/` or `gen/**` drops the folder
    /// itself. They only narrow.
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
    240_000
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
    /// Every admitted branch was explored; with no files, nothing under
    /// `path` looked relevant (widen `path` or use coder::search).
    Complete,
    /// Partial coverage (deadline, judge token budget, a failed or
    /// oversized request, a resource limit): the answer may be in files
    /// not listed. `reason` names the main gap and `hint` the next step.
    Incomplete,
    /// No judge answered; use coder::search, or retry when `hint` says so.
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
    /// Locations of the declarations worth reading (score above 0.25),
    /// including those `excerpts` shows.
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
    /// Why the result is not complete: the stop (`deadline`,
    /// `token_budget`, the judge's failure) or else the leading kind in
    /// `issues`.
    pub reason: Option<String>,
    /// What to do next, when the result is partial, empty or unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Best first.
    pub files: Vec<RelevantFile>,
    /// AGENTS.md files from the project folder down to `path`, and above
    /// returned files.
    pub agents_md: Vec<String>,
    /// Coverage issues by kind, with counts: `deadline`, `token_budget`,
    /// `judge_call_timeout`, `request_size`, `resource_limit`,
    /// `source_inspection_limit` (narrow `path` or raise `timeout_ms`);
    /// `invalid_response`, `invalid_request`, `provider` (judge failures:
    /// retry later); `changed` (a file changed during the ask: retry);
    /// `unreadable`, `local_call_context`. `agents_md_incomplete` (the
    /// `agents_md` list may miss one) alone leaves the result complete.
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
    let slots = judge::slots(cfg.find_relevant_judge_slots as usize);
    let evaluate = judge::evaluator(iii.clone(), provider.clone(), slots);
    let listed = provider.clone();
    run(
        resolver,
        cfg,
        req,
        |deadline| async move { judge::window(&iii, listed.as_deref(), deadline).await },
        evaluate,
        provider,
    )
    .await
    .map_err(err_to_string)
}

/// One ask over any judge: `window` lists `provider`'s models (`None` = the
/// hub's default) once with the ask deadline, after the input is validated;
/// `evaluate` answers every request the answer cache cannot.
pub async fn run<W: Future<Output = Result<Listing, JudgeError>>>(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: FindRelevantInput,
    window: impl FnOnce(Instant) -> W,
    evaluate: Evaluator,
    provider: Option<String>,
) -> Result<FindRelevantOutput, CoderError> {
    let started = Instant::now();
    if req.query.trim().is_empty() {
        return Err(CoderError::BadInput(
            "query must not be empty; retry with the behavioural question in plain words, \
             e.g. \"where does the harness stamp the filesystem scope on coder calls?\""
                .into(),
        ));
    }
    if req.query.len() > MAX_QUERY_BYTES {
        return Err(CoderError::BadInput(format!(
            "query is {} bytes; at most {MAX_QUERY_BYTES}: retry with the question \
             alone, and use coder::search for long literal text",
            req.query.len()
        )));
    }
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&req.timeout_ms) {
        return Err(CoderError::BadInput(format!(
            "timeout_ms is {}; it must be within {MIN_TIMEOUT_MS}..={MAX_TIMEOUT_MS}: \
             retry with a value in that range, or omit it for the default {}",
            req.timeout_ms,
            default_timeout_ms()
        )));
    }
    let deadline = started + Duration::from_millis(req.timeout_ms);
    let walk_root = resolver
        .resolve_scope(req.fs_scope.as_ref(), &req.path)
        .map_err(|e| with_nearby_folders(&resolver, &cfg, &req, e))?;
    if walk::in_git_dir(&walk_root) {
        return Err(CoderError::BadInput(format!(
            "path is inside a .git directory, which is never searched: {}; retry with \
             the work tree folder above it",
            req.path
        )));
    }
    let md = std::fs::metadata(&walk_root).map_err(|e| {
        with_nearby_folders(&resolver, &cfg, &req, CoderError::io_for_path(e, &req.path))
    })?;
    if !md.is_dir() {
        let parent = Path::new(&req.path)
            .parent()
            .map(|p| p.display().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| ".".into());
        return Err(CoderError::BadInput(format!(
            "not a directory: {}; find-relevant searches a folder: retry with path \
             \"{parent}\", or read the file with coder::read-file",
            req.path
        )));
    }
    let project = project_folder(&resolver, req.fs_scope.as_ref(), &walk_root, &req.path)?;
    let exclude = crate::code::functions::search::build_globset(&req.exclude_globs)?;

    let unavailable = |reason: String| FindRelevantOutput {
        status: Status::Unavailable,
        reason: Some(reason),
        hint: None,
        files: Vec::new(),
        agents_md: Vec::new(),
        issues: BTreeMap::new(),
        stats: Stats {
            elapsed_ms: started.elapsed().as_millis() as u64,
            ..Stats::default()
        },
    };
    let listing = match window(deadline).await {
        Err(error) => return Ok(finish(unavailable(error.reason()))),
        Ok(Listing {
            window: Some(tokens),
            ..
        }) if tokens < MIN_WINDOW_TOKENS => {
            return Ok(finish(unavailable("judge window too small".into())))
        }
        Ok(listing) => listing,
    };
    // Answers are kept per provider and listed models, so a switched model
    // never serves old ones. A failed listing keeps a named provider's
    // answers by name alone; the hub default could switch unseen, so it
    // bypasses the cache.
    let cache = match (listing.models, provider) {
        (Some(models), provider) => {
            Some(serde_json::json!({ "provider": provider.unwrap_or_default(), "models": models }))
        }
        (None, Some(provider)) => Some(serde_json::json!({ "provider": provider, "models": null })),
        (None, None) => None,
    }
    .map(|namespace| namespace.to_string());

    let run = Arc::new(Run {
        query: req.query,
        tree: walk::Tree::new(
            &resolver,
            &walk_root,
            exclude,
            &project.base,
            cfg.max_read_bytes,
        ),
        evaluate,
        deadline,
        state_cap: select::MAX_STATE_BYTES,
        window: listing.window,
        cache,
        slots: (cfg.find_relevant_judge_slots as usize).clamp(1, judge::MAX_SLOTS),
        token_budget: cfg.find_relevant_judge_token_budget,
        state: Mutex::new(Default::default()),
    });
    run.discover(vec![".".into()]).await;
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
    let agents_md =
        tokio::task::spawn_blocking(move || agents_md(&lookup, &project.agents_base, &listed))
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
        None => match GAPS.iter().find(|kind| state.issues.contains_key(**kind)) {
            Some(kind) => (Status::Incomplete, Some(kind.to_string())),
            None => (Status::Complete, None),
        },
    };
    let mut output = FindRelevantOutput {
        status,
        reason,
        hint: None,
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
    let source = usize::try_from(cfg.max_output_bytes)
        .map_or(MAX_SOURCE_BYTES, |max| max.min(MAX_SOURCE_BYTES));
    spend_budget(&mut output, source, MAX_RESULT_BYTES);
    Ok(finish(output))
}

/// Issue kinds that leave coverage partial, the most telling first: an ask
/// that did not stop reports the first one present as its `reason`. A new
/// issue kind belongs here unless coverage stays whole without it.
const GAPS: [&str; 12] = [
    "deadline",
    "token_budget",
    "judge_call_timeout",
    "request_size",
    "resource_limit",
    "source_inspection_limit",
    "invalid_response",
    "invalid_request",
    "provider",
    "changed",
    "unreadable",
    "local_call_context",
];

/// The next step for the agent reading `output`; `None` for a complete
/// result with files.
fn hint(output: &FindRelevantOutput) -> Option<String> {
    let reason = output.reason.as_deref().unwrap_or_default();
    let hint = match output.status {
        Status::Complete if output.files.is_empty() => {
            "Nothing under path looked relevant to the judge: widen path, or use \
             coder::search for exact names."
                .to_string()
        }
        Status::Complete => return None,
        Status::Unavailable if reason == judge::LISTING_TIMEOUT => {
            "The judge did not list its models in time (a local judge may still be \
             loading its model): retry the ask in a minute, or use coder::search now."
                .to_string()
        }
        Status::Unavailable if reason == judge::PAUSED => format!(
            "The judge failed moments ago and is paused for up to {} s: use \
             coder::search, or retry the ask after that.",
            judge::PAUSE_MS / 1000
        ),
        Status::Unavailable => "No judge answered: use coder::search.".to_string(),
        Status::Incomplete => {
            let next = match reason {
                "deadline" | "judge_call_timeout" => {
                    " Narrow path, or retry with a larger timeout_ms."
                }
                "token_budget" | "request_size" | "resource_limit" | "source_inspection_limit" => {
                    " Narrow path for fuller coverage."
                }
                "changed" => " Retry the ask once the files stop changing.",
                "unreadable" | "local_call_context" => "",
                // A judge failure: `invalid_*`, `provider` or its reason.
                _ => " Retry the ask later.",
            };
            format!(
                "Coverage is partial ({reason}): the answer may be in files not listed, so \
                 verify with coder::search before relying on this list.{next}"
            )
        }
    };
    Some(hint)
}

/// The folders bounding an ask at a [`project_folder`].
struct Project {
    /// The session folder, else the Git work tree, granted folder or
    /// configured root holding the walk root: `exclude_globs` match from
    /// here.
    base: PathBuf,
    /// Where the `AGENTS.md` walk starts: `base`, or the granted folder
    /// below it that a session-less walk sits in (above a grant is outside
    /// the Workspace boundary, C220).
    agents_base: PathBuf,
}

/// The project folder bounding an ask at `walk_root` (named `wire`), or the
/// refusal (`C210`) when there is none or `walk_root` is hidden,
/// secret-named or gitignored. Blocking.
fn project_folder(
    resolver: &PathResolver,
    scope: Option<&crate::fs::FsScope>,
    walk_root: &Path,
    wire: &str,
) -> Result<Project, CoderError> {
    let session = crate::fs::scope_anchor(scope)
        .and_then(|root| resolver.session_root(root))
        .filter(|root| walk_root.starts_with(root));
    let top = walk::git_top(walk_root);
    // An unjailed worker's roots only anchor relative paths (`/tmp`, the
    // engine's folder), so they make no project folder.
    let configured = resolver
        .configured_root(walk_root)
        .filter(|_| !resolver.unjailed());
    // The project folder bounding the ask: the session's, else the Git work
    // tree's (inside the jail), else a granted folder, else the configured
    // root.
    let Some(base) = session.clone().or_else(|| {
        top.filter(|top| resolver.unjailed() || resolver.containing_root(top).is_some())
            .or_else(|| resolver.grant_root(walk_root))
            .or(configured)
            .map(Path::to_path_buf)
    }) else {
        return Err(CoderError::BadInput(format!(
            "find-relevant sends file text to the judge, so it only searches a project \
             folder (the session folder, a Git work tree, a granted folder or a jailed \
             worker's root, see coder::info), and {wire} is none; use coder::search"
        )));
    };
    // Hidden and secret-named folders count from a linked worktree's top at
    // or below `base` (it may sit under a dot-folder, like
    // .claude/worktrees), else the session folder, else the configured
    // root, else the project folder's parent: a dot-folder repository or
    // grant is still hidden, a repository inside one is not.
    let linked = top.filter(|top| top.starts_with(&base) && walk::linked_worktree(top));
    let trusted = linked.or(session.as_deref()).or(configured);
    let hidden_from = trusted.unwrap_or_else(|| base.parent().unwrap_or(&base));
    let hidden = walk_root.strip_prefix(hidden_from).is_ok_and(|rel| {
        rel.components().any(|c| {
            let name = c.as_os_str().to_string_lossy();
            name.starts_with('.') || walk::is_sensitive(&name)
        })
    });
    if hidden {
        return Err(CoderError::BadInput(format!(
            "path is a hidden or secret-named folder or inside one, which find-relevant \
             never searches: {wire}; use coder::search"
        )));
    }
    // Ignore rules count up to the outermost work tree below `trusted` (or
    // `/`), so a repository nested in an ignored folder is still ignored;
    // a linked worktree's own top bounds them.
    let outermost = walk_root
        .ancestors()
        .take_while(|dir| dir.starts_with(trusted.unwrap_or(Path::new("/"))))
        .filter(|dir| dir.join(".git").exists())
        .last();
    let bound = match (linked, &session, outermost) {
        (Some(top), _, _) => top,
        (None, Some(session), _) => session.as_path(),
        (None, None, Some(top)) if top.starts_with(&base) => &base,
        (None, None, Some(top)) => top,
        (None, None, None) => &base,
    };
    if walk::ignored(bound, walk_root) {
        return Err(CoderError::BadInput(format!(
            "path is gitignored or inside an ignored folder, which find-relevant never \
             searches: {wire}; use coder::search"
        )));
    }
    let agents_base = resolver
        .grant_root(walk_root)
        .filter(|grant| session.is_none() && grant.starts_with(&base))
        .map_or_else(|| base.clone(), Path::to_path_buf);
    Ok(Project { base, agents_base })
}

/// A C211 for `req.path` that also names up to five eligible folders
/// beside it, closest name first, when its parent passes
/// [`project_folder`] (each then under the walk's own gates: never hidden,
/// ignored or protected), so a retry with one is not refused. A relative
/// path on an unjailed worker without a session names its anchor instead,
/// the worker's own folder rather than a project. Any other error
/// unchanged. The same whether the path is missing or denied (REDACTION
/// INVARIANT). Blocking.
fn with_nearby_folders(
    resolver: &Arc<PathResolver>,
    cfg: &CoderConfig,
    req: &FindRelevantInput,
    error: CoderError,
) -> CoderError {
    let wire = Path::new(&req.path);
    let (CoderError::NotFoundOrDenied(_), Some(name), Some(parent)) =
        (&error, wire.file_name(), wire.parent())
    else {
        return error;
    };
    if req.fs_scope.is_none() && wire.is_relative() && resolver.unjailed() {
        return CoderError::not_found_or_denied_relative(&req.path, resolver.base_root());
    }
    let listed = match parent.to_string_lossy() {
        p if p.is_empty() => ".".to_string(),
        p => p.into_owned(),
    };
    let Ok(dir) = resolver.resolve_scope(req.fs_scope.as_ref(), &listed) else {
        return error;
    };
    if project_folder(resolver, req.fs_scope.as_ref(), &dir, &listed).is_err() {
        return error;
    }
    let name = name.to_string_lossy().to_lowercase();
    let mut near: Vec<(usize, String)> =
        walk::Tree::new(resolver, &dir, None, &dir, cfg.max_read_bytes)
            .list_path(&dir, walk::MAX_ENTRIES)
            .entries
            .into_iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| (edit_distance(&name, &entry.name.to_lowercase()), entry.name))
            .collect();
    // Stable: name order within a distance.
    near.sort_by_key(|(distance, _)| *distance);
    let near: Vec<String> = near
        .into_iter()
        .take(5)
        .map(|(_, folder)| parent.join(folder).display().to_string())
        .collect();
    if near.is_empty() {
        error
    } else {
        CoderError::not_found_or_denied_near(&req.path, &near)
    }
}

/// Levenshtein distance in chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
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
            output.reason = Some("resource_limit".into());
        }
    }
}

/// repository-context.ts: `AGENTS.md` in every folder from `base` down to
/// the walk root (jevgrep starts at the root, its repository) and above a
/// returned file, when a listing admits it (the jail's protections, ignore
/// rules and `exclude_globs` all apply); a folder too large or unreadable to
/// tell counts `agents_md_incomplete`. Absolute. Blocking.
fn agents_md(run: &Run, base: &Path, candidates: &[Candidate]) -> Vec<String> {
    let root = &run.tree.root;
    let mut directories: Vec<PathBuf> = root
        .ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(base))
        .map(Path::to_path_buf)
        .collect();
    directories.reverse();
    directories.push(root.clone());
    for candidate in candidates {
        let mut path = candidate.path.as_str();
        while let Some((parent, _)) = path.rsplit_once('/') {
            let parent_dir = root.join(parent);
            if !directories.contains(&parent_dir) {
                directories.push(parent_dir);
            }
            path = parent;
        }
    }
    directories
        .into_iter()
        .filter(|directory| {
            let listing = run.tree.list_path(directory, walk::MAX_ENTRIES);
            let found = listing
                .entries
                .iter()
                .any(|entry| !entry.is_dir && entry.name == "AGENTS.md");
            if !found && (listing.truncated || listing.unreadable) {
                run.issue("agents_md_incomplete");
            }
            found
        })
        .map(|directory| directory.join("AGENTS.md").display().to_string())
        .collect()
}

/// cache.ts's `promptVersion`: bump when a prompt's wording changes.
const PROMPT_VERSION: &str = "unit-locators-1";
/// The parsers behind every unit: keep in step with `Cargo.lock`.
const PARSER_VERSION: &str =
    "tree-sitter-0.25.10-python-0.23.6-go-0.23.4-rust-0.24.2-typescript-0.23.2";
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
fn cache_key(judge: &str, request: &Evaluation) -> Option<[u8; 32]> {
    let namespace = serde_json::json!({
        "judge": judge,
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

/// Adds the [`hint`] and logs one line per ask; never the query or any
/// path.
fn finish(mut output: FindRelevantOutput) -> FindRelevantOutput {
    output.hint = hint(&output);
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
