//! `directory::search_functions` — one-shot lexical function search — plus
//! the engine-catalog plumbing and the per-session registries behind it.
//!
//! Moved from the reflex spike; only the bm25 method came along (the model
//! consult stages measured token-equal and latency-worse — see the workers
//! repo docs/reflex-discover-findings.md).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use crate::config::{FunctionSearchMode, SharedConfig, SkillsConfig};
use crate::functions::registry::{
    self, RegistryCache, Worker, WorkerInfoInput, WorkerInfoOutput, WorkerListInput,
};
use crate::functions::search_index::{
    canonical_tools, compact_query, excluded_from_search, tool_fingerprint, Bm25Index, ToolSchema,
};
use crate::functions::search_semantic::{
    weighted_rrf, SemanticSearch, MODEL_REVISION, MODEL_SHA256,
};
use crate::surface::search_catalog as catalog;

/// Timeout for one engine catalog call during a refresh.
const CATALOG_TIMEOUT_MS: u64 = 5_000;
/// Function ids per `functions::info` batch — the engine's documented max.
const CATALOG_INFO_BATCH: usize = 32;
/// Workers returned by one `directory::search_functions` call.
const MAX_SEARCH_WORKERS: usize = 6;
/// Candidates returned by one call — the ranked guards usually select a
/// handful; this is the backstop.
const MAX_SEARCH_FUNCTIONS: usize = 12;
/// Namespace-level relative floor: a worker only stays when its BEST match
/// scores at least this fraction of the leader (calibrated on the live
/// catalog: junk tails sat at <=37% of a clear leader, ambiguous queries
/// score flat >=70%).
const SEARCH_RANK_FLOOR: f64 = 0.4;
/// Function-level score floor relative to the leader: anything under half
/// the leader's score is a tail. Family members riding the namespace token
/// plus one generic word sit at 44-57% of a name-boosted leader, while
/// genuinely co-relevant functions ("merge" + its "checks") score >= 85%.
const SEARCH_FN_FLOOR: f64 = 0.5;
/// Above this fraction of the leader a function stays even with lower term
/// coverage — the high-score keep that protects co-relevant companions.
const SEARCH_FN_KEEP: f64 = 0.85;
const SEMANTIC_WEIGHT: f64 = 0.75;
const SEMANTIC_MINIMUM_COSINE: f32 = 0.441_937_3;
/// Frozen v14 production policy: fuse the complete BM25 and embedding
/// rankings, then anchor the cross-encoder order back into retrieval.
const PRODUCTION_RETRIEVAL_WEIGHT: f64 = 1.0;
const PRODUCTION_RERANKER_WEIGHT: f64 = 1.25;
/// Fused retrieval candidates the cross-encoder scores per query. The tail
/// keeps its retrieval order: `weighted_rrf` only adds the reranker term to
/// ids present in its second list, so an unscored id can never outrank a
/// scored one. Sized from the 2026-09-02 benchmark, where the deepest first
/// relevant hit sat at fused rank 11 across 79 cases.
const PRODUCTION_RERANK_DEPTH: usize = 48;
/// Admission floor on the best MiniLM cosine of a query. Below it the query
/// keeps its BM25 result (empty for most no-match wording) instead of the
/// always-ranked dense list. Calibrated on the 2026-09-02 snapshot: the 64
/// match/multi cases bottom out at 0.315 (holdout 0.351); 12 of 15 no-match
/// cases sit at or below 0.302. The 0.015 margin is thin, so re-run
/// `record_admission_scores_per_stage` whenever the model or qrels change.
const PRODUCTION_ADMISSION_COSINE: f64 = 0.30;
/// Wall-clock budget for one cross-encoder call (all queries of a request).
/// On expiry the request serves the fused retrieval order; the blocking
/// rerank task itself is not cancelled and finishes in the background.
const PRODUCTION_RERANK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// Capability-sized searches accepted by one request.
const MAX_SEARCH_QUERIES: usize = 6;
/// Registry list queries per search: each capability, then informative
/// terms one by one — the registry's pg_trgm similarity misses long
/// natural-language queries that a single term ("email") hits. All
/// variants run concurrently, so the cap bounds registry load, not
/// latency; it must cover the informative terms of a complex capability
/// ("fetch a web page and send an email report" carries five).
const MAX_REGISTRY_QUERIES: usize = 6;
/// Registry candidates whose API reference is fetched (info round trips):
/// enough for every capability to contribute one worker.
const MAX_REGISTRY_CANDIDATES: usize = MAX_SEARCH_QUERIES;
/// Registry workers returned per search.
const MAX_INSTALLABLE_WORKERS: usize = 2;
/// Contracts returned across the whole `installable` section.
const MAX_INSTALLABLE_FUNCTIONS: usize = 6;
/// OTel baggage key the harness stamps on agent-dispatched calls. The value
/// is caller-supplied and unauthenticated: good enough to key a
/// resend-avoidance cache, never a security boundary.
const SESSION_BAGGAGE_KEY: &str = "iii.session.id";
static CATALOG_RELOAD: Mutex<()> = Mutex::const_new(());

pub type CatalogCell = Arc<RwLock<Arc<Vec<ToolSchema>>>>;

/// Shared dependencies for the search handler and the hint hook. The
/// registry search runs in-process through [`registry::worker_list`] /
/// [`registry::worker_info`] with the worker's shared config and cache —
/// no engine round trip.
#[derive(Clone)]
pub struct Deps {
    pub config: SharedConfig,
    pub catalog: CatalogCell,
    pub sessions: Arc<std::sync::Mutex<SessionRegistry>>,
    pub registry_cache: RegistryCache,
    pub semantic: SemanticSearch,
}

const SESSIONS_CAP: usize = 1024;

/// Per-session memory: which candidates search already delivered (repeat
/// queries omit them) and whether the hint fired in the current turn.
/// Session identity comes from caller-supplied OTel baggage or the hook
/// payload — a cache key, never a security boundary; a missing or wrong id
/// only costs a full resend or an extra hint.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<String, SessionRecord>,
    // ponytail: insertion-order eviction; move to LRU touch if hot sessions
    // ever churn out behind 1024 short-lived ones.
    order: VecDeque<String>,
}

#[derive(Debug, Default)]
struct SessionRecord {
    fingerprint: String,
    delivered: HashSet<String>,
    hint: Option<HintRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HintRecord {
    pub turn_id: String,
    pub step: u64,
    pub functions_generation: u64,
    pub expose: crate::hook::ExposeKind,
}

impl SessionRegistry {
    /// Split `selected` into (new, repeated) against what this session
    /// already received, then record the new ids. When *nothing* is new the
    /// whole selection is treated as new and re-recorded — an all-repeat
    /// query re-sends all candidates, which is the recovery path after
    /// compaction dropped the earlier result from the window.
    fn split(
        &mut self,
        session_id: &str,
        fingerprint: &str,
        selected: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let record = self.session_record(session_id, fingerprint);
        let (repeated, new): (Vec<String>, Vec<String>) = selected
            .iter()
            .cloned()
            .partition(|function_id| record.delivered.contains(function_id));
        if new.is_empty() && !repeated.is_empty() {
            return (repeated, Vec::new());
        }
        record.delivered.extend(new.iter().cloned());
        (new, repeated)
    }

    /// One hint per turn: `true` means send it now. A same-step replay
    /// re-sends; a new turn re-anchors a fresh hint; anything else stays
    /// silent.
    pub fn hint_decision(&mut self, session_id: &str, current: HintRecord) -> bool {
        // The hint decision must not depend on the catalog fingerprint, so
        // reuse whatever fingerprint the record already carries.
        let fingerprint = self
            .sessions
            .get(session_id)
            .map(|record| record.fingerprint.clone())
            .unwrap_or_default();
        let record = self.session_record(session_id, &fingerprint);
        match &record.hint {
            Some(sent) if sent.turn_id != current.turn_id => {
                record.hint = Some(current);
                true
            }
            Some(sent) if sent.turn_id == current.turn_id && current.step <= sent.step => true,
            Some(_) => false,
            None => {
                record.hint = Some(current);
                true
            }
        }
    }

