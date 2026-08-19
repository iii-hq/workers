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

use crate::config::{SharedConfig, SkillsConfig};
use crate::functions::registry::{
    self, RegistryCache, Worker, WorkerInfoInput, WorkerInfoOutput, WorkerListInput,
};
use crate::functions::search_index::{
    canonical_tools, compact_query, tool_fingerprint, Bm25Index, ToolSchema,
    EXCLUDED_NAMESPACE_PREFIXES, SEARCH_FN,
};
use crate::surface::search_catalog as catalog;

/// Timeout for one engine catalog call during a refresh.
const CATALOG_TIMEOUT_MS: u64 = 5_000;
/// Workers returned by one `directory::search_functions` call.
const MAX_SEARCH_WORKERS: usize = 3;
/// Contracts returned by one call — the ranked guards usually select a
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
/// Secondary clauses of a multi-intent query ranked after the whole query.
const MAX_SEARCH_CLAUSES: usize = 4;
/// Consecutive empty answers before the next would-be-empty one widens to
/// single-term matches (weak anchors beat a third empty result).
const DESPERATION_STREAK: u32 = 2;
/// Registry list queries per search: the full query, then informative
/// terms one by one — the registry's pg_trgm similarity misses long
/// natural-language queries that a single term ("email") hits. All
/// variants run concurrently, so the cap bounds registry load, not
/// latency; it must cover the informative terms of a multi-intent query
/// ("fetch a web page and send an email report" carries five).
const MAX_REGISTRY_QUERIES: usize = 6;
/// Registry candidates whose API reference is fetched (info round trips).
const MAX_REGISTRY_CANDIDATES: usize = 4;
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
}

const SESSIONS_CAP: usize = 1024;

/// Per-session memory: which contracts search already delivered (repeat
/// queries omit them), the empty-answer streak behind the desperation
/// widen, and whether the hint fired (once per session). Session identity
/// comes from caller-supplied OTel baggage or the hook payload — a cache
/// key, never a security boundary; a missing or wrong id only costs a full
/// resend or an extra hint.
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
    empty_streak: u32,
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
    /// query re-sends full contracts, which is the recovery path after
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

    /// True once the session's last DESPERATION_STREAK answers all ranked
    /// empty — the widen signal for the next would-be-empty answer.
    fn desperate(&mut self, session_id: &str, fingerprint: &str) -> bool {
        self.session_record(session_id, fingerprint).empty_streak >= DESPERATION_STREAK
    }

    /// Record whether this answer ranked empty; any non-empty answer resets
    /// the streak.
    fn note_outcome(&mut self, session_id: &str, fingerprint: &str, empty: bool) {
        let record = self.session_record(session_id, fingerprint);
        record.empty_streak = if empty { record.empty_streak + 1 } else { 0 };
    }

    /// One hint per session: `true` means send it now. A same-step replay
    /// re-sends; a functions-generation or expose change re-anchors a fresh
    /// hint; anything else is a repeat and stays silent.
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
            Some(sent) if sent.turn_id == current.turn_id && current.step <= sent.step => true,
            Some(sent)
                if sent.functions_generation != current.functions_generation
                    || sent.expose != current.expose =>
            {
                record.hint = Some(current);
                true
            }
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
            record.empty_streak = 0;
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

const SEARCH_GUIDANCE: &str = "Only the functions relevant to the query are listed. This \
reference OVERRIDES discovery for them — do not call engine::functions::list or \
engine::functions::info for these. Call them with their listed request fields, directly when \
the function is a tool in your surface, otherwise via agent_trigger with { \"function\": \
\"<function_id>\", \"payload\": { ... } }. For anything missing, call \
directory::search_functions again with a more specific query; if a listed call is rejected, \
fall back to normal discovery.";

const SEARCH_REFINE_GUIDANCE: &str = "No functions matched this query. Refine the query \
with the concrete capabilities the task needs and call directory::search_functions once more.";