    fn session_record(&mut self, session_id: &str, fingerprint: &str) -> &mut SessionRecord {
        if !self.sessions.contains_key(session_id) {
            if self.order.len() >= SESSIONS_CAP {
                if let Some(evicted) = self.order.pop_front() {
                    self.sessions.remove(&evicted);
                }
            }
            self.order.push_back(session_id.to_string());
        }
        let record = self.sessions.entry(session_id.to_string()).or_default();
        if !fingerprint.is_empty() && record.fingerprint != fingerprint {
            record.fingerprint = fingerprint.to_string();
            record.delivered.clear();
        }
        record
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnFunctionsChangeEvent {
    pub event: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct AckResponse {
    pub ok: bool,
}

fn function_namespace(function_id: &str) -> Option<&str> {
    let (namespace, _) = function_id.split_once("::")?;
    (!namespace.is_empty()).then_some(namespace)
}

fn intrinsic_capability(capability: &str) -> bool {
    let mut summary = false;
    let mut existing_content = false;
    for term in crate::functions::search_index::bm25_terms(capability) {
        summary |= matches!(
            term.as_str(),
            "summarize" | "summarise" | "summary" | "summarization"
        );
        existing_content |= matches!(term.as_str(), "provided" | "text" | "content");
    }
    summary && existing_content
}

const SEARCH_GUIDANCE: &str = "Only candidates relevant to the requested `capabilities` \
are listed. This result replaces engine::functions::list for these ids. The `workers` entries \
are INSTALLED candidates. Choose the smallest candidate set the task needs from `workers`, then \
BEFORE their first use fetch those contracts in ONE engine::functions::info call with \
{ \"function_ids\": [\"<selected id>\", \"<another selected id>\"] }. Call them with the \
returned request schemas, directly when the function is a tool in your surface, otherwise via \
agent_trigger. For capabilities absent from both `workers` and `installable`, call \
directory::search_functions again once at the next decision point with all unmet external \
capabilities in `capabilities`. \
Always write every `capabilities` entry in English, even when the user writes in \
another language; preserve proper names, URLs, and function IDs. Do not search for intrinsic \
reasoning, summarization, planning, or formatting, and do not repeat \
satisfied or already represented needs. If a selected id is rejected, fall back to \
normal discovery.";

const SEARCH_REFINE_GUIDANCE: &str =
    "No functions matched these capabilities. When the need is to BUILD something no function \
covers — authoring a worker, registering a new engine function, running custom code — the \
how-to lives in the shipped skills, not in a function: call directory::skills::list, then \
directory::skills::get { id: \"<id>\" } for the matching how-to, and follow it. Otherwise, at \
the next decision point, call directory::search_functions once with all unmet \
external capabilities in `capabilities`. Do not search for intrinsic reasoning, summarization, \
planning, or formatting, and do not repeat needs already represented in the conversation or \
already satisfied. Always write every `capabilities` entry in English, even when the \
user writes in another language; preserve proper names, URLs, and function IDs.";

const SEARCH_INSTALL_GUIDANCE: &str = "No INSTALLED function matched these capabilities. The \
`installable` entries are registry workers (verified authors) whose functions WOULD match, \
but they are NOT installed: calling their functions now FAILS with function_not_found. To \
use one, run its `install` call exactly as given (compose::add), wait for it to report the \
new worker ready, then call directory::search_functions again \
for the newly registered candidates and fetch selected contracts with one batched \
engine::functions::info call. If none fit, search once more with concrete unmet \
`capabilities`; for a need no function covers — authoring a worker, registering a new \
engine function — read the shipped how-to instead: directory::skills::list, then \
directory::skills::get { id: \"<id>\" }. Do not search for intrinsic reasoning, \
summarization, planning, or \
formatting. Always write every `capabilities` entry in English, even when the user \
writes in another language; preserve proper names, URLs, and function IDs.";

const SEARCH_INSTALL_NOTE: &str = "Select from `workers` before considering `installable`. The \
`installable` entries are registry workers that are NOT installed: calling their functions now \
FAILS with function_not_found. Do not pass an `installable` function ID to \
engine::functions::info. Only when no `workers` entry fits, FIRST call the provided install \
function with its payload (compose::add), wait for it to report the worker ready, then search \
again and fetch selected contracts with one batched \
engine::functions::info call — never call an installable function before installing.";

#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchFunctionsRequest {
    /// One to six non-empty capability searches derived from the goal and
    /// current execution state. For one search at each decision point, include all unmet
    /// external capabilities once. Exclude intrinsic reasoning, summarization, planning, or
    /// formatting, and do not repeat needs already represented or satisfied. Requests to
    /// summarize provided text or content are ignored. Write every entry in English,
    /// translating non-English user requests while preserving proper names, URLs, and function
    /// IDs.
    // MOT-4654: kept verbatim on purpose — this is a tuned search directive pinned
    // phrase-by-phrase by tests/search_schemas.rs, not descriptive prose.
    #[schemars(
        with = "HashSet<String>",
        length(min = 1, max = 6),
        inner(length(min = 1))
    )]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct FunctionCandidate {
    pub function_id: String,
    /// First description sentence, capped at 160 bytes.
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchWorker {
    pub namespace: String,
    pub functions: Vec<FunctionCandidate>,
}

/// A registry worker that is NOT installed but carries functions matching
/// the requested capabilities. `name` is the registry slug `compose::add` installs.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct InstallableWorker {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Compact candidates only. After installation, search again and fetch
    /// selected contracts through `engine::functions::info`.
    pub functions: Vec<FunctionCandidate>,
    /// The `compose::add` target and payload; agent-trigger callers add `description`.
    pub install: InstallCall,
}

/// A ready-made `compose::add` target and payload. Under agent-trigger exposure,
/// the caller still supplies the wrapper's user-facing `description`.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct InstallCall {
    pub function: String,
    pub payload: Value,
}

fn install_call(worker_name: &str) -> InstallCall {
    InstallCall {
        function: "compose::add".to_string(),
        payload: json!({ "worker": worker_name }),
    }
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchFunctionsResponse {
    pub guidance: String,
    pub workers: Vec<SearchWorker>,
    /// Matching registry workers from verified authors. Their functions are
    /// NOT callable until the worker is installed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub installable: Vec<InstallableWorker>,
    pub latency_ms: f64,
}

/// Drop every worker whose best-ranked function scores below
/// `relative_floor` of the leader. `ranked` is sorted best-first, so the
/// first occurrence of a namespace carries its best score; functions of a
/// surviving worker are kept whatever their own score.
fn drop_trailing_namespaces(ranked: Vec<(String, f64)>, relative_floor: f64) -> Vec<(String, f64)> {
    let Some(floor) = ranked
        .first()
        .map(|(_, top_score)| top_score * relative_floor)
    else {
        return ranked;
    };
    let mut kept: Vec<&str> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (function_id, score) in &ranked {
        let Some(namespace) = function_namespace(function_id) else {
            continue;
        };
        if seen.contains(&namespace) {
            continue;
        }
        seen.push(namespace);
        if *score >= floor {
            kept.push(namespace);
        }
    }
    let kept: Vec<String> = kept.into_iter().map(str::to_string).collect();
    ranked
        .into_iter()
        .filter(|(function_id, _)| {
            function_namespace(function_id)
                .is_some_and(|namespace| kept.iter().any(|surviving| surviving == namespace))
        })
        .collect()
}

/// Coverage-aware function pruning against the rank leader: a function
/// stays only when it scores at least SEARCH_FN_FLOOR of the leader AND
/// either matches at least as many distinct query terms as the leader or
/// scores SEARCH_FN_KEEP of it. Family members share the namespace token
/// plus one generic word (fewer matched terms, mid scores) and are dropped;
/// genuinely co-relevant functions either cover the query as fully as the
/// leader or score close to it.
fn drop_low_coverage(ranked: Vec<(String, f64, u32)>) -> Vec<(String, f64)> {
    let Some((_, leader_score, leader_matched)) = ranked.first().cloned() else {
        return Vec::new();
    };
    ranked
        .into_iter()
        .filter(|(_, score, matched)| {
            *score >= SEARCH_FN_FLOOR * leader_score
                && (*matched >= leader_matched || *score >= SEARCH_FN_KEEP * leader_score)
        })
        .map(|(function_id, score, _)| (function_id, score))
        .collect()
}

/// Rank each capability against its own leader, taking results round-robin so
/// every capability gets a candidate before any gets a rider. Scores from
/// separate rankings are not comparable.
fn lexical_rankings(index: &Bm25Index, queries: &[String]) -> Vec<Vec<(String, f64)>> {
    queries
        .iter()
        .map(|query| drop_low_coverage(index.rank_with_matches(query)))
        .collect()
}

fn round_robin_rankings(rankings: &[Vec<(String, f64)>], budget: usize) -> Vec<String> {
    let mut selected: Vec<String> = Vec::new();
    let mut round = 0;
    while selected.len() < budget {
        let mut any = false;
        for ranking in rankings {
            let Some((function_id, _)) = ranking.get(round) else {
                continue;
            };
            any = true;
            if !selected.contains(function_id) {
                selected.push(function_id.clone());
                if selected.len() == budget {
                    break;
                }
            }
        }
        if !any {
            break;
        }
        round += 1;
    }
    selected
}

fn needs_semantic(mode: FunctionSearchMode) -> bool {
    mode != FunctionSearchMode::Lexical
}

fn rankings_for_mode(
    mode: FunctionSearchMode,
    lexical: &[Vec<(String, f64)>],
    semantic: Option<Vec<Vec<(String, f64)>>>,
    semantic_weight: f64,
) -> Vec<Vec<(String, f64)>> {
    if mode != FunctionSearchMode::Hybrid {
        return lexical.to_vec();
    }
    let fused = match semantic {
        Some(semantic) if semantic.len() == lexical.len() => lexical
            .iter()
            .zip(semantic)
            .map(|(lexical, semantic)| {
                if semantic.is_empty() {
                    Vec::new()
                } else {
                    weighted_rrf(lexical, &semantic, semantic_weight)
                }
            })
            .collect(),
        _ => return lexical.to_vec(),
    };
    fused
}

fn exact_function_id(query: &str, tools: &[ToolSchema]) -> Option<String> {
    let query = query.trim();
    tools
        .iter()
        .any(|tool| tool.name == query)
        .then(|| query.to_owned())
}

fn production_fallback_rankings(
    tools: &[ToolSchema],
    queries: &[String],
    lexical: &[Vec<(String, f64)>],
) -> Vec<Vec<(String, f64)>> {
    let mut rankings = lexical.to_vec();
    for (position, query) in queries.iter().take(rankings.len()).enumerate() {
        if let Some(id) = exact_function_id(query, tools) {
            rankings[position] = vec![(id, 0.0)];
        }
    }
    rankings
}

/// Stage-1 admission: does the dense lane show enough affinity to let MiniLM
/// order this query at all?
fn production_admits(dense: &[(String, f64)]) -> bool {
    dense
        .iter()
        .map(|(_, cosine)| *cosine)
        .fold(f64::NEG_INFINITY, f64::max)
        >= PRODUCTION_ADMISSION_COSINE
}

/// Fused retrieval head handed to the cross-encoder.
fn production_rerank_head(retrieval: &[(String, f64)]) -> impl Iterator<Item = &str> {
    retrieval
        .iter()
        .take(PRODUCTION_RERANK_DEPTH)
        .map(|(id, _)| id.as_str())
}

/// Apply the frozen production ordering only when the reranker returns one
/// finite, unique score for every member of the fused retrieval head (the
/// first `PRODUCTION_RERANK_DEPTH` ids). The unscored tail keeps its
/// retrieval order below the head. Invalid model output is represented by
/// `None` so the caller can fail open to the lexical baseline.
fn production_minilm_ordering(
    lexical: &[(String, f64)],
    semantic: &[(String, f64)],
    raw_reranker: &[(String, f64)],
) -> Option<Vec<(String, f64)>> {
    let retrieval = weighted_rrf(lexical, semantic, PRODUCTION_RETRIEVAL_WEIGHT);
    let head_ids: HashSet<&str> = production_rerank_head(&retrieval).collect();
    let mut reranker_ids = HashSet::with_capacity(raw_reranker.len());
    if raw_reranker.len() != head_ids.len()
        || raw_reranker
            .iter()
            .any(|(id, score)| !score.is_finite() || !reranker_ids.insert(id.as_str()))
        || reranker_ids != head_ids
    {
        return None;
    }
    let mut reranker = raw_reranker.to_vec();
    reranker.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    Some(weighted_rrf(
        &retrieval,
        &reranker,
        PRODUCTION_RERANKER_WEIGHT,
    ))
}

struct ProductionMinilmOutcome {
    rankings: Vec<Vec<(String, f64)>>,
    complete: bool,
}

async fn production_minilm_rankings(
    semantic: &SemanticSearch,
    catalog_fingerprint: &str,
    tools: &[ToolSchema],
    queries: &[String],
    lexical: &[Vec<(String, f64)>],
) -> Option<ProductionMinilmOutcome> {
    if queries.len() != lexical.len() {
        return None;
    }
    let mut rankings = production_fallback_rankings(tools, queries, lexical);
    let model_positions: Vec<usize> = queries
        .iter()
        .enumerate()
        .filter_map(|(position, query)| {
            exact_function_id(query, tools)
                .is_none()
                .then_some(position)
        })
        .collect();
    if model_positions.is_empty() {
        return Some(ProductionMinilmOutcome {
            rankings,
            complete: true,
        });
    }
    let model_queries: Vec<String> = model_positions
        .iter()
        .map(|position| queries[*position].clone())
        .collect();
    let semantic_ranked = semantic
        .rank(catalog_fingerprint, &model_queries, -1.0)
        .await
        .ok()?;
    if semantic_ranked.len() != model_positions.len() {
        return None;
    }
    // Queries below the admission floor keep the BM25 fallback already in
    // `rankings`; only admitted queries are fused and reranked.
    let (model_positions, semantic_ranked): (Vec<usize>, Vec<Vec<(String, f64)>>) = model_positions
        .into_iter()
        .zip(semantic_ranked)
        .filter(|(_, dense)| production_admits(dense))
        .unzip();
    if model_positions.is_empty() {
        return Some(ProductionMinilmOutcome {
            rankings,
            complete: true,
        });
    }
    let model_queries: Vec<String> = model_positions
        .iter()
        .map(|position| queries[*position].clone())
        .collect();
    let candidate_ids: Vec<Vec<String>> = model_positions
        .iter()
        .zip(&semantic_ranked)
        .map(|(position, dense)| {
            let retrieval = weighted_rrf(&lexical[*position], dense, PRODUCTION_RETRIEVAL_WEIGHT);
            production_rerank_head(&retrieval)
                .map(str::to_owned)
                .collect()
        })
        .collect();
    let reranked = match tokio::time::timeout(
        PRODUCTION_RERANK_TIMEOUT,
        semantic.rerank(catalog_fingerprint, &model_queries, &candidate_ids),
    )
    .await
    {
        Ok(Ok(reranked)) if reranked.len() == model_positions.len() => Some(reranked),
        Ok(Ok(_)) => {
            tracing::warn!("production MiniLM reranker returned a lane count mismatch");
            None
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "production MiniLM reranker unavailable");
            None
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = PRODUCTION_RERANK_TIMEOUT.as_millis(),
                "production MiniLM reranker timed out"
            );
            None
        }
    };
    let complete = production_minilm_assemble(
        &mut rankings,
        &model_positions,
        lexical,
        &semantic_ranked,
        reranked.as_deref(),
    );
    Some(ProductionMinilmOutcome { rankings, complete })
}

/// Stage-2 assembly. Every model position first receives the fused
/// BM25+dense retrieval order, so a missing or invalid reranker lane degrades
/// to that order instead of to BM25 alone. Returns whether every position
/// received the complete reranked ordering.
fn production_minilm_assemble(
    rankings: &mut [Vec<(String, f64)>],
    model_positions: &[usize],
    lexical: &[Vec<(String, f64)>],
    semantic_ranked: &[Vec<(String, f64)>],
    reranked: Option<&[Vec<(String, f64)>]>,
) -> bool {
    let mut complete = true;
    for (index, (position, dense)) in model_positions.iter().zip(semantic_ranked).enumerate() {
        let retrieval = weighted_rrf(&lexical[*position], dense, PRODUCTION_RETRIEVAL_WEIGHT);
        let ordered = reranked
            .and_then(|lanes| lanes.get(index))
            .and_then(|raw| production_minilm_ordering(&lexical[*position], dense, raw))
            .filter(|ordered| {
                let ordered_ids: HashSet<&str> =
                    ordered.iter().map(|(id, _)| id.as_str()).collect();
                production_rerank_head(&retrieval).all(|id| ordered_ids.contains(id))
            });
        match ordered {
            Some(ordered) => rankings[*position] = ordered,
            None => {
                complete = false;
                rankings[*position] = retrieval;
            }
        }
    }
    complete
}