const SEARCH_INSTALL_GUIDANCE: &str = "No INSTALLED function matched this query. The \
`installable` entries are registry workers (verified authors) whose functions WOULD match, \
but they are NOT installed: calling their functions now FAILS with function_not_found. To \
use one, run its `install` call exactly as given (worker::add), poll worker::status with \
the worker's name until it reports running, then call directory::search_functions again \
for the newly registered contracts. If none fit, refine the query and search once more.";

const SEARCH_INSTALL_NOTE: &str = "The `installable` entries are registry workers that are \
NOT installed: calling their functions now FAILS with function_not_found. Prefer the \
installed functions above. If the task truly needs an installable worker, FIRST run its \
`install` call exactly as given (worker::add), poll worker::status with the worker's name \
until running, then search again for its contracts — never call an installable function \
before installing.";

#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchFunctionsRequest {
    /// One natural-language query naming every capability the task needs.
    pub query: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchContract {
    pub function_id: String,
    pub description: String,
    pub request_schema: Value,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchWorker {
    pub namespace: String,
    pub functions: Vec<SearchContract>,
}

/// A registry worker that is NOT installed but carries functions matching
/// the query. `name` is the registry slug `worker::add` installs.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct InstallableWorker {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Names + descriptions only — deliberately NO request schema: an
    /// uninstalled function must not look callable, so there is nothing
    /// here for a model to pattern-match into a direct call. Contracts
    /// arrive through a fresh search after the install registers them.
    pub functions: Vec<InstallableFunction>,
    /// The exact call that installs this worker, ready to execute.
    pub install: InstallCall,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct InstallableFunction {
    pub function_id: String,
    pub description: String,
}

/// A ready-made `worker::add` invocation: `{ function, payload }` matches
/// the agent_trigger call envelope so the model can execute it verbatim.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct InstallCall {
    pub function: String,
    pub payload: Value,
}

fn install_call(worker_name: &str) -> InstallCall {
    InstallCall {
        function: "worker::add".to_string(),
        payload: json!({
            "source": { "kind": "registry", "name": worker_name },
            "wait": false,
        }),
    }
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchFunctionsResponse {
    pub guidance: String,
    pub workers: Vec<SearchWorker>,
    /// Registry workers (verified authors) with matching functions, present
    /// only when nothing installed matched. Their functions are NOT
    /// callable until the worker is installed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub installable: Vec<InstallableWorker>,
    pub latency_ms: f64,
}

/// Clause boundaries of a multi-intent discover query: list punctuation
/// always splits; the word "and" splits only when every fragment around it
/// keeps at least two informative terms, so "read and write state" stays
/// whole while "register functions, and store state" separates. Returns
/// nothing for single-clause queries (the whole-query ranking already
/// covers them) and at most MAX_SEARCH_CLAUSES clauses.
fn query_clauses(query: &str) -> Vec<String> {
    let informative = |text: &str| crate::functions::search_index::bm25_terms(text).count();
    let mut clauses: Vec<String> = Vec::new();
    for piece in query.split([',', ';']) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let fragments: Vec<&str> = piece.split(" and ").map(str::trim).collect();
        if fragments.len() > 1 && fragments.iter().all(|fragment| informative(fragment) >= 2) {
            clauses.extend(fragments.into_iter().map(str::to_string));
        } else {
            clauses.push(piece.to_string());
        }
    }
    if clauses.len() <= 1 {
        return Vec::new();
    }
    clauses.truncate(MAX_SEARCH_CLAUSES);
    clauses
}

/// Coverage-aware function pruning against the rank leader: a function
/// stays only when it scores at least SEARCH_FN_FLOOR of the leader AND
/// either matches at least as many distinct query terms as the leader or
/// scores SEARCH_FN_KEEP of it. Family members share the namespace token
/// plus one generic word (fewer matched terms, mid scores) and are dropped;
/// genuinely co-relevant functions either cover the query as fully as the
/// leader or score close to it. The consult pick is inserted afterwards and
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

/// Clause boundaries of a multi-intent discover query: list punctuation
/// always splits; the word "and" splits only when every fragment around it
/// keeps at least two informative terms, so "read and write state" stays
/// whole while "register functions, and store state" separates. Returns
/// nothing for single-clause queries (the whole-query ranking already
/// Coverage-aware function pruning against the rank leader: a function
/// stays only when it scores at least SEARCH_FN_FLOOR of the leader AND
/// either matches at least as many distinct query terms as the leader or
/// scores SEARCH_FN_KEEP of it. Family members share the namespace token
/// plus one generic word (fewer matched terms, mid scores) and are dropped;
/// genuinely co-relevant functions either cover the query as fully as the
/// leader or score close to it. The consult pick is inserted afterwards and
/// bypasses every floor by design.
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

/// The caller's harness session id from OTel baggage, when the dispatch
/// carried one. Caller-supplied and unauthenticated — a cache key for
/// The caller's harness session id from OTel baggage, when the dispatch
/// carried one. Caller-supplied and unauthenticated — a cache key for
/// resend avoidance, never a security boundary.
fn baggage_session_id() -> Option<String> {
    use opentelemetry::baggage::BaggageExt;
    let context = opentelemetry::Context::current();
    let session_id = context.baggage().get(SESSION_BAGGAGE_KEY)?.to_string();
    (!session_id.is_empty()).then_some(session_id)
}