fn select_preordered_ids(rankings: Vec<Vec<(String, f64)>>) -> Vec<String> {
    let rankings = rankings
        .into_iter()
        .map(|ranking| drop_trailing_namespaces(ranking, SEARCH_RANK_FLOOR))
        .collect::<Vec<_>>();
    limit_search_workers(round_robin_rankings(&rankings, MAX_SEARCH_FUNCTIONS))
}

#[cfg(test)]
fn select_ranked_ids(
    mode: FunctionSearchMode,
    lexical: &[Vec<(String, f64)>],
    semantic: Option<Vec<Vec<(String, f64)>>>,
    semantic_weight: f64,
) -> Vec<String> {
    let rankings = rankings_for_mode(mode, lexical, semantic, semantic_weight)
        .into_iter()
        .map(|ranking| drop_trailing_namespaces(ranking, SEARCH_RANK_FLOOR))
        .collect::<Vec<_>>();
    limit_search_workers(round_robin_rankings(&rankings, MAX_SEARCH_FUNCTIONS))
}

fn search_queries(capabilities: &[String]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| compact_query(capability.trim()))
        .filter(|capability| !intrinsic_capability(capability))
        .collect()
}

#[cfg(test)]
fn hybrid_ranking_for_test(
    tools: &[ToolSchema],
    capabilities: &[String],
    mut raw_semantic: Vec<Vec<(String, f64)>>,
    minimum_cosine: f32,
    semantic_weight: f64,
) -> Vec<String> {
    let queries = search_queries(capabilities);
    let corpus = canonical_tools(tools);
    let lexical = lexical_rankings(&Bm25Index::build(&corpus), &queries);
    for ranking in &mut raw_semantic {
        ranking.retain(|(_, score)| *score >= f64::from(minimum_cosine));
    }
    let selected = select_ranked_ids(
        FunctionSearchMode::Hybrid,
        &lexical,
        Some(raw_semantic),
        semantic_weight,
    );
    assemble_workers(&selected, tools)
        .into_iter()
        .flat_map(|worker| {
            worker
                .functions
                .into_iter()
                .map(|function| function.function_id)
        })
        .collect()
}

/// The caller's harness session id from OTel baggage, when the dispatch
/// carried one. Caller-supplied and unauthenticated — a cache key for
/// resend avoidance, never a security boundary.
fn baggage_session_id() -> Option<String> {
    use opentelemetry::baggage::BaggageExt;
    let context = opentelemetry::Context::current();
    let session_id = context.baggage().get(SESSION_BAGGAGE_KEY)?.to_string();
    (!session_id.is_empty()).then_some(session_id)
}

fn limit_search_workers(selected: Vec<String>) -> Vec<String> {
    let mut namespaces: Vec<String> = Vec::new();
    selected
        .into_iter()
        .filter(|function_id| {
            let Some(namespace) = function_namespace(function_id) else {
                return false;
            };
            if namespaces.iter().any(|seen| seen == namespace) {
                return true;
            }
            if namespaces.len() == MAX_SEARCH_WORKERS {
                return false;
            }
            namespaces.push(namespace.to_string());
            true
        })
        .collect()
}

/// Group the selected function ids into compact candidates by worker,
/// keeping at most `MAX_SEARCH_WORKERS` workers. Ids missing from the catalog
/// are skipped; within a worker the rank order is preserved — best first.
fn assemble_workers(selected: &[String], tools: &[ToolSchema]) -> Vec<SearchWorker> {
    let mut workers: Vec<SearchWorker> = Vec::new();
    for function_id in selected {
        let Some(namespace) = function_namespace(function_id) else {
            continue;
        };
        let Some(tool) = tools.iter().find(|tool| &tool.name == function_id) else {
            continue;
        };
        let candidate = FunctionCandidate {
            function_id: tool.name.clone(),
            description: crate::functions::search_index::slim_description(&tool.description),
        };
        match workers
            .iter()
            .position(|worker| worker.namespace == namespace)
        {
            Some(index) => workers[index].functions.push(candidate),
            None if workers.len() < MAX_SEARCH_WORKERS => workers.push(SearchWorker {
                namespace: namespace.to_string(),
                functions: vec![candidate],
            }),
            None => {}
        }
    }
    workers
}

/// A registry candidate distilled from a `worker_list` row.
#[derive(Debug, Clone, PartialEq)]
struct RegistryCandidate {
    name: String,
    version: String,
    description: String,
}

/// Verified-author candidates from a registry list page, in the registry's
/// own relevance order. Unverified authors never surface: the section ends
/// in an install suggestion, and suggesting unvetted code is not this
/// worker's call.
fn registry_candidates(workers: &[Worker]) -> Vec<RegistryCandidate> {
    // This registry is private and every worker is team-authored, so all
    // listed workers are installable candidates — no author-verification
    // filter (which would drop first-party workers like browser/web that
    // carry no verified author).
    workers
        .iter()
        .map(|worker| RegistryCandidate {
            name: worker.name.clone(),
            version: worker.version.clone().unwrap_or_default(),
            description: worker.description.clone().unwrap_or_default(),
        })
        .collect()
}

/// The searchable contracts of one registry worker's `api_reference`:
/// internal functions, excluded namespaces, and functions already installed
/// (same id in the live catalog) are all skipped — the section only ever
/// offers what installing would actually add.
fn registry_contracts(info: &WorkerInfoOutput, installed: &HashSet<&str>) -> Vec<ToolSchema> {
    info.api_reference
        .functions
        .iter()
        .filter(|function| {
            function
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("internal"))
                .and_then(Value::as_bool)
                != Some(true)
        })
        .filter(|function| {
            !installed.contains(function.name.as_str()) && !excluded_from_search(&function.name)
        })
        .map(|function| ToolSchema {
            name: function.name.clone(),
            description: function.description.clone().unwrap_or_default(),
            parameters: function
                .request_schema
                .clone()
                .unwrap_or_else(|| json!({ "type": "object" })),
        })
        .collect()
}

/// Rank pooled registry contracts per capability: BM25 with the same
/// coverage pruning as the installed catalog, fused with the dense lane for
/// every capability MiniLM admits — so a capability sharing no vocabulary
/// with the contract ("retrieve web news articles" → `web::fetch`) still
/// surfaces an installable instead of dying on the two-term BM25 minimum.
fn rank_registry_contracts(
    search_queries: &[String],
    contracts: Vec<ToolSchema>,
    dense: Option<Vec<Vec<(String, f64)>>>,
    budget: usize,
) -> Vec<ToolSchema> {
    let corpus = canonical_tools(&contracts);
    let index = Bm25Index::build(&corpus);
    let rankings = fuse_admitted(lexical_rankings(&index, search_queries), dense);
    round_robin_rankings(&rankings, budget)
        .into_iter()
        .filter_map(|id| contracts.iter().find(|tool| tool.name == id).cloned())
        .collect()
}

/// Per capability: an admitted dense ranking (best cosine at or above the
/// production floor) is RRF-fused with BM25; a rejected or absent one leaves
/// BM25 alone — the installed catalog's admission rule, applied to ad-hoc
/// documents.
fn fuse_admitted(
    lexical: Vec<Vec<(String, f64)>>,
    dense: Option<Vec<Vec<(String, f64)>>>,
) -> Vec<Vec<(String, f64)>> {
    match dense {
        Some(dense) if dense.len() == lexical.len() => lexical
            .into_iter()
            .zip(dense)
            .map(|(lexical, dense)| {
                if production_admits(&dense) {
                    weighted_rrf(&lexical, &dense, PRODUCTION_RETRIEVAL_WEIGHT)
                } else {
                    lexical
                }
            })
            .collect(),
        _ => lexical,
    }
}

/// Registry list searches try every explicit/derived capability first, then
/// spend the remaining bounded slots on per-term fallbacks round-robin because
/// pg_trgm can miss long natural-language queries that a single term lands.
fn registry_queries(search_queries: &[String]) -> Vec<String> {
    let mut queries: Vec<String> = search_queries
        .iter()
        .take(MAX_REGISTRY_QUERIES)
        .cloned()
        .collect();
    let terms: Vec<Vec<String>> = search_queries
        .iter()
        .map(|query| crate::functions::search_index::bm25_terms(query).collect())
        .collect();
    let mut round = 0;
    while queries.len() < MAX_REGISTRY_QUERIES {
        let mut any = false;
        for terms in &terms {
            let Some(term) = terms.get(round) else {
                continue;
            };
            any = true;
            if !queries.contains(term) {
                queries.push(term.clone());
                if queries.len() == MAX_REGISTRY_QUERIES {
                    break;
                }
            }
        }
        if !any {
            break;
        }
        round += 1;
    }
    queries
}

fn round_robin_registry_candidates(
    lists: &[Vec<RegistryCandidate>],
    budget: usize,
) -> Vec<RegistryCandidate> {
    let mut candidates: Vec<RegistryCandidate> = Vec::new();
    let mut round = 0;
    while candidates.len() < budget {
        let mut any = false;
        for list in lists {
            let Some(candidate) = list.get(round) else {
                continue;
            };
            any = true;
            if candidates.len() == budget {
                break;
            }
            if !candidates.iter().any(|seen| seen.name == candidate.name) {
                candidates.push(candidate.clone());
            }
        }
        if !any {
            break;
        }
        round += 1;
    }
    candidates
}

/// Group ranked contracts back under their owning candidates, in ranked
/// order, capped at MAX_INSTALLABLE_WORKERS workers.
fn assemble_installable(
    ranked: Vec<ToolSchema>,
    owners: &HashMap<String, RegistryCandidate>,
) -> Vec<InstallableWorker> {
    let mut section: Vec<InstallableWorker> = Vec::new();
    for tool in ranked {
        let Some(candidate) = owners.get(&tool.name) else {
            continue;
        };
        let function = FunctionCandidate {
            function_id: tool.name.clone(),
            // First sentence only — enough to decide whether the worker
            // fits; the full contract arrives after installing.
            description: crate::functions::search_index::slim_description(&tool.description),
        };
        match section
            .iter()
            .position(|worker| worker.name == candidate.name)
        {
            Some(index) => section[index].functions.push(function),
            None if section.len() < MAX_INSTALLABLE_WORKERS => section.push(InstallableWorker {
                name: candidate.name.clone(),
                version: candidate.version.clone(),
                description: candidate.description.clone(),
                functions: vec![function],
                install: install_call(&candidate.name),
            }),
            None => {}
        }
    }
    section
}

/// Dense ranking of ad-hoc documents through the production MiniLM lane;
/// `None` when the lane is off for this search or unavailable, so the caller
/// stays lexical (fail-open, like every other registry step).
async fn registry_dense_rankings(
    semantic: Option<&SemanticSearch>,
    search_queries: &[String],
    documents: &[ToolSchema],
    minimum_cosine: f32,
) -> Option<Vec<Vec<(String, f64)>>> {
    match semantic?
        .rank_documents(search_queries, documents, minimum_cosine)
        .await
    {
        Ok(rankings) => Some(rankings),
        Err(error) => {
            tracing::debug!(%error, "registry search: dense lane unavailable; lexical only");
            None
        }
    }
}