/// Group the selected function ids by worker in first-appearance order and
/// render their un-slimmed contracts, keeping at most
/// `MAX_SEARCH_WORKERS` workers. Ids missing from the catalog are skipped;
/// render their un-slimmed contracts, keeping at most
/// `MAX_SEARCH_WORKERS` workers. Ids missing from the catalog are skipped;
/// within a worker the rank order is preserved — best match first.
fn assemble_workers(selected: &[String], tools: &[ToolSchema]) -> Vec<SearchWorker> {
    let mut workers: Vec<SearchWorker> = Vec::new();
    for function_id in selected {
        let Some(namespace) = function_namespace(function_id) else {
            continue;
        };
        let Some(tool) = tools.iter().find(|tool| &tool.name == function_id) else {
            continue;
        };
        let contract = SearchContract {
            function_id: tool.name.clone(),
            description: tool.description.clone(),
            request_schema: tool.parameters.clone(),
        };
        match workers
            .iter()
            .position(|worker| worker.namespace == namespace)
        {
            Some(index) => workers[index].functions.push(contract),
            None if workers.len() < MAX_SEARCH_WORKERS => workers.push(SearchWorker {
                namespace: namespace.to_string(),
                functions: vec![contract],
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
    workers
        .iter()
        .filter(|worker| worker.author.as_ref().is_some_and(|author| author.verified))
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
            !installed.contains(function.name.as_str())
                && function.name != SEARCH_FN
                && EXCLUDED_NAMESPACE_PREFIXES
                    .iter()
                    .all(|prefix| !function.name.starts_with(prefix))
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

/// Rank the pooled candidate contracts against the FULL query and keep the
/// survivors, bounded by `budget`. Same BM25 + coverage pruning as the
/// installed catalog, and one shared ranking across every candidate — so a
/// contract that merely grazes the query ("send a message") cannot outrank
/// the real match ("send an email") by hiding in its own worker's pool.
fn rank_registry_contracts(
    query: &str,
    contracts: Vec<ToolSchema>,
    budget: usize,
) -> Vec<ToolSchema> {
    let corpus = canonical_tools(&contracts);
    let index = Bm25Index::build(&corpus);
    let selected: Vec<String> = drop_low_coverage(index.rank_with_matches(query))
        .into_iter()
        .map(|(function_id, _)| function_id)
        .take(budget)
        .collect();
    selected
        .into_iter()
        .filter_map(|id| contracts.iter().find(|tool| tool.name == id).cloned())
        .collect()
}

/// The registry list queries one fallback issues: the full query first
/// (best server-side ranking when it hits), then informative terms one by
/// one — pg_trgm similarity misses long natural-language queries that a
/// single term lands.
fn registry_queries(query: &str) -> Vec<String> {
    let mut queries = vec![query.to_string()];
    for term in crate::functions::search_index::bm25_terms(query) {
        if queries.len() >= MAX_REGISTRY_QUERIES {
            break;
        }
        if !queries.contains(&term) {
            queries.push(term);
        }
    }
    queries
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
        let function = InstallableFunction {
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

/// The installable side of a search: ask the public registry (full query,
/// then per-term retries until anything hits), pull the top verified
/// candidates' API references, and keep the functions that rank against the
/// full query in one shared ranking. Runs on every search; candidates whose
/// name is already an installed namespace are skipped — installing them
/// adds nothing the installed results don't already cover. Every failure —
/// registry down, malformed payload, no matches — returns an empty section:
/// the search itself must never error over this.
async fn registry_installable(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    installed: &[ToolSchema],
    query: &str,
) -> Vec<InstallableWorker> {
    // All list variants concurrently: a cold registry costs one timeout,
    // not one per variant, and no variant's hits starve another's — the
    // global ranking dedupes. Variant order still decides candidate
    // priority (full query first). In-process calls: same client, cache,
    // and error hygiene as `directory::registry::workers::list`.
    let mut lists = tokio::task::JoinSet::new();
    for (priority, list_query) in registry_queries(query).into_iter().enumerate() {
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
    let mut candidates: Vec<RegistryCandidate> = Vec::new();
    let mut round = 0;
    while candidates.len() < MAX_REGISTRY_CANDIDATES {
        let mut any = false;
        for list in &variant_lists {
            let Some(candidate) = list.get(round) else {
                continue;
            };
            any = true;
            if candidates.len() >= MAX_REGISTRY_CANDIDATES {
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
    // Info round trips concurrently too; pooling stays in candidate order
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
    let ranked = rank_registry_contracts(query, pooled, MAX_INSTALLABLE_FUNCTIONS);
    assemble_installable(ranked, &owners)
}

/// Up to `cap` tools sampled breadth-first across namespaces in the corpus's
/// canonical order: every worker gets its first function before any worker
/// One-shot lexical search: rank the catalog with BM25 and return the API
/// reference of ONLY the ranked functions — never a whole worker.
pub async fn search_functions(
    deps: &Deps,
    request: SearchFunctionsRequest,
) -> Result<SearchFunctionsResponse, Error> {
    if request.query.trim().is_empty() {
        return Err(Error::Handler("query must not be empty".into()));
    }
    let started = Instant::now();
    let tools = deps.catalog.read().await.clone();
    let query = compact_query(&request.query);
    // ponytail: index rebuilt per call (~250 slim docs, sub-ms); cache by
    // tool_fingerprint if search latency ever matters.
    let corpus = canonical_tools(&tools);
    let ranked = {
        // Coverage pruning judges every function against the global
        // leader, so in a multi-intent query ("register functions …,
        // read and write state, and take a screenshot") every
        // secondary intent would drop and the model re-queries once
        // per capability (measured live). Rank the whole query first
        // — single-intent results stay byte-identical — then each
        // clause on its own leader, appending survivors the whole
        // query missed. Clause scores are not comparable across
        // rankings, so order is whole-query first, then clause order.
        let index = Bm25Index::build(&corpus);
        let whole = drop_trailing_namespaces(
            drop_low_coverage(index.rank_with_matches(&query)),
            SEARCH_RANK_FLOOR,
        );
        let clauses = query_clauses(&query);
        if clauses.is_empty() {
            whole
        } else {
            // Clause survivors first, in clause order: each intent's own
            // leader claims a worker slot before the whole-query rank —
            // which a multi-intent query pollutes with cross-clause
            // matches — appends what the clauses missed.
            // Fair split of the function cap across clauses, so an
            // early clause with a wide surviving set cannot starve the
            // later clauses out of the selection.
            let budget = (MAX_SEARCH_FUNCTIONS / clauses.len()).max(3);
            let mut merged: Vec<(String, f64)> = Vec::new();
            for clause in &clauses {
                for entry in drop_trailing_namespaces(
                    drop_low_coverage(index.rank_with_matches(clause)),
                    SEARCH_RANK_FLOOR,
                )
                .into_iter()
                .take(budget)
                {
                    if !merged.iter().any(|(id, _)| id == &entry.0) {
                        merged.push(entry);
                    }
                }
            }
            for entry in whole {
                if !merged.iter().any(|(id, _)| id == &entry.0) {
                    merged.push(entry);
                }
            }
            merged
        }
    };
    let mut selected: Vec<String> = ranked
        .iter()
        .map(|(function_id, _)| function_id.clone())
        .collect();
    selected.truncate(MAX_SEARCH_FUNCTIONS);
    let session_id = baggage_session_id();
    let fingerprint = tool_fingerprint(&tools);
    // Desperation widen: once the session's recent discover answers all
    // ranked empty, a further empty answer teaches the model nothing — it
    // measurably abandons discovery for the (usually denied) engine
    // catalog. Weak single-term anchors restore navigation; the strict
    // guards stay untouched for every session that is not starving.
    if selected.is_empty() {
        if let Some(session_id) = session_id.as_deref() {
            let starving = deps
                .sessions
                .lock()
                .expect("delivered registry")
                .desperate(session_id, &fingerprint);
            if starving {
                selected = Bm25Index::build(&corpus)
                    .rank_desperate(&query)
                    .into_iter()
                    .map(|(function_id, _, _)| function_id)
                    .take(MAX_SEARCH_FUNCTIONS)
                    .collect();
            }
        }
    }
    // Repeat queries in one session skip contracts the session already
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
    let workers = assemble_workers(&selected, &tools);
    // Installable section: every search also consults the public registry
    // for NOT-installed workers whose functions match — behind the
    // registry_search knob; every failure inside returns an empty section
    // (fail-open).
    let mut installable: Vec<InstallableWorker> = Vec::new();
    let cfg = deps.config.load_full();
    if cfg.registry_search {
        installable = registry_installable(&cfg, &deps.registry_cache, &tools, &query).await;
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
                "{guidance} Already provided earlier in this session (contracts \
unchanged — reuse the earlier reference): {}.",
                repeated.join(", ")
            );
        }
        if !installable.is_empty() {
            guidance = format!("{guidance} {SEARCH_INSTALL_NOTE}");
        }
        guidance
    };
    if let Some(session_id) = session_id.as_deref() {
        deps.sessions
            .lock()
            .expect("delivered registry")
            .note_outcome(
                session_id,
                &fingerprint,
                workers.is_empty() && installable.is_empty(),
            );
    }
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
    ids.retain(|id| {
        id != SEARCH_FN
            && EXCLUDED_NAMESPACE_PREFIXES
                .iter()
                .all(|prefix| !id.starts_with(prefix))
    });
    Ok(ids)
}

fn catalog_from_responses(
    list: &Value,
    responses: Vec<Result<Value, String>>,
) -> Result<Vec<ToolSchema>, String> {
    let ids = listed_ids(list)?;
    if ids.len() != responses.len() {
        return Err("catalog info response count was incomplete".into());
    }
    ids.into_iter()
        .zip(responses)
        .filter_map(|(expected_id, response)| {
            let response = match response {
                Ok(response) => response,
                Err(_) => return Some(Err("catalog info response failed".to_string())),
            };
            let id = match response
                .get("function_id")
                .or_else(|| response.get("id"))
                .or_else(|| response.get("name"))
                .and_then(Value::as_str)
            {
                Some(id) => id,
                None => return Some(Err("catalog info response was malformed".to_string())),
            };
            if id != expected_id {
                return Some(Err("catalog info response did not match its request".into()));
            }
            // Internal plumbing (hooks, config handlers, on-change
            // listeners) is not a capability an agent should discover —
            // exclusion is by metadata, not by namespace, so the worker's
            // own public directory::* functions stay searchable.
            if response
                .get("metadata")
                .and_then(|metadata| metadata.get("internal"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                return None;
            }
            Some(Ok(ToolSchema {
                name: expected_id,
                description: response
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                parameters: response
                    .get("parameters")
                    .or_else(|| response.get("request_format"))
                    .or_else(|| response.get("request_schema"))
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object" })),
            }))
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
    let mut responses = Vec::with_capacity(ids.len());
    for id in ids {
        responses.push(
            iii.trigger(TriggerRequest {
                function_id: "engine::functions::info".into(),
                payload: json!({ "function_id": id }),
                action: None,
                timeout_ms: Some(CATALOG_TIMEOUT_MS),
            })
            .await
            .map_err(|_| "catalog info failed".to_string()),
        );
    }
    catalog_from_responses(&list, responses)
}

/// Swap the catalog cell when the fingerprint changed; `true` = changed.
async fn activate_catalog(cell: &CatalogCell, tools: Vec<ToolSchema>) -> bool {
    let catalog_unchanged =
        tool_fingerprint(cell.read().await.as_ref()) == tool_fingerprint(&tools);
    if catalog_unchanged {
        return false;
    }
    *cell.write().await = Arc::new(tools);
    true
}

pub async fn refresh_catalog(iii: &IIIClient, cell: &CatalogCell) -> Result<bool, String> {
    let iii = iii.clone();
    let cell = cell.clone();
    match tokio::spawn(async move {
        let _reload = CATALOG_RELOAD.lock().await;
        let tools = fetch_catalog(&iii)
            .await
            .map_err(|_| "catalog_fetch_failed".to_string())?;
        Ok(activate_catalog(&cell, tools).await)
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
        if let Err(error) = iii.register_trigger(RegisterTriggerInput {
            trigger_type: trigger_type.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
        }) {
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
    iii.register_function(
        specs[2].function_id,
        RegisterFunction::new_async(move |_event: OnFunctionsChangeEvent| {
            let iii = refresh_iii.clone();
            let cell = refresh_cell.clone();
            async move {
                let changed = refresh_catalog(&iii, &cell).await.map_err(Error::Handler)?;
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
    use crate::hook::ExposeKind;

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
    fn registry_candidates_keep_verified_authors_in_order() {
        let workers = vec![
            registry_worker("unverified", "1.0.0", "d", false),
            registry_worker("email-kit", "0.3.1", "send email", true),
            registry_worker("mailer", "2.0.0", "smtp", true),
        ];
        let candidates = registry_candidates(&workers);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].name, "email-kit");
        assert_eq!(candidates[0].version, "0.3.1");
        assert_eq!(candidates[1].name, "mailer");
        assert!(registry_candidates(&[]).is_empty());
    }

    #[test]
    fn registry_queries_try_the_full_query_then_informative_terms() {
        assert_eq!(
            registry_queries("send an email message"),
            ["send an email message", "send", "email", "message"]
        );
        // Stopword-only queries fall back to just the full query.
        assert_eq!(registry_queries("the and of"), ["the and of"]);
        // The cap bounds the list calls.
        assert_eq!(
            registry_queries("alpha beta gamma delta epsilon").len(),
            MAX_REGISTRY_QUERIES
        );
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
        let ranked = rank_registry_contracts("send an email", contracts.clone(), 6);
        assert_eq!(
            ranked.first().map(|tool| tool.name.as_str()),
            Some("email::send")
        );
        assert!(rank_registry_contracts("send an email", contracts, 0).is_empty());
        assert!(rank_registry_contracts("send an email", Vec::new(), 6).is_empty());
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
        // An all-repeat query re-sends full contracts (compaction recovery).
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
    fn hint_fires_once_per_session_with_replay_and_reanchor() {
        let mut registry = SessionRegistry::default();
        let record = |step: u64, generation: u64| HintRecord {
            turn_id: "t1".into(),
            step,
            functions_generation: generation,
            expose: ExposeKind::AgentTrigger,
        };
        assert!(registry.hint_decision("s", record(7, 3)));
        // Same-step replay re-sends.
        assert!(registry.hint_decision("s", record(7, 3)));
        // A later step stays silent.
        assert!(!registry.hint_decision("s", record(8, 3)));
        // A functions-generation change re-anchors a fresh hint.
        assert!(registry.hint_decision("s", record(9, 4)));
        // The hint record survives a catalog fingerprint change (dedupe
        // state resets, hint-once does not).
        registry.split("s", "fresh-fingerprint", &["a::x".to_string()]);
        assert!(!registry.hint_decision("s", record(10, 4)));
    }
}