/// Pull the candidates' API references, pool their contracts (first-seen
/// owner wins), rank per capability and group back under the owning workers.
async fn installable_from_candidates(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    semantic: Option<&SemanticSearch>,
    installed: &[ToolSchema],
    search_queries: &[String],
    candidates: &[RegistryCandidate],
) -> Vec<InstallableWorker> {
    // Info round trips concurrently; pooling stays in candidate order
    // so first-seen contract dedupe is deterministic.
    let mut infos = tokio::task::JoinSet::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let cfg = cfg.clone();
        let cache = cache.clone();
        let name = candidate.name.clone();
        infos.spawn(async move {
            let result = registry::worker_info(
                &cfg,
                &cache,
                WorkerInfoInput {
                    name,
                    ..WorkerInfoInput::default()
                },
            )
            .await;
            (index, result)
        });
    }
    let mut info_responses: Vec<(usize, WorkerInfoOutput)> = Vec::new();
    while let Some(joined) = infos.join_next().await {
        match joined {
            Ok((index, Ok(info))) => info_responses.push((index, info)),
            Ok((_, Err(error))) => {
                tracing::warn!(%error, "registry search: info failed; skipping candidate")
            }
            Err(error) => tracing::warn!(%error, "registry search: info task failed; skipping"),
        }
    }
    info_responses.sort_by_key(|(index, _)| *index);
    let installed: HashSet<&str> = installed.iter().map(|tool| tool.name.as_str()).collect();
    let mut owners: HashMap<String, RegistryCandidate> = HashMap::new();
    let mut pooled: Vec<ToolSchema> = Vec::new();
    for (index, info) in &info_responses {
        let candidate = &candidates[*index];
        for contract in registry_contracts(info, &installed) {
            if !owners.contains_key(&contract.name) {
                owners.insert(contract.name.clone(), candidate.clone());
                pooled.push(contract);
            }
        }
    }
    let dense = registry_dense_rankings(semantic, search_queries, &pooled, -1.0).await;
    let ranked = rank_registry_contracts(search_queries, pooled, dense, MAX_INSTALLABLE_FUNCTIONS);
    assemble_installable(ranked, &owners)
}

/// Upper bound on registry pages one walk reads (75 workers in 4 pages of
/// 20 today).
const MAX_REGISTRY_WALK_PAGES: usize = 10;

/// Every worker the private registry lists, page by page; a failing page
/// keeps what was read before it.
async fn registry_walk(cfg: &SkillsConfig, cache: &RegistryCache) -> Vec<Worker> {
    let mut workers = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_REGISTRY_WALK_PAGES {
        let page = match registry::worker_list(
            cfg,
            cache,
            WorkerListInput {
                search: None,
                cursor: cursor.take(),
            },
        )
        .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "registry search: walk page failed; ranking what was read");
                break;
            }
        };
        workers.extend(page.workers);
        if !page.pagination.has_more {
            break;
        }
        cursor = page.pagination.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    workers
}

/// Stage-2 acquisition. The registry's own search is trigram-based, so a
/// capability sharing no term with any worker ("latest headlines") never
/// yields a candidate. Rank every not-yet-tried, not-installed worker's name
/// and description through the dense lane instead and take the admitted best
/// per capability.
async fn registry_walk_candidates(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    semantic: &SemanticSearch,
    installed_namespaces: &HashSet<&str>,
    search_queries: &[String],
    tried: &[RegistryCandidate],
) -> Vec<RegistryCandidate> {
    let workers = registry_walk(cfg, cache).await;
    let candidates: Vec<RegistryCandidate> = registry_candidates(&workers)
        .into_iter()
        .filter(|candidate| {
            !installed_namespaces.contains(candidate.name.as_str())
                && !tried.iter().any(|seen| seen.name == candidate.name)
        })
        .collect();
    let documents: Vec<ToolSchema> = candidates
        .iter()
        .map(|candidate| ToolSchema {
            name: candidate.name.clone(),
            description: candidate.description.clone(),
            parameters: json!({ "type": "object" }),
        })
        .collect();
    let Some(rankings) = registry_dense_rankings(
        Some(semantic),
        search_queries,
        &documents,
        PRODUCTION_ADMISSION_COSINE as f32,
    )
    .await
    else {
        return Vec::new();
    };
    round_robin_rankings(&rankings, MAX_REGISTRY_CANDIDATES)
        .into_iter()
        .filter_map(|name| {
            candidates
                .iter()
                .find(|candidate| candidate.name == name)
                .cloned()
        })
        .collect()
}

/// The installable side of a search: ask the private registry with each
/// capability, pull the top candidates' API references, and rank their
/// functions per capability. Candidates whose name is already installed are
/// skipped. When that offers nothing and the dense lane is on, stage 2 walks
/// the whole registry through the dense lane once. Every failure — registry
/// down, malformed payload, no matches — returns an empty section: the
/// search itself must never error over this.
async fn registry_installable(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    semantic: Option<&SemanticSearch>,
    installed: &[ToolSchema],
    search_queries: &[String],
) -> Vec<InstallableWorker> {
    if search_queries.is_empty() {
        return Vec::new();
    }
    // All list variants concurrently: a cold registry costs one timeout,
    // not one per variant, and no variant's hits starve another's — the
    // global ranking dedupes. Query order still decides candidate priority.
    // In-process calls: same client, cache,
    // and error hygiene as `directory::registry::workers::list`.
    let mut lists = tokio::task::JoinSet::new();
    for (priority, list_query) in registry_queries(search_queries).into_iter().enumerate() {
        let cfg = cfg.clone();
        let cache = cache.clone();
        lists.spawn(async move {
            let result = registry::worker_list(
                &cfg,
                &cache,
                WorkerListInput {
                    search: Some(list_query),
                    cursor: None,
                },
            )
            .await;
            (priority, result)
        });
    }
    let mut responses: Vec<(usize, Vec<Worker>)> = Vec::new();
    while let Some(joined) = lists.join_next().await {
        match joined {
            Ok((priority, Ok(list))) => responses.push((priority, list.workers)),
            Ok((_, Err(error))) => {
                tracing::warn!(%error, "registry search: list variant failed; skipping")
            }
            Err(error) => tracing::warn!(%error, "registry search: list task failed; skipping"),
        }
    }
    responses.sort_by_key(|(priority, _)| *priority);
    let installed_namespaces: HashSet<&str> = installed
        .iter()
        .filter_map(|tool| function_namespace(&tool.name))
        .collect();
    // Round-robin across the variants' result lists: every query intent
    // claims a candidate slot before any variant claims its second, so a
    // multi-intent query's later capabilities ("… and send an email") are
    // not starved out of the cap by the first one's lookalikes.
    let variant_lists: Vec<Vec<RegistryCandidate>> = responses
        .iter()
        .map(|(_, workers)| {
            registry_candidates(workers)
                .into_iter()
                .filter(|candidate| !installed_namespaces.contains(candidate.name.as_str()))
                .collect()
        })
        .collect();
    let candidates = round_robin_registry_candidates(&variant_lists, MAX_REGISTRY_CANDIDATES);
    let section =
        installable_from_candidates(cfg, cache, semantic, installed, search_queries, &candidates)
            .await;
    if !section.is_empty() {
        return section;
    }
    let Some(dense) = semantic else {
        return section;
    };
    let walked = registry_walk_candidates(
        cfg,
        cache,
        dense,
        &installed_namespaces,
        search_queries,
        &candidates,
    )
    .await;
    if walked.is_empty() {
        return section;
    }
    tracing::debug!(
        candidates = ?walked.iter().map(|candidate| candidate.name.as_str()).collect::<Vec<_>>(),
        "registry search: keyword acquisition offered nothing; dense walk candidates"
    );
    installable_from_candidates(cfg, cache, semantic, installed, search_queries, &walked).await
}

/// One-shot lexical search: rank the catalog with BM25 and return compact
/// candidates for only the ranked functions — never a whole worker.
pub async fn search_functions(
    deps: &Deps,
    request: SearchFunctionsRequest,
) -> Result<SearchFunctionsResponse, Error> {
    if request.capabilities.is_empty() {
        return Err(Error::Handler("provide at least one capability".into()));
    }
    if request.capabilities.len() > MAX_SEARCH_QUERIES {
        return Err(Error::Handler(format!(
            "provide at most {MAX_SEARCH_QUERIES} capabilities"
        )));
    }
    if request
        .capabilities
        .iter()
        .any(|capability| capability.trim().is_empty())
    {
        return Err(Error::Handler("capabilities must not be blank".into()));
    }
    let started = Instant::now();
    let tools = deps.catalog.read().await.clone();
    let search_queries = search_queries(&request.capabilities);
    let cfg = deps.config.load_full();
    let mode = cfg.function_search_mode;
    let lexical_started = Instant::now();
    // ponytail: index rebuilt per call (~250 slim docs, sub-ms); cache by
    // tool_fingerprint if search latency ever matters.
    let corpus = canonical_tools(&tools);
    let index = Bm25Index::build(&corpus);
    let lexical = lexical_rankings(&index, &search_queries);
    let fingerprint = tool_fingerprint(&tools);
    let lexical_top_ids: Vec<&str> = lexical
        .iter()
        .filter_map(|ranking| ranking.first().map(|(id, _)| id.as_str()))
        .collect();
    tracing::debug!(
        ?mode,
        %fingerprint,
        elapsed_ms = lexical_started.elapsed().as_secs_f64() * 1000.0,
        ?lexical_top_ids,
        "lexical lane ranked"
    );
    let production_minilm =
        mode == FunctionSearchMode::Hybrid && deps.semantic.is_production_minilm();
    let production_outcome = if production_minilm {
        let semantic_started = Instant::now();
        let rankings = production_minilm_rankings(
            &deps.semantic,
            &fingerprint,
            &tools,
            &search_queries,
            &lexical,
        )
        .await;
        tracing::debug!(
            ?mode,
            available = rankings.is_some(),
            complete = rankings.as_ref().is_some_and(|outcome| outcome.complete),
            %fingerprint,
            repository = deps.semantic.model_repository(),
            revision = deps.semantic.model_revision(),
            reranker_repository = deps.semantic.reranker_repository(),
            reranker_revision = deps.semantic.reranker_revision(),
            elapsed_ms = semantic_started.elapsed().as_secs_f64() * 1000.0,
            "production MiniLM retrieval and reranking completed"
        );
        rankings
    } else {
        None
    };
    let production_rankings = production_outcome.map(|outcome| outcome.rankings);
    let semantic = if needs_semantic(mode) && !production_minilm {
        let semantic_started = Instant::now();
        match deps
            .semantic
            .rank(&fingerprint, &search_queries, SEMANTIC_MINIMUM_COSINE)
            .await
        {
            Ok(rankings) => {
                let top_ids: Vec<&str> = rankings
                    .iter()
                    .filter_map(|ranking| ranking.first().map(|(id, _)| id.as_str()))
                    .collect();
                tracing::debug!(
                    ?mode,
                    available = true,
                    %fingerprint,
                    revision = MODEL_REVISION,
                    model_sha256 = MODEL_SHA256,
                    elapsed_ms = semantic_started.elapsed().as_secs_f64() * 1000.0,
                    ?top_ids,
                    "semantic lane ranked"
                );
                Some(rankings)
            }
            Err(error) => {
                tracing::debug!(
                    %error,
                    ?mode,
                    available = false,
                    %fingerprint,
                    revision = MODEL_REVISION,
                    model_sha256 = MODEL_SHA256,
                    elapsed_ms = semantic_started.elapsed().as_secs_f64() * 1000.0,
                    "semantic lane fell back to lexical"
                );
                None
            }
        }
    } else {
        tracing::debug!(?mode, available = false, %fingerprint, "semantic lane bypassed");
        None
    };
    let mut selected = match production_rankings {
        Some(rankings) => select_preordered_ids(rankings),
        None if production_minilm => select_preordered_ids(production_fallback_rankings(
            &tools,
            &search_queries,
            &lexical,
        )),
        None => select_preordered_ids(production_fallback_rankings(
            &tools,
            &search_queries,
            &rankings_for_mode(mode, &lexical, semantic, SEMANTIC_WEIGHT),
        )),
    };
    let session_id = baggage_session_id();
    // Repeat queries in one session skip candidates the session already
    // received (session identity from caller baggage; absent → full
    // response, fail-open).
    let mut repeated: Vec<String> = Vec::new();
    if let Some(session_id) = session_id.as_deref() {
        let (new, prior) = deps.sessions.lock().expect("delivered registry").split(
            session_id,
            &fingerprint,
            &selected,
        );
        selected = new;
        repeated = prior;
    }
    tracing::debug!(?mode, %fingerprint, top_ids = ?selected, "function search selected");
    let workers = assemble_workers(&selected, &tools);
    // Installable section: every search also consults the public registry
    // for NOT-installed workers whose functions match — behind the
    // registry_search knob; every failure inside returns an empty section
    // (fail-open).
    let mut installable: Vec<InstallableWorker> = Vec::new();
    if cfg.registry_search {
        installable = registry_installable(
            &cfg,
            &deps.registry_cache,
            production_minilm.then_some(&deps.semantic),
            &tools,
            &search_queries,
        )
        .await;
    }
    let guidance = if workers.is_empty() && repeated.is_empty() {
        if installable.is_empty() {
            SEARCH_REFINE_GUIDANCE.to_string()
        } else {
            SEARCH_INSTALL_GUIDANCE.to_string()
        }
    } else {
        let mut guidance = SEARCH_GUIDANCE.to_string();
        if !repeated.is_empty() {
            guidance = format!(
                "{guidance} Already provided earlier in this session (candidates \
unchanged — reuse the earlier result): {}.",
                repeated.join(", ")
            );
        }
        if !installable.is_empty() {
            guidance = format!("{guidance} {SEARCH_INSTALL_NOTE}");
        }
        guidance
    };
    Ok(SearchFunctionsResponse {
        guidance,
        workers,
        installable,
        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

fn listed_ids(value: &Value) -> Result<Vec<String>, String> {
    let items = match value {
        Value::Array(items) => items,
        Value::Object(object) => object
            .get("functions")
            .or_else(|| object.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| "catalog list response was malformed".to_string())?,
        _ => return Err("catalog list response was malformed".into()),
    };
    let mut ids = items
        .iter()
        .map(|item| {
            item.as_str()
                .or_else(|| {
                    item.get("function_id")
                        .or_else(|| item.get("id"))
                        .or_else(|| item.get("name"))
                        .and_then(Value::as_str)
                })
                .map(str::to_string)
                .ok_or_else(|| "catalog list response was malformed".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = HashSet::with_capacity(ids.len());
    if ids.iter().any(|id| !seen.insert(id.as_str())) {
        return Err("catalog list contained duplicate function id".into());
    }
    ids.retain(|id| !excluded_from_search(id));
    Ok(ids)
}

/// Pool one `functions::info` batch response's entries by function id.
/// Each entry is either a FunctionDetail or `{ function_id, error }` for an
/// id this caller cannot see; both carry the id, so association survives
/// any ordering.
fn pool_info_entries(response: &Value, pool: &mut HashMap<String, Value>) {
    let Some(functions) = response.get("functions").and_then(Value::as_array) else {
        tracing::warn!("catalog info batch response was malformed; skipping batch");
        return;
    };
    for entry in functions {
        let Some(id) = entry
            .get("function_id")
            .or_else(|| entry.get("id"))
            .or_else(|| entry.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        pool.insert(id.to_string(), entry.clone());
    }
}

/// Build the catalog from pooled per-function entries, in listed-id order.
/// Resilient by design: an errored, missing, or malformed entry only drops
/// THAT function — the surviving contracts still make a working catalog.
fn catalog_from_entries(ids: &[String], entries: &HashMap<String, Value>) -> Vec<ToolSchema> {
    ids.iter()
        .filter_map(|id| {
            let entry = entries.get(id)?;
            // Per-entry batch errors ({ function_id, error: "forbidden" |
            // "not_found" }) mean this caller cannot see the function.
            if entry.get("error").is_some() {
                return None;
            }
            // Internal plumbing (hooks, config handlers, on-change
            // listeners) is not a capability an agent should discover —
            // exclusion is by metadata, not by namespace, so the worker's
            // own public directory::* functions stay searchable.
            if entry
                .get("metadata")
                .and_then(|metadata| metadata.get("internal"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                return None;
            }
            Some(ToolSchema {
                name: id.clone(),
                description: entry
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parameters: entry
                    .get("parameters")
                    .or_else(|| entry.get("request_format"))
                    .or_else(|| entry.get("request_schema"))
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object" })),
            })
        })
        .collect()
}

pub async fn fetch_catalog(iii: &IIIClient) -> Result<Vec<ToolSchema>, String> {
    let list = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(CATALOG_TIMEOUT_MS),
        })
        .await
        .map_err(|_| "catalog list failed".to_string())?;
    let ids = listed_ids(&list)?;
    // Contracts arrive in `functions::info` batches (`function_ids`, engine
    // max 32 per call) fired concurrently — fan-out is bounded to
    // ceil(ids/32) in-flight calls, each under the same per-call timeout.
    let mut batches = tokio::task::JoinSet::new();
    for chunk in ids.chunks(CATALOG_INFO_BATCH) {
        let iii = iii.clone();
        let chunk: Vec<String> = chunk.to_vec();
        batches.spawn(async move {
            iii.trigger(TriggerRequest {
                function_id: "engine::functions::info".into(),
                payload: json!({ "function_ids": chunk }),
                action: None,
                timeout_ms: Some(CATALOG_TIMEOUT_MS),
            })
            .await
        });
    }
    let mut entries: HashMap<String, Value> = HashMap::with_capacity(ids.len());
    while let Some(joined) = batches.join_next().await {
        match joined {
            Ok(Ok(response)) => pool_info_entries(&response, &mut entries),
            Ok(Err(error)) => {
                tracing::warn!(%error, "catalog info batch failed; skipping its functions")
            }
            Err(error) => {
                tracing::warn!(%error, "catalog info batch task failed; skipping its functions")
            }
        }
    }
    let catalog = catalog_from_entries(&ids, &entries);
    // A listed-but-empty catalog means every batch failed: erroring keeps
    // the previous catalog live instead of activating an empty one.
    if catalog.is_empty() && !ids.is_empty() {
        return Err("catalog info failed for every function".into());
    }
    Ok(catalog)
}

/// Swap the catalog cell when the fingerprint changed; `true` = changed.
async fn activate_catalog(
    cell: &CatalogCell,
    semantic: &SemanticSearch,
    tools: Vec<ToolSchema>,
) -> bool {
    let catalog_unchanged =
        tool_fingerprint(cell.read().await.as_ref()) == tool_fingerprint(&tools);
    if catalog_unchanged {
        return false;
    }
    let tools = Arc::new(tools);
    *cell.write().await = tools.clone();
    semantic.rebuild(tools);
    true
}

pub async fn refresh_catalog(
    iii: &IIIClient,
    cell: &CatalogCell,
    semantic: &SemanticSearch,
) -> Result<bool, String> {
    let iii = iii.clone();
    let cell = cell.clone();
    let semantic = semantic.clone();
    match tokio::spawn(async move {
        let _reload = CATALOG_RELOAD.lock().await;
        let tools = fetch_catalog(&iii)
            .await
            .map_err(|_| "catalog_fetch_failed".to_string())?;
        Ok(activate_catalog(&cell, &semantic, tools).await)
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Err("catalog_refresh_task_failed".into()),
    }
}

/// Always-on trigger bindings: the engine's functions-available push keeps
/// the catalog fresh. The pre-generate hint binding is NOT here — it
/// follows the `inject_hint` config knob (crate::hook::apply).
pub fn required_bindings() -> Vec<(&'static str, &'static str, Value)> {
    vec![(
        "engine::functions-available",
        "directory::on-functions-change",
        json!({}),
    )]
}

pub fn bind_best_effort(iii: &IIIClient) {
    for (trigger_type, function_id, config) in required_bindings() {
        if let Err(error) =
            iii.register_trigger(RegisterTriggerInput::new(trigger_type, function_id, config))
        {
            tracing::warn!(%trigger_type, %function_id, %error, "trigger binding failed");
        }
    }
}

/// Register the public search function, the internal catalog listener, and
/// the internal pre-generate hook.
pub fn register(iii: &Arc<IIIClient>, deps: &Deps) {
    let specs = catalog();

    let search_deps = deps.clone();
    iii.register_function(
        specs[0].function_id,
        RegisterFunction::new_async(move |request: SearchFunctionsRequest| {
            let deps = search_deps.clone();
            async move { search_functions(&deps, request).await }
        })
        .description(specs[0].description),
    );

    let hook_deps = deps.clone();
    iii.register_function(
        specs[1].function_id,
        RegisterFunction::new_async(move |request: crate::hook::PreGenerateHookRequest| {
            let deps = hook_deps.clone();
            async move { Ok::<_, Error>(crate::hook::pre_generate(&deps, request).await) }
        })
        .description(specs[1].description)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    iii.register_function(
        specs[3].function_id,
        RegisterFunction::new_async(
            move |_request: crate::hook::HintPreviewRequest| async move {
                Ok::<_, Error>(crate::hook::hint_preview())
            },
        )
        .description(specs[3].description)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let refresh_iii = iii.clone();
    let refresh_cell = deps.catalog.clone();
    let refresh_semantic = deps.semantic.clone();
    iii.register_function(
        specs[2].function_id,
        RegisterFunction::new_async(move |_event: OnFunctionsChangeEvent| {
            let iii = refresh_iii.clone();
            let cell = refresh_cell.clone();
            let semantic = refresh_semantic.clone();
            async move {
                let changed = refresh_catalog(&iii, &cell, &semantic)
                    .await
                    .map_err(Error::Handler)?;
                if changed {
                    let functions = cell.read().await.len();
                    tracing::info!(functions, "discovery catalog refreshed");
                }
                Ok::<_, Error>(AckResponse { ok: true })
            }
        })
        .description(specs[2].description)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::search_index::SEARCH_FN;
    use crate::hook::ExposeKind;

    #[test]
    fn excluded_from_search_hides_claim_namespace_and_infra() {
        assert!(excluded_from_search(SEARCH_FN));
        assert!(excluded_from_search("engine::functions::list"));
        assert!(excluded_from_search("state::claim-namespace"));
        assert!(
            excluded_from_search("state::on-config-change"),
            "config-reload handlers are internal by convention"
        );
        assert!(excluded_from_search("codex::on-config-change"));
        assert!(!excluded_from_search("state::set"));
        assert!(!excluded_from_search("state::claim"));
        assert!(!excluded_from_search("state::on-config"));
    }

    #[test]
    fn canonical_tools_drops_excluded_ids() {
        let tools = vec![
            ToolSchema {
                name: "state::claim-namespace".into(),
                description: "claim a namespace".into(),
                parameters: json!({ "type": "object" }),
            },
            ToolSchema {
                name: "state::set".into(),
                description: "set a value".into(),
                parameters: json!({ "type": "object" }),
            },
        ];
        let kept: Vec<String> = canonical_tools(&tools)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(kept, vec!["state::set".to_string()]);
    }

    fn search_deps(tools: Vec<ToolSchema>) -> Deps {
        let config = SkillsConfig {
            registry_search: false,
            ..SkillsConfig::default()
        };
        Deps {
            config: config.into_shared(),
            catalog: Arc::new(RwLock::new(Arc::new(tools))),
            sessions: Arc::default(),
            registry_cache: RegistryCache::new(std::time::Duration::ZERO),
            semantic: SemanticSearch::default(),
        }
    }

    #[test]
    fn assembled_search_results_are_slim_candidates() {
        let tools = vec![ToolSchema {
            name: "github::pr::create".into(),
            description: "Create a pull request? This sentence is contract detail.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "draft": { "type": "boolean" } }
            }),
        }];

        let workers = assemble_workers(&["github::pr::create".into()], &tools);
        let candidate = serde_json::to_value(&workers[0].functions[0]).unwrap();

        assert_eq!(
            candidate,
            json!({
                "function_id": "github::pr::create",
                "description": "Create a pull request?"
            })
        );
    }

    #[test]
    fn capability_rankings_merge_round_robin_before_riders() {
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let tools = vec![
            tool("alpha::one", "Handle alpha capability."),
            tool("alpha::two", "Handle alpha capability."),
            tool("beta::one", "Handle beta capability."),
        ];
        let index = Bm25Index::build(&canonical_tools(&tools));
        let queries = ["alpha capability".into(), "beta capability".into()];

        let selected = round_robin_rankings(&lexical_rankings(&index, &queries), 2);
        let namespaces: Vec<&str> = selected
            .iter()
            .filter_map(|function_id| function_namespace(function_id))
            .collect();

        assert_eq!(namespaces, ["alpha", "beta"]);
    }

    #[test]
    fn semantic_mode_selection_preserves_lexical_and_fuses_only_hybrid() {
        let lexical = vec![vec![("lex::one".into(), 2.0), ("lex::two".into(), 1.0)]];
        let semantic = vec![vec![("sem::one".into(), 0.9), ("lex::two".into(), 0.8)]];

        assert!(!needs_semantic(FunctionSearchMode::Lexical));
        assert!(needs_semantic(FunctionSearchMode::Shadow));
        assert!(needs_semantic(FunctionSearchMode::Hybrid));
        assert_eq!(
            rankings_for_mode(
                FunctionSearchMode::Lexical,
                &lexical,
                Some(semantic.clone()),
                1.0,
            ),
            lexical
        );
        assert_eq!(
            rankings_for_mode(
                FunctionSearchMode::Shadow,
                &lexical,
                Some(semantic.clone()),
                1.0,
            ),
            lexical
        );
        assert_ne!(
            rankings_for_mode(FunctionSearchMode::Hybrid, &lexical, Some(semantic), 1.0),
            lexical
        );
        assert_eq!(
            rankings_for_mode(FunctionSearchMode::Hybrid, &lexical, None, 1.0),
            lexical
        );
        assert_eq!(
            rankings_for_mode(
                FunctionSearchMode::Hybrid,
                &lexical,
                Some(vec![vec![]]),
                1.0
            ),
            vec![vec![]]
        );
        assert_eq!(
            rankings_for_mode(
                FunctionSearchMode::Shadow,
                &lexical,
                Some(vec![vec![]]),
                1.0
            ),
            lexical
        );
    }

    #[test]
    fn production_minilm_ordering_anchors_the_reranker_in_full_retrieval() {
        let lexical = vec![
            ("mail::send".into(), 10.0),
            ("web::fetch".into(), 9.0),
            ("docs::read".into(), 8.0),
        ];
        let semantic = vec![
            ("docs::read".into(), 0.92),
            ("web::fetch".into(), 0.88),
            ("calendar::list".into(), 0.81),
        ];
        let raw_reranker = vec![
            ("calendar::list".into(), 5.0),
            ("docs::read".into(), 4.0),
            ("web::fetch".into(), 3.0),
            ("mail::send".into(), 2.0),
        ];

        let retrieval = weighted_rrf(&lexical, &semantic, PRODUCTION_RETRIEVAL_WEIGHT);
        let expected = weighted_rrf(&retrieval, &raw_reranker, PRODUCTION_RERANKER_WEIGHT);

        assert_eq!(
            production_minilm_ordering(&lexical, &semantic, &raw_reranker),
            Some(expected)
        );
    }

    #[test]
    fn production_minilm_ordering_rejects_partial_duplicate_and_non_finite_output() {
        let lexical = vec![("mail::send".into(), 2.0), ("web::fetch".into(), 1.0)];
        let semantic = vec![("web::fetch".into(), 0.9), ("calendar::list".into(), 0.8)];

        assert_eq!(
            production_minilm_ordering(
                &lexical,
                &semantic,
                &[("mail::send".into(), 1.0), ("web::fetch".into(), 0.9)]
            ),
            None,
            "the reranker must cover the entire rerank head"
        );
        assert_eq!(
            production_minilm_ordering(
                &lexical,
                &semantic,
                &[
                    ("mail::send".into(), 1.0),
                    ("mail::send".into(), 0.9),
                    ("web::fetch".into(), 0.8),
                ]
            ),
            None
        );
        assert_eq!(
            production_minilm_ordering(
                &lexical,
                &semantic,
                &[
                    ("mail::send".into(), f64::NAN),
                    ("web::fetch".into(), 0.9),
                    ("calendar::list".into(), 0.8),
                ]
            ),
            None
        );
    }

    #[test]
    fn production_minilm_ordering_reranks_only_the_head_and_keeps_the_tail_in_retrieval_order() {
        let lexical: Vec<(String, f64)> = (0..PRODUCTION_RERANK_DEPTH + 10)
            .map(|index| (format!("fn::{index:03}"), 100.0 - index as f64))
            .collect();
        let semantic: Vec<(String, f64)> = Vec::new();
        let retrieval = weighted_rrf(&lexical, &semantic, PRODUCTION_RETRIEVAL_WEIGHT);
        let head: Vec<String> = production_rerank_head(&retrieval)
            .map(str::to_owned)
            .collect();
        assert_eq!(head.len(), PRODUCTION_RERANK_DEPTH);

        // Reranker inverts the head: the last head member becomes its favourite.
        let raw_reranker: Vec<(String, f64)> = head
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index as f64))
            .collect();
        let ordered = production_minilm_ordering(&lexical, &semantic, &raw_reranker)
            .expect("head-only reranker output is accepted");
        let ordered_ids: Vec<&str> = ordered.iter().map(|(id, _)| id.as_str()).collect();

        assert_eq!(ordered.len(), lexical.len(), "the tail is not dropped");
        assert_eq!(
            ordered_ids[0],
            head.last().unwrap(),
            "the reranker reorders the head"
        );
        let tail_ids: Vec<&str> = ordered_ids[PRODUCTION_RERANK_DEPTH..].to_vec();
        let expected_tail: Vec<&str> = lexical[PRODUCTION_RERANK_DEPTH..]
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(tail_ids, expected_tail, "the tail keeps retrieval order");

        // Scoring the whole union is no longer the contract: reject it.
        let full_union: Vec<(String, f64)> = lexical
            .iter()
            .map(|(id, score)| (id.clone(), *score))
            .collect();
        assert_eq!(
            production_minilm_ordering(&lexical, &semantic, &full_union),
            None
        );
    }

    #[test]
    fn production_minilm_assemble_degrades_to_fused_retrieval_not_bm25() {
        let lexical = vec![vec![
            ("mail::send".into(), 10.0),
            ("web::fetch".into(), 9.0),
            ("docs::read".into(), 8.0),
        ]];
        let semantic = vec![vec![
            ("docs::read".into(), 0.92),
            ("web::fetch".into(), 0.88),
            ("calendar::list".into(), 0.81),
        ]];
        let fused = weighted_rrf(&lexical[0], &semantic[0], PRODUCTION_RETRIEVAL_WEIGHT);
        let bm25_only = lexical.clone();

        // Reranker absent or timed out: fused order, flagged incomplete.
        let mut rankings = bm25_only.clone();
        assert!(!production_minilm_assemble(
            &mut rankings,
            &[0],
            &lexical,
            &semantic,
            None
        ));
        assert_eq!(rankings[0], fused);
        assert_ne!(rankings[0], bm25_only[0]);

        // Reranker returned garbage for the lane: same fused fallback.
        let mut rankings = bm25_only.clone();
        let garbage = vec![vec![("mail::send".into(), f64::NAN)]];
        assert!(!production_minilm_assemble(
            &mut rankings,
            &[0],
            &lexical,
            &semantic,
            Some(&garbage)
        ));
        assert_eq!(rankings[0], fused);

        // Valid reranker output: the frozen ordering, flagged complete.
        let raw_reranker = vec![vec![
            ("calendar::list".into(), 5.0),
            ("docs::read".into(), 4.0),
            ("web::fetch".into(), 3.0),
            ("mail::send".into(), 2.0),
        ]];
        let mut rankings = bm25_only;
        assert!(production_minilm_assemble(
            &mut rankings,
            &[0],
            &lexical,
            &semantic,
            Some(&raw_reranker)
        ));
        assert_eq!(
            rankings[0],
            production_minilm_ordering(&lexical[0], &semantic[0], &raw_reranker[0]).unwrap()
        );
    }

    #[test]
    fn production_admission_floor_is_inclusive_and_rejects_empty_lanes() {
        let at_floor = vec![("mail::send".to_string(), PRODUCTION_ADMISSION_COSINE)];
        let below = vec![
            ("mail::send".to_string(), PRODUCTION_ADMISSION_COSINE - 1e-9),
            ("web::fetch".to_string(), 0.1),
        ];
        let unsorted_but_strong = vec![
            ("web::fetch".to_string(), 0.1),
            ("mail::send".to_string(), 0.9),
        ];
        assert!(production_admits(&at_floor));
        assert!(!production_admits(&below));
        assert!(
            production_admits(&unsorted_but_strong),
            "uses the max, not the first"
        );
        assert!(!production_admits(&[]));
    }

    #[test]
    fn production_exact_id_bypasses_models() {
        let tools = vec![
            ToolSchema {
                name: "mail::send".into(),
                description: "Send a message".into(),
                parameters: json!({ "type": "object" }),
            },
            ToolSchema {
                name: "web::fetch".into(),
                description: "Fetch a web page".into(),
                parameters: json!({ "type": "object" }),
            },
        ];

        assert_eq!(
            exact_function_id(" mail::send ", &tools),
            Some("mail::send".into())
        );
        assert_eq!(exact_function_id("send mail", &tools), None);
    }

    #[test]
    fn production_model_failure_preserves_exact_lane_and_lexical_fallback() {
        let tools = vec![
            ToolSchema {
                name: "mail::send".into(),
                description: "Send a message".into(),
                parameters: json!({ "type": "object" }),
            },
            ToolSchema {
                name: "web::fetch".into(),
                description: "Fetch a web page".into(),
                parameters: json!({ "type": "object" }),
            },
        ];
        let queries = vec![" mail::send ".into(), "fetch a page".into()];
        let lexical = vec![
            vec![("web::fetch".into(), 2.0), ("mail::send".into(), 1.0)],
            vec![("web::fetch".into(), 2.0), ("mail::send".into(), 1.0)],
        ];

        assert_eq!(
            production_fallback_rankings(&tools, &queries, &lexical),
            vec![
                vec![("mail::send".into(), 0.0)],
                vec![("web::fetch".into(), 2.0), ("mail::send".into(), 1.0)],
            ]
        );
    }

    #[test]
    fn hybrid_test_adapter_returns_public_worker_grouped_order() {
        let tool = |name: &str| ToolSchema {
            name: name.into(),
            description: "Unrelated contract vocabulary.".into(),
            parameters: json!({ "type": "object" }),
        };
        let tools = vec![tool("alpha::one"), tool("beta::one"), tool("alpha::two")];
        let ranking = hybrid_ranking_for_test(
            &tools,
            &["zzzx qqqx".into()],
            vec![vec![
                ("alpha::one".into(), 0.9),
                ("beta::one".into(), 0.8),
                ("alpha::two".into(), 0.7),
            ]],
            0.0,
            1.0,
        );
        assert_eq!(ranking, ["alpha::one", "alpha::two", "beta::one"]);
    }

    #[tokio::test]
    async fn explicit_capabilities_drive_independent_searches() {
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let tools = vec![
            tool("git::clone", "Access and clone a git repository."),
            tool("git::status", "Show git repository status."),
            tool(
                "security::scan",
                "Analyze source code for security vulnerabilities.",
            ),
            tool(
                "progress::save",
                "Persist task progress and status updates.",
            ),
            tool("coder::edit", "Modify source files in a repository."),
            tool("github::pr::create", "Create a draft pull request."),
        ];
        let deps = search_deps(tools);
        let request: SearchFunctionsRequest = serde_json::from_value(json!({
            "capabilities": [
                "access clone git repository",
                "security vulnerability analysis source code",
                "persist task progress status updates",
                "modify files repository",
                "create draft pull request"
            ]
        }))
        .unwrap();

        let response = search_functions(&deps, request).await.unwrap();
        let ids: Vec<&str> = response
            .workers
            .iter()
            .flat_map(|worker| {
                worker
                    .functions
                    .iter()
                    .map(|function| function.function_id.as_str())
            })
            .collect();

        for expected in [
            "git::clone",
            "security::scan",
            "progress::save",
            "coder::edit",
            "github::pr::create",
        ] {
            assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
        }
    }

    #[tokio::test]
    async fn capabilities_do_not_add_unrequested_matches() {
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let deps = search_deps(vec![
            tool("browser::fetch", "Fetch a browser page."),
            tool("browser::parser", "Parse a browser DOM."),
        ]);
        let response = search_functions(
            &deps,
            SearchFunctionsRequest {
                capabilities: vec!["fetch a browser page".into()],
            },
        )
        .await
        .unwrap();
        let ids: Vec<&str> = response
            .workers
            .iter()
            .flat_map(|worker| {
                worker
                    .functions
                    .iter()
                    .map(|function| function.function_id.as_str())
            })
            .collect();

        assert_eq!(ids, ["browser::fetch"]);
    }

    #[tokio::test]
    async fn g1_fetch_capability_selects_only_browser_fetch() {
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let fetch_tier = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({
                "type": "object",
                "properties": { "main_content_only": { "type": "boolean" } }
            }),
        };
        let deps = search_deps(vec![
            tool(
                "web::fetch",
                "Fetch a URL over HTTP(S) and return the response as a structured envelope.",
            ),
            fetch_tier(
                "browser::fetch",
                "Fast HTTP fetch, TLS impersonation: get/post/put/delete, inline extraction, bulk `urls`.",
            ),
            fetch_tier(
                "browser::dynamic-fetch",
                "Playwright/Chromium fetch: JS render, waits, XHR capture, CDP; extraction + bulk.",
            ),
            fetch_tier(
                "browser::stealthy-fetch",
                "Camoufox stealth browser: solves Cloudflare, hardens WebRTC/canvas; extraction + bulk.",
            ),
            fetch_tier(
                "browser::session-fetch",
                "Fetch a URL on an open session (reuses its cookies/browser); same page/extraction output.",
            ),
            tool(
                "browser::extract",
                "Parse HTML with a selector list (css/xpath/regex, text/attr/html, all-or-first).",
            ),
            tool(
                "browser::css",
                "One CSS query over HTML; first-or-all; `attr` pulls an attribute else text.",
            ),
            tool(
                "browser::xpath",
                "One XPath query over HTML; first-or-all; `attr` pulls an attribute else text.",
            ),
            tool(
                "browser::regex",
                "Run a regex over the visible text of provided HTML; `first` returns the first match, else all.",
            ),
            tool(
                "browser::find-similar",
                "Structural auto-match: given one example element, return it plus similar elements.",
            ),
            tool(
                "browser::find",
                "Find elements by tag/attribute filters (+ optional text regex); BeautifulSoup-style.",
            ),
            tool(
                "browser::find-by-text",
                "Find elements whose visible text matches a string (exact or `partial`).",
            ),
            tool(
                "browser::find-by-regex",
                "Find elements whose visible text matches a regex pattern.",
            ),
            tool(
                "browser::describe",
                "Describe the first css/xpath match: attrs, generated selectors, class list, DOM context.",
            ),
            tool(
                "browser::to-markdown",
                "Convert HTML to compact Markdown (or text/html); optional CSS scope + main-content clean.",
            ),
            tool(
                "shell::fs::read",
                "Stream a file from a path — returns a ContentRef HANDLE (channel_id/access_key), NOT the file text.",
            ),
            tool(
                "coder::read-file",
                "Read a file window-first: probe with stat: true (size/mtime/mode plus total_lines, no content), then fetch just the lines you need with line_from/line_to (1-based, inclusive) — windows keep files larger than max_read_bytes readable window by window, with more_lines/total_lines reporting what remains.",
            ),
        ]);
        let response = search_functions(
            &deps,
            SearchFunctionsRequest {
                capabilities: vec![
                    "fetch webpage content".into(),
                    "web scraping".into(),
                    "news extraction".into(),
                    "summarize text".into(),
                ],
            },
        )
        .await
        .unwrap();
        let ids: Vec<&str> = response
            .workers
            .iter()
            .flat_map(|worker| worker.functions.iter())
            .map(|function| function.function_id.as_str())
            .collect();

        assert_eq!(ids, ["browser::fetch"]);
    }

    #[tokio::test]
    async fn all_intrinsic_capabilities_stay_empty() {
        let server = wiremock::MockServer::start().await;
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let tools = vec![
            tool(
                "browser::fetch",
                "Fetch web page content over HTTP from a URL.",
            ),
            tool(
                "browser::to-markdown",
                "Convert HTML to compact Markdown text and clean main content.",
            ),
            tool("shell::fs::read", "Read text content from a file path."),
        ];
        let config = SkillsConfig {
            registry_url: server.uri(),
            registry_search: true,
            ..SkillsConfig::default()
        };
        let deps = Deps {
            config: config.into_shared(),
            catalog: Arc::new(RwLock::new(Arc::new(tools))),
            sessions: Arc::default(),
            registry_cache: RegistryCache::new(std::time::Duration::ZERO),
            semantic: SemanticSearch::default(),
        };
        let response = search_functions(
            &deps,
            SearchFunctionsRequest {
                capabilities: vec![
                    "summarize text content".into(),
                    "summarise provided text".into(),
                    "summary of provided content".into(),
                ],
            },
        )
        .await
        .unwrap();

        assert!(response.workers.is_empty(), "{:?}", response.workers);
        assert!(
            response.installable.is_empty(),
            "{:?}",
            response.installable
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(response
            .guidance
            .contains("Always write every `capabilities` entry in English"));
    }

    #[tokio::test]
    async fn successful_guidance_excludes_intrinsic_work_from_follow_up_searches() {
        let deps = search_deps(vec![ToolSchema {
            name: "browser::fetch".into(),
            description: "Fetch a browser page.".into(),
            parameters: json!({ "type": "object" }),
        }]);
        let response = search_functions(
            &deps,
            SearchFunctionsRequest {
                capabilities: vec!["fetch a browser page".into()],
            },
        )
        .await
        .unwrap();

        assert!(
            response.guidance.contains(
                "Do not search for intrinsic reasoning, summarization, planning, or formatting"
            ),
            "{}",
            response.guidance
        );
        assert!(response
            .guidance
            .contains("Always write every `capabilities` entry in English"));
    }

    #[tokio::test]
    async fn mixed_results_route_installed_candidates_before_installable_candidates() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/w"))
            .and(query_param("search", "send an email message"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "workers": [{
                    "name": "mailer",
                    "description": "Email worker",
                    "version": "1.0.0",
                    "author": { "verified": true }
                }],
                "pagination": {}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/w/mailer"))
            .and(query_param("version", "latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "worker": {
                    "name": "mailer",
                    "functions": [{
                        "name": "mailer::send",
                        "description": "Send an email message.",
                        "request_schema": { "type": "object" }
                    }]
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/w/mailer/skills"))
            .and(query_param("version", "latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "skills": [],
                "prompts": []
            })))
            .mount(&server)
            .await;
        let config = SkillsConfig {
            registry_url: server.uri(),
            registry_search: true,
            ..SkillsConfig::default()
        };
        let deps = Deps {
            config: config.into_shared(),
            catalog: Arc::new(RwLock::new(Arc::new(vec![ToolSchema {
                name: "mail::send".into(),
                description: "Send an email message.".into(),
                parameters: json!({ "type": "object" }),
            }]))),
            sessions: Arc::default(),
            registry_cache: RegistryCache::new(std::time::Duration::ZERO),
            semantic: SemanticSearch::default(),
        };
        let response = search_functions(
            &deps,
            SearchFunctionsRequest {
                capabilities: vec!["send an email message".into()],
            },
        )
        .await
        .unwrap();

        assert_eq!(response.workers[0].functions[0].function_id, "mail::send");
        assert_eq!(response.installable[0].name, "mailer");
        let installed_route = response
            .guidance
            .find("Select from `workers` before considering `installable`")
            .expect("mixed guidance prioritizes installed candidates");
        let install_route = response
            .guidance
            .find("Only when no `workers` entry fits")
            .expect("mixed guidance gates the installation route");
        assert!(installed_route < install_route);
        assert!(response
            .guidance
            .contains("absent from both `workers` and `installable`"));
        assert!(response
            .guidance
            .contains("Do not pass an `installable` function ID to engine::functions::info"));
        let install_call = response
            .guidance
            .find("FIRST call the provided install function")
            .expect("mixed guidance installs before lookup");
        let ready_wait = response
            .guidance
            .find("wait for it to report the worker ready")
            .expect("mixed guidance waits for compose readiness");
        let search_again = response
            .guidance
            .find("then search again")
            .expect("mixed guidance searches after installation");
        let installed_info = response
            .guidance
            .rfind("one batched engine::functions::info call")
            .expect("mixed guidance fetches the installed contract last");
        assert!(install_call < ready_wait);
        assert!(ready_wait < search_again);
        assert!(search_again < installed_info);
        assert_eq!(response.installable[0].install.function, "compose::add");
        assert_eq!(
            response.installable[0].install.payload,
            json!({ "worker": "mailer" })
        );
        assert!(
            response.guidance.contains(
                "Do not search for intrinsic reasoning, summarization, planning, or formatting"
            ),
            "{}",
            response.guidance
        );
        assert!(response
            .guidance
            .contains("Always write every `capabilities` entry in English"));
    }

    #[tokio::test]
    async fn search_rejects_more_than_six_capabilities() {
        let request: SearchFunctionsRequest = serde_json::from_value(json!({
            "capabilities": ["one", "two", "three", "four", "five", "six", "seven"]
        }))
        .unwrap();

        let error = search_functions(&search_deps(Vec::new()), request)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("at most 6 capabilities"));
    }

    #[tokio::test]
    async fn search_requires_at_least_one_capability() {
        let error = search_functions(
            &search_deps(Vec::new()),
            SearchFunctionsRequest {
                capabilities: Vec::new(),
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("at least one capability"));
    }

    #[test]
    fn request_requires_capabilities_on_the_wire() {
        for payload in [json!({}), json!({ "query": "legacy search" })] {
            assert!(serde_json::from_value::<SearchFunctionsRequest>(payload).is_err());
        }
    }

    #[tokio::test]
    async fn search_rejects_blank_capabilities() {
        let request: SearchFunctionsRequest = serde_json::from_value(json!({
            "capabilities": ["inspect repository", "   "]
        }))
        .unwrap();

        let error = search_functions(&search_deps(Vec::new()), request)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("capabilities must not be blank"));
    }

    #[test]
    fn coverage_prune_keeps_full_coverage_and_high_scores_only() {
        let ranked = vec![
            ("a::leader".to_string(), 10.0, 3),
            ("a::cochecks".to_string(), 8.9, 2), // high score, low coverage → stays
            ("b::getter".to_string(), 6.0, 3),   // full coverage, mid score → stays
            ("a::family".to_string(), 5.2, 2),   // low coverage, mid score → dropped
            ("c::tail".to_string(), 2.0, 3),     // full coverage, sub-floor → dropped
        ];
        let kept = drop_low_coverage(ranked);
        let ids: Vec<&str> = kept.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a::leader", "a::cochecks", "b::getter"]);
        assert!(drop_low_coverage(Vec::new()).is_empty());
    }

    fn registry_worker(name: &str, version: &str, description: &str, verified: bool) -> Worker {
        serde_json::from_value(json!({
            "name": name,
            "version": version,
            "description": description,
            "author": { "verified": verified },
        }))
        .expect("worker fixture deserializes")
    }

    #[test]
    fn fuse_admitted_rescues_a_capability_bm25_missed() {
        let lexical = vec![Vec::new(), vec![("a::x".to_string(), 2.0)]];
        let dense = vec![
            vec![("web::fetch".to_string(), 0.41), ("b::y".to_string(), 0.10)],
            // Below the admission floor: BM25 stays.
            vec![("a::z".to_string(), 0.12)],
        ];
        let fused = fuse_admitted(lexical.clone(), Some(dense));
        let ids: Vec<&str> = fused[0].iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["web::fetch", "b::y"]);
        assert_eq!(fused[1], lexical[1]);
        assert_eq!(fuse_admitted(lexical.clone(), None), lexical);
        // Lane-count mismatch is treated as no dense lane.
        assert_eq!(
            fuse_admitted(lexical.clone(), Some(vec![Vec::new()])),
            lexical
        );
    }

    #[tokio::test]
    async fn registry_walk_follows_cursors_until_the_last_page() {
        use wiremock::matchers::{method, path, query_param, query_param_is_missing};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let page = |names: &[&str], next: Option<&str>| {
            json!({
                "workers": names
                    .iter()
                    .map(|name| json!({ "name": name, "version": "1.0.0", "description": format!("{name} worker") }))
                    .collect::<Vec<_>>(),
                "pagination": { "next_cursor": next, "has_more": next.is_some(), "page_size": 2 },
            })
        };
        Mock::given(method("GET"))
            .and(path("/w"))
            .and(query_param("cursor", "p2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(&["c"], None)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/w"))
            .and(query_param_is_missing("cursor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(&["a", "b"], Some("p2"))))
            .mount(&server)
            .await;
        let cfg = SkillsConfig {
            registry_url: server.uri(),
            ..SkillsConfig::default()
        };
        let cache = RegistryCache::new(std::time::Duration::ZERO);
        let names: Vec<String> = registry_walk(&cfg, &cache)
            .await
            .into_iter()
            .map(|worker| worker.name)
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn registry_candidates_keep_all_workers_in_order() {
        // Private registry: unverified (no verified author) workers are kept.
        let workers = vec![
            registry_worker("browser", "1.0.0", "headless browser", false),
            registry_worker("email-kit", "0.3.1", "send email", true),
            registry_worker("mailer", "2.0.0", "smtp", true),
        ];
        let candidates = registry_candidates(&workers);
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].name, "browser");
        assert_eq!(candidates[0].version, "1.0.0");
        assert_eq!(candidates[1].name, "email-kit");
        assert_eq!(candidates[2].name, "mailer");
        assert!(registry_candidates(&[]).is_empty());
    }

    #[test]
    fn registry_queries_try_each_capability_then_informative_terms() {
        assert_eq!(
            registry_queries(&["send an email message".into()]),
            ["send an email message", "send", "email", "message"]
        );
        // Stopword-only capabilities still use their full text.
        assert_eq!(registry_queries(&["the and of".into()]), ["the and of"]);
        // The cap bounds the list calls.
        assert_eq!(
            registry_queries(&["alpha beta gamma delta epsilon".into()]).len(),
            MAX_REGISTRY_QUERIES
        );
    }

    #[test]
    fn registry_queries_expand_explicit_capabilities_round_robin() {
        let capabilities = [
            "local CSV document bytes to Markdown conversion".to_string(),
            "detect and render CSV document content as Markdown".to_string(),
        ];

        assert_eq!(
            registry_queries(&capabilities),
            [
                "local CSV document bytes to Markdown conversion",
                "detect and render CSV document content as Markdown",
                "local",
                "detect",
                "csv",
                "render",
            ]
        );
    }

    #[test]
    fn registry_queries_use_each_explicit_capability() {
        let capabilities = [
            "access git repository".to_string(),
            "analyze source security".to_string(),
            "persist progress updates".to_string(),
            "modify repository files".to_string(),
            "create draft pull request".to_string(),
            "send completion notification".to_string(),
        ];

        assert_eq!(registry_queries(&capabilities), capabilities);
    }

    #[test]
    fn registry_acquisition_gives_all_six_capabilities_a_candidate() {
        let variants: Vec<Vec<RegistryCandidate>> = (1..=6)
            .map(|number| {
                vec![RegistryCandidate {
                    name: format!("worker-{number}"),
                    version: "1.0.0".into(),
                    description: String::new(),
                }]
            })
            .collect();

        let candidates = round_robin_registry_candidates(&variants, MAX_REGISTRY_CANDIDATES);

        assert_eq!(candidates.len(), 6);
        assert_eq!(candidates[5].name, "worker-6");
    }

    #[test]
    fn assemble_installable_groups_by_owner_in_rank_order() {
        let tool = |name: &str| ToolSchema {
            name: name.into(),
            description: format!("{name} description"),
            parameters: json!({ "type": "object" }),
        };
        let candidate = |name: &str| RegistryCandidate {
            name: name.into(),
            version: "1.0.0".into(),
            description: format!("{name} worker"),
        };
        let owners: HashMap<String, RegistryCandidate> = [
            ("email::send".to_string(), candidate("email")),
            ("email::read".to_string(), candidate("email")),
            ("telegram::send".to_string(), candidate("telegram-bot")),
            ("slack::send".to_string(), candidate("slack")),
        ]
        .into();
        let section = assemble_installable(
            vec![
                tool("email::send"),
                tool("telegram::send"),
                tool("email::read"),
                tool("slack::send"), // third worker: over the cap, dropped
                tool("orphan::fn"),  // no owner: skipped
            ],
            &owners,
        );
        assert_eq!(section.len(), MAX_INSTALLABLE_WORKERS);
        assert_eq!(section[0].name, "email");
        assert_eq!(section[0].functions.len(), 2);
        assert_eq!(section[1].name, "telegram-bot");
    }

    #[test]
    fn catalog_entries_skip_errors_internals_and_missing_but_keep_order() {
        let ids: Vec<String> = ["state::set", "gone::fn", "email::send", "hooks::internal"]
            .into_iter()
            .map(String::from)
            .collect();
        let mut entries = HashMap::new();
        let mut batch_pool = HashMap::new();
        pool_info_entries(
            &json!({ "functions": [
                { "function_id": "email::send", "description": "send",
                  "request_schema": { "type": "object" } },
                { "function_id": "state::set", "description": "set a value" },
                { "function_id": "gone::fn", "error": "not_found" },
                { "function_id": "hooks::internal", "description": "internal",
                  "metadata": { "internal": true } },
                { "description": "no id, skipped" },
            ]}),
            &mut batch_pool,
        );
        entries.extend(batch_pool);
        let catalog = catalog_from_entries(&ids, &entries);
        let names: Vec<&str> = catalog.iter().map(|tool| tool.name.as_str()).collect();
        // Listed order, minus the errored, internal, and id-less entries.
        assert_eq!(names, ["state::set", "email::send"]);
        assert_eq!(catalog[1].parameters, json!({ "type": "object" }));
        // A schema-less entry falls back to the open object schema.
        assert_eq!(catalog[0].parameters, json!({ "type": "object" }));
    }

    #[test]
    fn malformed_batch_responses_pool_nothing() {
        let mut pool = HashMap::new();
        pool_info_entries(&json!({ "nope": true }), &mut pool);
        pool_info_entries(&json!("not an object"), &mut pool);
        assert!(pool.is_empty());
    }

    #[test]
    fn registry_contracts_skip_internal_installed_and_excluded() {
        let info: WorkerInfoOutput = serde_json::from_value(json!({
            "worker": { "name": "email" },
            "api_reference": { "functions": [
                { "name": "email::send", "description": "send an email",
                  "request_schema": { "type": "object" }, "metadata": {} },
                { "name": "email::on-config-change", "description": "internal",
                  "metadata": { "internal": true } },
                { "name": "state::set", "description": "already installed" },
                { "name": "engine::functions::list", "description": "excluded" },
                { "name": SEARCH_FN, "description": "the search itself" },
            ]},
            "skills_tree": { "skills": [], "prompts": [] },
        }))
        .expect("info fixture deserializes");
        let installed: HashSet<&str> = ["state::set"].into();
        let contracts = registry_contracts(&info, &installed);
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].name, "email::send");
        assert_eq!(contracts[0].parameters, json!({ "type": "object" }));
    }

    #[test]
    fn registry_contract_ranking_keeps_matches_within_budget() {
        let contracts = vec![
            ToolSchema {
                name: "email::send".into(),
                description: "Send an email message to a recipient over smtp.".into(),
                parameters: json!({ "type": "object" }),
            },
            ToolSchema {
                name: "email::templates::render".into(),
                description: "Render an html template.".into(),
                parameters: json!({ "type": "object" }),
            },
        ];
        let queries = ["send an email".to_string()];
        let ranked = rank_registry_contracts(&queries, contracts.clone(), None, 6);
        assert_eq!(
            ranked.first().map(|tool| tool.name.as_str()),
            Some("email::send")
        );
        assert!(rank_registry_contracts(&queries, contracts, None, 0).is_empty());
        assert!(rank_registry_contracts(&queries, Vec::new(), None, 6).is_empty());
    }

    #[test]
    fn registry_contract_ranking_covers_explicit_capabilities() {
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let contracts = vec![
            tool("git::clone", "Access and clone a git repository."),
            tool(
                "security::scan",
                "Analyze source code for security vulnerabilities.",
            ),
            tool(
                "progress::save",
                "Persist task progress and status updates.",
            ),
            tool("coder::edit", "Modify source files in a repository."),
            tool("github::pr::create", "Create a draft pull request."),
        ];
        let capabilities = [
            "access clone git repository".to_string(),
            "security vulnerability analysis source code".to_string(),
            "persist task progress status updates".to_string(),
            "modify files repository".to_string(),
            "create draft pull request".to_string(),
        ];

        let ranked = rank_registry_contracts(&capabilities, contracts, None, 6);
        let ids: Vec<&str> = ranked.iter().map(|tool| tool.name.as_str()).collect();

        for expected in [
            "git::clone",
            "security::scan",
            "progress::save",
            "coder::edit",
            "github::pr::create",
        ] {
            assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
        }
    }

    #[test]
    fn registry_capabilities_do_not_add_unrequested_matches() {
        let tool = |name: &str, description: &str| ToolSchema {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object" }),
        };
        let ranked = rank_registry_contracts(
            &["fetch a browser page".into()],
            vec![
                tool("browser::fetch", "Fetch a browser page."),
                tool("browser::parser", "Parse a browser DOM."),
            ],
            None,
            6,
        );
        let ids: Vec<&str> = ranked.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(ids, ["browser::fetch"]);
    }

    #[test]
    fn worker_cap_drops_candidates_before_session_delivery() {
        let ranked: Vec<String> = (b'a'..=b'g')
            .map(|letter| format!("{}::run", char::from(letter)))
            .collect();
        let emitted = limit_search_workers(ranked);
        let mut registry = SessionRegistry::default();

        registry.split("session", "catalog", &emitted);
        let later = vec!["g::run".to_string(), "h::run".to_string()];
        let (new, repeated) = registry.split("session", "catalog", &later);

        assert_eq!(new, later);
        assert!(repeated.is_empty());
    }

    #[test]
    fn session_registry_splits_repeats_and_recovers_on_all_repeat() {
        let mut registry = SessionRegistry::default();
        let first = vec!["a::x".to_string(), "b::y".to_string()];
        assert_eq!(
            registry.split("s", "f1", &first),
            (first.clone(), Vec::new())
        );
        let second = vec!["a::x".to_string(), "c::z".to_string()];
        assert_eq!(
            registry.split("s", "f1", &second),
            (vec!["c::z".to_string()], vec!["a::x".to_string()])
        );
        // An all-repeat query re-sends all candidates (compaction recovery).
        let repeat = vec!["a::x".to_string()];
        assert_eq!(
            registry.split("s", "f1", &repeat),
            (repeat.clone(), Vec::new())
        );
        // A catalog fingerprint change wipes the session record.
        assert_eq!(
            registry.split("s", "f2", &first),
            (first.clone(), Vec::new())
        );
        // Other sessions are isolated.
        assert_eq!(registry.split("other", "f1", &first), (first, Vec::new()));
    }

    #[test]
    fn hint_fires_once_per_turn_with_replay_and_reanchor() {
        let mut registry = SessionRegistry::default();
        let record = |turn_id: &str, step: u64, generation: u64, expose| HintRecord {
            turn_id: turn_id.into(),
            step,
            functions_generation: generation,
            expose,
        };
        assert!(registry.hint_decision("s", record("t1", 7, 3, ExposeKind::AgentTrigger)));
        // Same-step replay re-sends.
        assert!(registry.hint_decision("s", record("t1", 7, 3, ExposeKind::AgentTrigger)));
        // A later step stays silent.
        assert!(!registry.hint_decision("s", record("t1", 8, 3, ExposeKind::AgentTrigger)));
        // Generation and exposure changes within the turn do not re-arm.
        assert!(!registry.hint_decision("s", record("t1", 9, 4, ExposeKind::AgentTrigger)));
        assert!(!registry.hint_decision("s", record("t1", 10, 4, ExposeKind::Native)));
        // The hint record survives a catalog fingerprint change (dedupe
        // state resets, hint-once does not).
        registry.split("s", "fresh-fingerprint", &["a::x".to_string()]);
        assert!(!registry.hint_decision("s", record("t1", 11, 4, ExposeKind::Native)));
        // Only a new user turn re-arms after injection.
        assert!(registry.hint_decision("s", record("t2", 12, 4, ExposeKind::Native)));
    }
}
