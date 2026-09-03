//! Deterministic relevance tests for `reflex::discover` over a frozen
//! snapshot of the live catalog (512 functions with their real
//! descriptions and request schemas, captured 2026-08-28 via
//! `engine::functions::info`). bm25 — the shipped
//! default — is purely lexical, so no engine, no model, and no judge are
//! involved: expectations pin exact function sets and run in
//! milliseconds. If a rank change breaks one of these, either the scorer
//! regressed or the expectation needs a *reviewed* update — never loosen
//! an assertion to make a run green without reading the new result.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::config::{FunctionSearchMode, SkillsConfig};
use crate::functions::registry::RegistryCache;
use crate::functions::search::{
    hybrid_ranking_for_test, lexical_rankings, production_rerank_head, search_functions,
    search_queries, Deps, SearchFunctionsRequest, SearchFunctionsResponse,
    PRODUCTION_RETRIEVAL_WEIGHT, SEMANTIC_MINIMUM_COSINE, SEMANTIC_WEIGHT,
};
use crate::functions::search_index::{canonical_tools, tool_fingerprint, Bm25Index, ToolSchema};
use crate::functions::search_semantic::{
    weighted_rrf, SemanticSearch, MINILM_MODEL_SHA256, MINILM_REPOSITORY, MINILM_REVISION,
    MINILM_TOKENIZER_SHA256, MODEL_REVISION, MODEL_SHA256, RERANKER_MODEL_SHA256,
    RERANKER_REPOSITORY, RERANKER_REVISION, RERANKER_TOKENIZER_SHA256,
};

const CATALOG_FIXTURE: &str = include_str!("../../tests/fixtures/discover_catalog.json");
const QRELS_FIXTURE: &str = include_str!("../../tests/fixtures/search_qrels.json");

const DIRECT_PUBLIC_NAMESPACES: &[&str] = &[
    "a2ui",
    "approval",
    "canvas",
    "claude",
    "codex",
    "compose",
    "compose-ui",
    "cursor",
    "devin",
    "document",
    "editor",
    "email",
    "eval",
    "grok",
    "hermes",
    "image_resize",
    "memory",
    "opencode",
    "pdf",
    "pi",
    "publish",
    "run",
    "sandbox-code-runner",
    "tailscale",
    "vscode",
    "workflow",
    "worktree",
];
const INFRASTRUCTURE_ONLY_NAMESPACES: &[&str] = &["cron", "pubsub", "gantry", "mcp"];

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EvalFixture {
    version: u32,
    cases: Vec<EvalCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EvalCase {
    id: String,
    split: String,
    kind: String,
    capabilities: Vec<String>,
    qrels: Vec<Qrel>,
    forbidden_prefixes: Vec<String>,
    forbidden_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Qrel {
    function_id: String,
    grade: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    covers: Vec<usize>,
}

#[derive(Clone, Debug)]
struct LaneResult {
    ranking: Vec<String>,
    latency_ms: f64,
    production_minilm_complete: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct LatencySummary {
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

impl LaneResult {
    fn new<I, S>(ranking: I, latency_ms: f64) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            ranking: ranking.into_iter().map(Into::into).collect(),
            latency_ms,
            production_minilm_complete: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
struct LaneMetrics {
    mrr_at_1: f64,
    recall_at_1: f64,
    recall_at_3: f64,
    recall_at_6: f64,
    recall_at_12: f64,
    ndcg_at_6: f64,
    ndcg_at_12: f64,
    worker_recall_at_12: f64,
    multi_capability_coverage_at_12: f64,
    forbidden_contamination: usize,
    false_positive_cases: usize,
    false_positive_rate: f64,
    mean_latency_ms: f64,
    max_latency_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
struct EvaluationComparison {
    lexical: LaneMetrics,
    semantic: LaneMetrics,
}

impl EvalCase {
    fn matching(id: &str, qrels: Vec<(&str, u8, Vec<usize>)>) -> Self {
        Self::fixture_case(id, "match", 1, qrels)
    }

    fn multi(id: &str, capability_count: usize, qrels: Vec<(&str, u8, Vec<usize>)>) -> Self {
        Self::fixture_case(id, "multi", capability_count, qrels)
    }

    fn no_match(id: &str) -> Self {
        Self::fixture_case(id, "no_match", 1, vec![])
    }

    fn fixture_case(
        id: &str,
        kind: &str,
        capability_count: usize,
        qrels: Vec<(&str, u8, Vec<usize>)>,
    ) -> Self {
        Self {
            id: id.into(),
            split: "calibration".into(),
            kind: kind.into(),
            capabilities: (0..capability_count)
                .map(|index| format!("capability {index}"))
                .collect(),
            qrels: qrels
                .into_iter()
                .map(|(function_id, grade, covers)| Qrel {
                    function_id: function_id.into(),
                    grade,
                    covers,
                })
                .collect(),
            forbidden_prefixes: vec!["engine::".into()],
            forbidden_ids: vec!["directory::search_functions".into()],
        }
    }
}

fn evaluate_all(
    cases: &[EvalCase],
    lexical: Vec<LaneResult>,
    semantic: Vec<LaneResult>,
) -> EvaluationComparison {
    assert_eq!(cases.len(), lexical.len(), "one lexical result per case");
    assert_eq!(cases.len(), semantic.len(), "one semantic result per case");
    EvaluationComparison {
        lexical: evaluate_lane(cases, &lexical),
        semantic: evaluate_lane(cases, &semantic),
    }
}

fn next_f32_above(value: f64) -> f32 {
    assert!(value.is_finite(), "calibration cosine must be finite");
    assert!(
        value >= f64::from(-f32::MAX) && value < f64::from(f32::MAX),
        "calibration cosine must have a finite f32 successor"
    );
    let mut candidate = value as f32;
    while f64::from(candidate) <= value {
        candidate = if candidate == 0.0 {
            f32::from_bits(1)
        } else {
            let bits = candidate.to_bits();
            f32::from_bits(if candidate.is_sign_negative() {
                bits - 1
            } else {
                bits + 1
            })
        };
    }
    candidate
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    assert!(quantile > 0.0 && quantile <= 1.0);
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((quantile * sorted.len() as f64).ceil() as usize).max(1);
    sorted[rank - 1]
}

fn p95(values: &[f64]) -> f64 {
    percentile(values, 0.95)
}

fn latency_summary(results: &[LaneResult]) -> LatencySummary {
    if results.is_empty() {
        return LatencySummary::default();
    }
    let values: Vec<f64> = results.iter().map(|result| result.latency_ms).collect();
    LatencySummary {
        mean_ms: values.iter().sum::<f64>() / values.len() as f64,
        p50_ms: percentile(&values, 0.5),
        p95_ms: p95(&values),
        max_ms: values.iter().copied().fold(0.0, f64::max),
    }
}

fn evaluate_lane(cases: &[EvalCase], results: &[LaneResult]) -> LaneMetrics {
    let mut metrics = LaneMetrics::default();
    let mut match_cases = 0usize;
    let mut multi_cases = 0usize;
    let mut no_match_cases = 0usize;

    for (case, result) in cases.iter().zip(results) {
        let ranking = deduplicated_top_12(&result.ranking);
        metrics.max_latency_ms = metrics.max_latency_ms.max(result.latency_ms);
        metrics.mean_latency_ms += result.latency_ms;
        metrics.forbidden_contamination += ranking
            .iter()
            .filter(|id| {
                case.forbidden_ids.iter().any(|forbidden| forbidden == **id)
                    || case
                        .forbidden_prefixes
                        .iter()
                        .any(|prefix| id.starts_with(prefix))
            })
            .count();

        if case.kind == "no_match" {
            no_match_cases += 1;
            metrics.false_positive_cases += usize::from(!ranking.is_empty());
            continue;
        }

        if case.kind == "multi" {
            multi_cases += 1;
            metrics.multi_capability_coverage_at_12 +=
                multi_capability_coverage(&ranking, case) as u8 as f64;
            continue;
        }

        match_cases += 1;
        let grades: HashMap<&str, u8> = case
            .qrels
            .iter()
            .map(|qrel| (qrel.function_id.as_str(), qrel.grade))
            .collect();
        metrics.mrr_at_1 += ranking.first().is_some_and(|id| grades.contains_key(*id)) as u8 as f64;
        metrics.recall_at_1 += recall_at(&ranking, &grades, 1);
        metrics.recall_at_3 += recall_at(&ranking, &grades, 3);
        metrics.recall_at_6 += recall_at(&ranking, &grades, 6);
        metrics.recall_at_12 += recall_at(&ranking, &grades, 12);
        metrics.ndcg_at_6 += ndcg_at(&ranking, &grades, 6);
        metrics.ndcg_at_12 += ndcg_at(&ranking, &grades, 12);
        metrics.worker_recall_at_12 += worker_recall_at_12(&ranking, &case.qrels);
    }

    if match_cases > 0 {
        let divisor = match_cases as f64;
        metrics.mrr_at_1 /= divisor;
        metrics.recall_at_1 /= divisor;
        metrics.recall_at_3 /= divisor;
        metrics.recall_at_6 /= divisor;
        metrics.recall_at_12 /= divisor;
        metrics.ndcg_at_6 /= divisor;
        metrics.ndcg_at_12 /= divisor;
        metrics.worker_recall_at_12 /= divisor;
    }
    if multi_cases > 0 {
        metrics.multi_capability_coverage_at_12 /= multi_cases as f64;
    }
    if no_match_cases > 0 {
        metrics.false_positive_rate = metrics.false_positive_cases as f64 / no_match_cases as f64;
    }
    if !results.is_empty() {
        metrics.mean_latency_ms /= results.len() as f64;
    }
    metrics
}

fn deduplicated_top_12(ranking: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    ranking
        .iter()
        .map(String::as_str)
        .filter(|id| seen.insert(*id))
        .take(12)
        .collect()
}

fn recall_at(ranking: &[&str], grades: &HashMap<&str, u8>, cutoff: usize) -> f64 {
    ranking
        .iter()
        .take(cutoff)
        .filter(|id| grades.contains_key(**id))
        .count() as f64
        / grades.len() as f64
}

fn ndcg_at(ranking: &[&str], grades: &HashMap<&str, u8>, cutoff: usize) -> f64 {
    let dcg = ranking
        .iter()
        .take(cutoff)
        .enumerate()
        .map(|(index, id)| dcg_gain(*grades.get(id).unwrap_or(&0), index))
        .sum::<f64>();
    let mut ideal: Vec<u8> = grades.values().copied().collect();
    ideal.sort_unstable_by(|left, right| right.cmp(left));
    let ideal_dcg = ideal
        .into_iter()
        .take(cutoff)
        .enumerate()
        .map(|(index, grade)| dcg_gain(grade, index))
        .sum::<f64>();
    if ideal_dcg == 0.0 {
        0.0
    } else {
        dcg / ideal_dcg
    }
}

fn dcg_gain(grade: u8, zero_based_rank: usize) -> f64 {
    ((1u32 << grade) - 1) as f64 / ((zero_based_rank + 2) as f64).log2()
}

fn worker_recall_at_12(ranking: &[&str], qrels: &[Qrel]) -> f64 {
    let expected: HashSet<&str> = qrels
        .iter()
        .map(|qrel| namespace(&qrel.function_id))
        .collect();
    let found: HashSet<&str> = ranking.iter().map(|id| namespace(id)).collect();
    expected.intersection(&found).count() as f64 / expected.len() as f64
}

fn namespace(function_id: &str) -> &str {
    function_id
        .split_once("::")
        .map_or(function_id, |(head, _)| head)
}

fn multi_capability_coverage(ranking: &[&str], case: &EvalCase) -> bool {
    (0..case.capabilities.len()).all(|capability| {
        case.qrels.iter().any(|qrel| {
            qrel.covers.contains(&capability) && ranking.contains(&qrel.function_id.as_str())
        })
    })
}

fn fixture_catalog() -> Vec<ToolSchema> {
    let entries: Vec<Value> = serde_json::from_str(CATALOG_FIXTURE).expect("fixture parses");
    let catalog: Vec<ToolSchema> = entries
        .iter()
        .map(|entry| ToolSchema {
            name: entry["name"].as_str().expect("fixture name").to_string(),
            description: entry["description"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            parameters: entry["parameters"].clone(),
        })
        .collect();
    assert_eq!(catalog.len(), 512, "fixture catalog count changed");
    catalog
}

fn fixture_qrels() -> EvalFixture {
    serde_json::from_str(QRELS_FIXTURE).expect("qrels fixture parses")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn load_benchmark_catalog(path: &Path) -> Result<(Vec<ToolSchema>, String), String> {
    let errors_path = path.with_file_name("errors.json");
    let errors_bytes = std::fs::read(&errors_path).map_err(|error| {
        format!(
            "read benchmark capture errors {}: {error}",
            errors_path.display()
        )
    })?;
    let capture_errors: Vec<Value> = serde_json::from_slice(&errors_bytes).map_err(|error| {
        format!(
            "parse benchmark capture errors {}: {error}",
            errors_path.display()
        )
    })?;
    if !capture_errors.is_empty() {
        return Err(format!(
            "benchmark catalog has {} capture errors in {}",
            capture_errors.len(),
            errors_path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("read benchmark catalog {}: {error}", path.display()))?;
    let catalog: Vec<ToolSchema> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse benchmark catalog {}: {error}", path.display()))?;
    if catalog.is_empty() {
        return Err(format!("benchmark catalog {} is empty", path.display()));
    }
    let mut names = HashSet::new();
    if let Some(duplicate) = catalog
        .iter()
        .map(|tool| tool.name.as_str())
        .find(|name| name.is_empty() || !names.insert(*name))
    {
        return Err(format!(
            "benchmark catalog {} has an empty or duplicate function id {duplicate:?}",
            path.display()
        ));
    }
    Ok((catalog, sha256_hex(&bytes)))
}

fn validate_qrels(fixture: &EvalFixture, catalog: &[ToolSchema]) -> Result<(), String> {
    if fixture.version != 1 {
        return Err(format!("unsupported fixture version {}", fixture.version));
    }
    let catalog_ids: HashSet<&str> = catalog.iter().map(|tool| tool.name.as_str()).collect();
    let mut case_ids = HashSet::new();
    let mut splits: HashMap<(&str, &str), usize> = HashMap::new();
    let mut direct_namespaces = HashSet::new();
    let mut no_match_count = 0usize;
    let mut multi_count = 0usize;

    for case in &fixture.cases {
        if !case_ids.insert(case.id.as_str()) {
            return Err(format!("duplicate case id {}", case.id));
        }
        if !matches!(case.split.as_str(), "calibration" | "holdout") {
            return Err(format!("invalid split in {}", case.id));
        }
        if !matches!(case.kind.as_str(), "match" | "multi" | "no_match") {
            return Err(format!("invalid kind in {}", case.id));
        }
        if !(1..=6).contains(&case.capabilities.len())
            || case
                .capabilities
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err(format!("invalid capabilities in {}", case.id));
        }
        if !case
            .forbidden_prefixes
            .iter()
            .any(|prefix| prefix == "engine::")
            || !case
                .forbidden_ids
                .iter()
                .any(|id| id == "directory::search_functions")
        {
            return Err(format!("missing standard forbidden ids in {}", case.id));
        }

        *splits
            .entry((case.kind.as_str(), case.split.as_str()))
            .or_default() += 1;
        match case.kind.as_str() {
            "no_match" => {
                no_match_count += 1;
                if !case.qrels.is_empty() {
                    return Err(format!("no-match case {} has qrels", case.id));
                }
            }
            "multi" => {
                multi_count += 1;
                if case.qrels.is_empty() {
                    return Err(format!("multi case {} has no qrels", case.id));
                }
            }
            _ if case.qrels.is_empty() => {
                return Err(format!("match case {} has no qrels", case.id));
            }
            _ => {}
        }

        let mut qrel_ids = HashSet::new();
        for qrel in &case.qrels {
            if !qrel_ids.insert(qrel.function_id.as_str()) {
                return Err(format!(
                    "duplicate qrel {} in {}",
                    qrel.function_id, case.id
                ));
            }
            if !(1..=3).contains(&qrel.grade) {
                return Err(format!("invalid grade in {}", case.id));
            }
            if !catalog_ids.contains(qrel.function_id.as_str()) {
                return Err(format!("unknown qrel {} in {}", qrel.function_id, case.id));
            }
            if qrel.function_id.starts_with("engine::")
                || qrel.function_id == "directory::search_functions"
            {
                return Err(format!(
                    "forbidden qrel {} in {}",
                    qrel.function_id, case.id
                ));
            }
            if qrel
                .covers
                .iter()
                .any(|index| *index >= case.capabilities.len())
                || (case.kind == "multi" && qrel.covers.is_empty())
            {
                return Err(format!(
                    "invalid covers for {} in {}",
                    qrel.function_id, case.id
                ));
            }
            if case.id.starts_with("direct-") {
                direct_namespaces.insert(namespace(&qrel.function_id));
            }
        }
        if case.kind == "multi"
            && !(0..case.capabilities.len())
                .all(|index| case.qrels.iter().any(|qrel| qrel.covers.contains(&index)))
        {
            return Err(format!("uncovered capability in {}", case.id));
        }
    }

    let capability_count = fixture
        .cases
        .iter()
        .map(|case| case.capabilities.len())
        .sum::<usize>();
    if capability_count != 91 {
        return Err(format!("unexpected capability count {capability_count}"));
    }
    if no_match_count < 15 || multi_count < 10 {
        return Err("fixture needs at least 15 no-match and 10 multi cases".into());
    }
    for namespace in DIRECT_PUBLIC_NAMESPACES {
        if !direct_namespaces.contains(namespace) {
            return Err(format!("missing direct case for {namespace}"));
        }
    }
    for excluded_namespace in INFRASTRUCTURE_ONLY_NAMESPACES {
        if fixture
            .cases
            .iter()
            .flat_map(|case| &case.qrels)
            .any(|qrel| namespace(&qrel.function_id) == *excluded_namespace)
        {
            return Err(format!(
                "infrastructure-only namespace {excluded_namespace} has a qrel"
            ));
        }
    }
    for (kind, expected_calibration, expected_holdout) in
        [("match", 38, 16), ("no_match", 11, 4), ("multi", 7, 3)]
    {
        let actual = (
            splits.get(&(kind, "calibration")).copied().unwrap_or(0),
            splits.get(&(kind, "holdout")).copied().unwrap_or(0),
        );
        if actual != (expected_calibration, expected_holdout) {
            return Err(format!("unexpected {kind} split: {actual:?}"));
        }
    }
    Ok(())
}

#[test]
fn system_prompt_write_catalog_keeps_full_discovery_guidance() {
    let catalog = fixture_catalog();
    for (id, expected) in [
        (
            "directory::system-prompts::create",
            "Create a NEW system prompt at <skills_folder>/system-prompts/<name>.md from full-file markdown content (frontmatter block included; a non-empty `description` is required, and a declared frontmatter `name` must match the requested name). Rejects names that already exist anywhere in the merged system-prompt scan, or a target path that already exists on disk (even one the scanner would skip). The write is atomic and fans out directory::system-prompts::on-change with { op: \"create\" }. Use directory::system-prompts::update to edit existing system prompts.",
        ),
        (
            "directory::system-prompts::update",
            "Overwrite one EXISTING filesystem-backed system prompt with new full-file markdown content. Updating a worker-bundled prompt (`builtin: true` in the list) that has no local file yet copy-on-writes that file, which then shadows the bundled copy. The frontmatter must keep a non-empty `description` (and a valid `name` when it declares one) — the same rules the scanner enforces, so an update can never produce a file the next directory::system-prompts::list would skip. The write is atomic and fans out directory::system-prompts::on-change with { op: \"update\" }. Returns the system prompt's effective name after the write (frontmatter `name:` wins over the file stem).",
        ),
        (
            "directory::system-prompts::delete",
            "Permanently delete one EXISTING filesystem-backed system prompt by name. Resolves against the same merged scan as directory::system-prompts::list, removes only that prompt's markdown file, and fans out directory::system-prompts::on-change with { op: \"delete\" }.",
        ),
    ] {
        let description = catalog
            .iter()
            .find(|entry| entry.name == id)
            .unwrap_or_else(|| panic!("missing {id}"))
            .description
            .as_str();
        assert_eq!(description, expected, "stale discovery guidance for {id}");
    }
}

fn fixture_deps() -> Deps {
    let config = SkillsConfig {
        registry_search: false,
        ..SkillsConfig::default()
    };
    Deps {
        config: config.into_shared(),
        catalog: Arc::new(RwLock::new(Arc::new(fixture_catalog()))),
        sessions: Arc::default(),
        registry_cache: RegistryCache::new(std::time::Duration::from_millis(0)),
        semantic: super::search_semantic::SemanticSearch::default(),
    }
}

async fn ask_capabilities(deps: &Deps, capabilities: &[&str]) -> SearchFunctionsResponse {
    search_functions(
        deps,
        SearchFunctionsRequest {
            capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
        },
    )
    .await
    .expect("search succeeds")
}

async fn ask(deps: &Deps, capability: &str) -> SearchFunctionsResponse {
    ask_capabilities(deps, &[capability]).await
}

fn workers(response: &SearchFunctionsResponse) -> Vec<&str> {
    response
        .workers
        .iter()
        .map(|worker| worker.namespace.as_str())
        .collect()
}

fn function_ids(response: &SearchFunctionsResponse) -> Vec<&str> {
    response
        .workers
        .iter()
        .flat_map(|worker| worker.functions.iter().map(|f| f.function_id.as_str()))
        .collect()
}

fn owned_function_ids(response: &SearchFunctionsResponse) -> Vec<String> {
    function_ids(response)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn fixture_deps_with_mode(
    catalog: &[ToolSchema],
    mode: FunctionSearchMode,
    semantic: SemanticSearch,
) -> Deps {
    let config = SkillsConfig {
        registry_search: false,
        function_search_mode: mode,
        ..SkillsConfig::default()
    };
    Deps {
        config: config.into_shared(),
        catalog: Arc::new(RwLock::new(Arc::new(catalog.to_vec()))),
        sessions: Arc::default(),
        registry_cache: RegistryCache::new(std::time::Duration::from_millis(0)),
        semantic,
    }
}

async fn evaluate_handler_case(deps: &Deps, case: &EvalCase) -> LaneResult {
    let response = search_functions(
        deps,
        SearchFunctionsRequest {
            capabilities: case.capabilities.clone(),
        },
    )
    .await
    .expect("search succeeds");
    LaneResult {
        ranking: owned_function_ids(&response),
        latency_ms: response.latency_ms,
        production_minilm_complete: response.production_minilm_complete,
    }
}

fn comparison_for_indices(
    fixture: &EvalFixture,
    lexical: &[LaneResult],
    hybrid: &[LaneResult],
    indices: &[usize],
) -> EvaluationComparison {
    evaluate_all(
        &indices
            .iter()
            .map(|index| fixture.cases[*index].clone())
            .collect::<Vec<_>>(),
        indices
            .iter()
            .map(|index| lexical[*index].clone())
            .collect(),
        indices.iter().map(|index| hybrid[*index].clone()).collect(),
    )
}

fn benchmark_comparison(
    fixture: &EvalFixture,
    lexical: &[LaneResult],
    hybrid: &[LaneResult],
    predicate: impl Fn(&EvalCase) -> bool,
) -> EvaluationComparison {
    let indices: Vec<usize> = fixture
        .cases
        .iter()
        .enumerate()
        .filter_map(|(index, case)| predicate(case).then_some(index))
        .collect();
    comparison_for_indices(fixture, lexical, hybrid, &indices)
}

fn benchmark_comparison_value(
    fixture: &EvalFixture,
    lexical: &[LaneResult],
    hybrid: &[LaneResult],
    predicate: impl Fn(&EvalCase) -> bool,
) -> Value {
    let comparison = benchmark_comparison(fixture, lexical, hybrid, predicate);
    serde_json::json!({
        "bm25": comparison.lexical,
        "minilm": comparison.semantic,
    })
}

fn benchmark_delta_value(
    fixture: &EvalFixture,
    lexical: &[LaneResult],
    hybrid: &[LaneResult],
    predicate: impl Fn(&EvalCase) -> bool,
) -> Value {
    let comparison = benchmark_comparison(fixture, lexical, hybrid, predicate);
    let bm25 = comparison.lexical;
    let minilm = comparison.semantic;
    serde_json::json!({
        "mrr_at_1": minilm.mrr_at_1 - bm25.mrr_at_1,
        "recall_at_1": minilm.recall_at_1 - bm25.recall_at_1,
        "recall_at_3": minilm.recall_at_3 - bm25.recall_at_3,
        "recall_at_6": minilm.recall_at_6 - bm25.recall_at_6,
        "recall_at_12": minilm.recall_at_12 - bm25.recall_at_12,
        "ndcg_at_6": minilm.ndcg_at_6 - bm25.ndcg_at_6,
        "ndcg_at_12": minilm.ndcg_at_12 - bm25.ndcg_at_12,
        "worker_recall_at_12": minilm.worker_recall_at_12 - bm25.worker_recall_at_12,
        "multi_capability_coverage_at_12": minilm.multi_capability_coverage_at_12 - bm25.multi_capability_coverage_at_12,
        "forbidden_contamination": minilm.forbidden_contamination as i64 - bm25.forbidden_contamination as i64,
        "false_positive_cases": minilm.false_positive_cases as i64 - bm25.false_positive_cases as i64,
        "false_positive_rate": minilm.false_positive_rate - bm25.false_positive_rate,
    })
}

/// Paired bootstrap resamples for the MiniLM-minus-BM25 deltas.
const BOOTSTRAP_RESAMPLES: usize = 2000;
/// Fixed seed so two runs on the same inputs produce the same intervals.
const BOOTSTRAP_SEED: u64 = 0x2026_0902_5EED;
const BOOTSTRAP_METRICS: [&str; 6] = [
    "mrr_at_1",
    "recall_at_12",
    "ndcg_at_12",
    "worker_recall_at_12",
    "multi_capability_coverage_at_12",
    "false_positive_rate",
];

/// splitmix64: reproducible resamples without a crate dependency.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn lane_metric(metrics: &LaneMetrics, name: &str) -> f64 {
    match name {
        "mrr_at_1" => metrics.mrr_at_1,
        "recall_at_12" => metrics.recall_at_12,
        "ndcg_at_12" => metrics.ndcg_at_12,
        "worker_recall_at_12" => metrics.worker_recall_at_12,
        "multi_capability_coverage_at_12" => metrics.multi_capability_coverage_at_12,
        "false_positive_rate" => metrics.false_positive_rate,
        other => panic!("unknown bootstrap metric {other}"),
    }
}

/// Paired bootstrap over cases: resample case indices with replacement,
/// evaluate both lanes on the same resample, and report the 2.5th/97.5th
/// percentiles of the MiniLM-minus-BM25 delta. `significant` means the 95%
/// interval excludes zero.
fn benchmark_bootstrap_value(
    fixture: &EvalFixture,
    lexical: &[LaneResult],
    hybrid: &[LaneResult],
    predicate: impl Fn(&EvalCase) -> bool,
) -> Value {
    let indices: Vec<usize> = fixture
        .cases
        .iter()
        .enumerate()
        .filter_map(|(index, case)| predicate(case).then_some(index))
        .collect();
    if indices.is_empty() {
        return serde_json::json!({});
    }
    let point = comparison_for_indices(fixture, lexical, hybrid, &indices);
    let mut deltas: HashMap<&str, Vec<f64>> = BOOTSTRAP_METRICS
        .iter()
        .map(|name| (*name, Vec::with_capacity(BOOTSTRAP_RESAMPLES)))
        .collect();
    let mut state = BOOTSTRAP_SEED;
    let mut sample = Vec::with_capacity(indices.len());
    for _ in 0..BOOTSTRAP_RESAMPLES {
        sample.clear();
        sample.extend(
            (0..indices.len())
                .map(|_| indices[(splitmix64(&mut state) % indices.len() as u64) as usize]),
        );
        let comparison = comparison_for_indices(fixture, lexical, hybrid, &sample);
        for name in BOOTSTRAP_METRICS {
            deltas.get_mut(name).expect("metric slot").push(
                lane_metric(&comparison.semantic, name) - lane_metric(&comparison.lexical, name),
            );
        }
    }
    let mut out = serde_json::Map::new();
    for name in BOOTSTRAP_METRICS {
        let values = &deltas[name];
        let low = percentile(values, 0.025);
        let high = percentile(values, 0.975);
        out.insert(
            name.into(),
            serde_json::json!({
                "delta": lane_metric(&point.semantic, name) - lane_metric(&point.lexical, name),
                "ci95": [low, high],
                "significant": low > 0.0 || high < 0.0,
            }),
        );
    }
    Value::Object(out)
}

fn benchmark_population_value(
    fixture: &EvalFixture,
    predicate: impl Fn(&EvalCase) -> bool,
) -> Value {
    let cases: Vec<&EvalCase> = fixture
        .cases
        .iter()
        .filter(|case| predicate(case))
        .collect();
    serde_json::json!({
        "case_count": cases.len(),
        "match_case_count": cases.iter().filter(|case| case.kind == "match").count(),
        "multi_case_count": cases.iter().filter(|case| case.kind == "multi").count(),
        "no_match_case_count": cases.iter().filter(|case| case.kind == "no_match").count(),
        "capability_count": cases.iter().map(|case| case.capabilities.len()).sum::<usize>(),
    })
}

fn benchmark_case_judgment(case: &EvalCase, result: &LaneResult) -> Value {
    let ranking = deduplicated_top_12(&result.ranking);
    let relevant: HashSet<&str> = case
        .qrels
        .iter()
        .map(|qrel| qrel.function_id.as_str())
        .collect();
    let relevant_at_12: Vec<&str> = ranking
        .iter()
        .copied()
        .filter(|id| relevant.contains(id))
        .collect();
    let forbidden_contamination = ranking
        .iter()
        .filter(|id| {
            case.forbidden_ids.iter().any(|forbidden| forbidden == **id)
                || case
                    .forbidden_prefixes
                    .iter()
                    .any(|prefix| id.starts_with(prefix))
        })
        .count();
    serde_json::json!({
        "top_1_relevant": (case.kind == "match").then(|| ranking.first().is_some_and(|id| relevant.contains(id))),
        "relevant_ids_at_12": relevant_at_12,
        "recall_at_12": (case.kind == "match").then(|| relevant_at_12.len() as f64 / relevant.len() as f64),
        "complete_capability_coverage_at_12": (case.kind == "multi").then(|| multi_capability_coverage(&ranking, case)),
        "false_positive": (case.kind == "no_match").then_some(!ranking.is_empty()),
        "forbidden_contamination": forbidden_contamination,
    })
}

fn build_minilm_production_benchmark_report(
    fixture: &EvalFixture,
    catalog: &[ToolSchema],
    catalog_sha256: &str,
    qrels_sha256: &str,
    lexical: &[LaneResult],
    hybrid: &[LaneResult],
) -> Value {
    assert_eq!(fixture.cases.len(), lexical.len());
    assert_eq!(fixture.cases.len(), hybrid.len());
    let bm25_latency = latency_summary(lexical);
    let minilm_latency = latency_summary(hybrid);
    let cases: Vec<Value> = fixture
        .cases
        .iter()
        .zip(lexical)
        .zip(hybrid)
        .map(|((case, lexical), hybrid)| {
            serde_json::json!({
                "id": case.id,
                "split": case.split,
                "kind": case.kind,
                "capabilities": case.capabilities,
                "qrels": case.qrels,
                "forbidden_prefixes": case.forbidden_prefixes,
                "forbidden_ids": case.forbidden_ids,
                "bm25": {
                    "ranking": deduplicated_top_12(&lexical.ranking),
                    "latency_ms": lexical.latency_ms,
                    "judgment": benchmark_case_judgment(case, lexical),
                },
                "minilm": {
                    "ranking": deduplicated_top_12(&hybrid.ranking),
                    "latency_ms": hybrid.latency_ms,
                    "production_minilm_complete": hybrid.production_minilm_complete,
                    "judgment": benchmark_case_judgment(case, hybrid),
                },
            })
        })
        .collect();

    serde_json::json!({
        "schema_version": 1,
        "benchmark": "bm25-vs-minilm-production",
        "catalog": {
            "count": catalog.len(),
            "fingerprint": tool_fingerprint(catalog),
            "sha256": catalog_sha256,
        },
        "qrels": {
            "version": fixture.version,
            "case_count": fixture.cases.len(),
            "capability_count": fixture.cases.iter().map(|case| case.capabilities.len()).sum::<usize>(),
            "sha256": qrels_sha256,
        },
        "models": {
            "embedding": {
                "repository": MINILM_REPOSITORY,
                "revision": MINILM_REVISION,
                "model_sha256": MINILM_MODEL_SHA256,
                "tokenizer_sha256": MINILM_TOKENIZER_SHA256,
            },
            "reranker": {
                "repository": RERANKER_REPOSITORY,
                "revision": RERANKER_REVISION,
                "model_sha256": RERANKER_MODEL_SHA256,
                "tokenizer_sha256": RERANKER_TOKENIZER_SHA256,
            },
        },
        "scope": {
            "registry_search": false,
            "bm25_mode": "lexical",
            "minilm_mode": "hybrid",
        },
        "populations": {
            "overall": benchmark_population_value(fixture, |_| true),
            "calibration": benchmark_population_value(fixture, |case| case.split == "calibration"),
            "holdout": benchmark_population_value(fixture, |case| case.split == "holdout"),
            "exact": benchmark_population_value(fixture, |case| case.kind == "match" && !case.id.starts_with("paraphrase-")),
            "paraphrase": benchmark_population_value(fixture, |case| case.kind == "match" && case.id.starts_with("paraphrase-")),
            "multi": benchmark_population_value(fixture, |case| case.kind == "multi"),
            "no_match": benchmark_population_value(fixture, |case| case.kind == "no_match"),
        },
        "metrics": {
            "overall": benchmark_comparison_value(fixture, lexical, hybrid, |_| true),
            "calibration": benchmark_comparison_value(fixture, lexical, hybrid, |case| case.split == "calibration"),
            "holdout": benchmark_comparison_value(fixture, lexical, hybrid, |case| case.split == "holdout"),
            "exact": benchmark_comparison_value(fixture, lexical, hybrid, |case| case.kind == "match" && !case.id.starts_with("paraphrase-")),
            "paraphrase": benchmark_comparison_value(fixture, lexical, hybrid, |case| case.kind == "match" && case.id.starts_with("paraphrase-")),
            "multi": benchmark_comparison_value(fixture, lexical, hybrid, |case| case.kind == "multi"),
            "no_match": benchmark_comparison_value(fixture, lexical, hybrid, |case| case.kind == "no_match"),
        },
        "deltas": {
            "overall": benchmark_delta_value(fixture, lexical, hybrid, |_| true),
            "calibration": benchmark_delta_value(fixture, lexical, hybrid, |case| case.split == "calibration"),
            "holdout": benchmark_delta_value(fixture, lexical, hybrid, |case| case.split == "holdout"),
            "exact": benchmark_delta_value(fixture, lexical, hybrid, |case| case.kind == "match" && !case.id.starts_with("paraphrase-")),
            "paraphrase": benchmark_delta_value(fixture, lexical, hybrid, |case| case.kind == "match" && case.id.starts_with("paraphrase-")),
            "multi": benchmark_delta_value(fixture, lexical, hybrid, |case| case.kind == "multi"),
            "no_match": benchmark_delta_value(fixture, lexical, hybrid, |case| case.kind == "no_match"),
        },
        "bootstrap": {
            "resamples": BOOTSTRAP_RESAMPLES,
            "seed": BOOTSTRAP_SEED,
            "method": "paired percentile bootstrap over cases",
            "overall": benchmark_bootstrap_value(fixture, lexical, hybrid, |_| true),
            "calibration": benchmark_bootstrap_value(fixture, lexical, hybrid, |case| case.split == "calibration"),
            "holdout": benchmark_bootstrap_value(fixture, lexical, hybrid, |case| case.split == "holdout"),
            "exact": benchmark_bootstrap_value(fixture, lexical, hybrid, |case| case.kind == "match" && !case.id.starts_with("paraphrase-")),
            "paraphrase": benchmark_bootstrap_value(fixture, lexical, hybrid, |case| case.kind == "match" && case.id.starts_with("paraphrase-")),
            "multi": benchmark_bootstrap_value(fixture, lexical, hybrid, |case| case.kind == "multi"),
            "no_match": benchmark_bootstrap_value(fixture, lexical, hybrid, |case| case.kind == "no_match"),
        },
        "latency_ms": {
            "bm25": bm25_latency,
            "minilm": minilm_latency,
            "delta": {
                "mean_ms": minilm_latency.mean_ms - bm25_latency.mean_ms,
                "p50_ms": minilm_latency.p50_ms - bm25_latency.p50_ms,
                "p95_ms": minilm_latency.p95_ms - bm25_latency.p95_ms,
                "max_ms": minilm_latency.max_ms - bm25_latency.max_ms,
            },
        },
        "cases": cases,
    })
}

fn write_benchmark_report(path: &Path, report: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("benchmark report path {} has no parent", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create benchmark report directory {}: {error}",
            parent.display()
        )
    })?;
    let mut json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("serialize benchmark report: {error}"))?;
    json.push('\n');
    std::fs::write(path, json)
        .map_err(|error| format!("write benchmark report {}: {error}", path.display()))
}

fn write_potion_report(report: &Value) -> std::path::PathBuf {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/search-eval/potion-calibration.json");
    std::fs::create_dir_all(output.parent().expect("report parent"))
        .expect("create report directory");
    let mut json = serde_json::to_string_pretty(report).expect("serialize Potion report");
    json.push('\n');
    std::fs::write(&output, json).expect("write Potion report");
    output
}

#[test]
fn fixture_deps_disables_registry_search() {
    assert!(!fixture_deps().config.load().registry_search);
}

#[tokio::test]
async fn search_response_reports_when_production_minilm_fell_back() {
    let catalog = fixture_catalog();
    let deps = fixture_deps_with_mode(
        &catalog,
        FunctionSearchMode::Hybrid,
        SemanticSearch::production_minilm_unavailable_for_test(),
    );

    let response = ask(&deps, "fetch a web page by URL").await;

    assert_eq!(response.production_minilm_complete, Some(false));
    assert_eq!(
        ask(&fixture_deps(), "fetch a web page by URL")
            .await
            .production_minilm_complete,
        None
    );
}

#[test]
fn search_qrels_fixture_is_valid_and_stratified() {
    let fixture = fixture_qrels();
    validate_qrels(&fixture, &fixture_catalog()).expect("reviewed qrels are valid");
    assert_eq!(fixture.cases.len(), 79);
    assert_eq!(
        fixture
            .cases
            .iter()
            .map(|case| case.capabilities.len())
            .sum::<usize>(),
        91
    );
    assert_eq!(
        fixture
            .cases
            .iter()
            .map(|case| case.qrels.len())
            .sum::<usize>(),
        97
    );
    // These newly captured prefixes expose configuration callbacks,
    // registration middleware, or an HTTP bridge, not caller-facing intents.
    for excluded_namespace in INFRASTRUCTURE_ONLY_NAMESPACES {
        assert!(!fixture
            .cases
            .iter()
            .flat_map(|case| &case.qrels)
            .any(|qrel| { namespace(&qrel.function_id) == *excluded_namespace }));
    }

    let mut changed_population = fixture;
    changed_population.cases[0]
        .capabilities
        .push("an unreviewed capability".into());
    assert_eq!(
        validate_qrels(&changed_population, &fixture_catalog()),
        Err("unexpected capability count 92".into())
    );
}

#[test]
fn evaluator_compares_lanes_and_keeps_no_match_separate() {
    let cases = vec![
        EvalCase::matching("match", vec![("wanted", 3, vec![])]),
        EvalCase::multi(
            "multi",
            2,
            vec![("first", 3, vec![0]), ("second", 3, vec![1])],
        ),
        EvalCase::no_match("none"),
    ];
    let comparison = evaluate_all(
        &cases,
        vec![
            LaneResult::new(vec!["other", "engine::functions::list"], 1.0),
            LaneResult::new(vec!["first"], 2.0),
            LaneResult::new(vec!["false-positive"], 3.0),
        ],
        vec![
            LaneResult::new(vec!["wanted", "engine::functions::list"], 1.0),
            LaneResult::new(vec!["first", "second"], 2.0),
            LaneResult::new(Vec::<&str>::new(), 3.0),
        ],
    );

    assert!(comparison.semantic.mrr_at_1 > comparison.lexical.mrr_at_1);
    assert!(comparison.semantic.ndcg_at_6 > comparison.lexical.ndcg_at_6);
    assert_eq!(comparison.semantic.multi_capability_coverage_at_12, 1.0);
    assert_eq!(comparison.lexical.false_positive_cases, 1);
    assert_eq!(comparison.lexical.forbidden_contamination, 1);
    assert_eq!(comparison.semantic.forbidden_contamination, 1);
}

#[test]
fn bootstrap_intervals_are_zero_for_identical_lanes_and_exclude_zero_for_uniform_gains() {
    let cases: Vec<EvalCase> = (0..20)
        .map(|index| EvalCase::matching(&format!("m{index}"), vec![("wanted", 3, vec![])]))
        .collect();
    let fixture = EvalFixture {
        version: 1,
        cases: cases.clone(),
    };
    let miss: Vec<LaneResult> = cases
        .iter()
        .map(|_| LaneResult::new(vec!["other", "wanted"], 1.0))
        .collect();
    let hit: Vec<LaneResult> = cases
        .iter()
        .map(|_| LaneResult::new(vec!["wanted", "other"], 1.0))
        .collect();

    let same = benchmark_bootstrap_value(&fixture, &miss, &miss, |_| true);
    assert_eq!(same["mrr_at_1"]["delta"], 0.0);
    assert_eq!(same["mrr_at_1"]["ci95"], serde_json::json!([0.0, 0.0]));
    assert_eq!(same["mrr_at_1"]["significant"], false);

    let gain = benchmark_bootstrap_value(&fixture, &miss, &hit, |_| true);
    assert_eq!(gain["mrr_at_1"]["delta"], 1.0);
    assert_eq!(gain["mrr_at_1"]["ci95"], serde_json::json!([1.0, 1.0]));
    assert_eq!(gain["mrr_at_1"]["significant"], true);
    assert_eq!(
        gain["recall_at_12"]["significant"], false,
        "both lanes recall the target within 12"
    );

    // Same inputs, same seed, same intervals.
    assert_eq!(
        benchmark_bootstrap_value(&fixture, &miss, &hit, |_| true),
        gain
    );
    assert_eq!(
        benchmark_bootstrap_value(&fixture, &miss, &hit, |case| case.kind == "multi"),
        serde_json::json!({}),
        "an empty slice yields no intervals"
    );
}

#[test]
fn rollout_helpers_use_strict_f32_threshold_and_nearest_rank_p95() {
    let observed = f64::from(0.5_f32);
    let threshold = next_f32_above(observed);
    assert!(f64::from(threshold) > observed);
    assert_eq!(threshold.to_bits(), 0.5_f32.to_bits() + 1);
    let negative = next_f32_above(f64::from(-0.5_f32));
    assert!(f64::from(negative) > f64::from(-0.5_f32));
    assert_eq!(negative.to_bits(), (-0.5_f32).to_bits() - 1);
    assert_eq!(next_f32_above(0.0).to_bits(), 1);
    assert_eq!(next_f32_above(-0.0).to_bits(), 1);
    assert_eq!(p95(&[1.0, 3.0, 2.0, 100.0]), 100.0);
    assert_eq!(p95(&[]), 0.0);
}

#[test]
fn benchmark_latency_summary_reports_mean_p50_p95_and_max() {
    let results = vec![
        LaneResult::new(Vec::<String>::new(), 40.0),
        LaneResult::new(Vec::<String>::new(), 10.0),
        LaneResult::new(Vec::<String>::new(), 30.0),
        LaneResult::new(Vec::<String>::new(), 20.0),
    ];

    let summary = latency_summary(&results);

    assert_eq!(summary.mean_ms, 25.0);
    assert_eq!(summary.p50_ms, 20.0);
    assert_eq!(summary.p95_ms, 40.0);
    assert_eq!(summary.max_ms, 40.0);

    let empty = latency_summary(&[]);
    assert_eq!(empty.mean_ms, 0.0);
    assert_eq!(empty.p50_ms, 0.0);
    assert_eq!(empty.p95_ms, 0.0);
    assert_eq!(empty.max_ms, 0.0);
}

#[test]
fn minilm_benchmark_report_keeps_inputs_metrics_latency_and_case_evidence() {
    let mut exact = EvalCase::matching("exact-state-get", vec![("state::get", 3, vec![])]);
    let mut paraphrase =
        EvalCase::matching("paraphrase-state-read", vec![("state::get", 3, vec![])]);
    paraphrase.split = "holdout".into();
    let multi = EvalCase::multi(
        "multi-state",
        2,
        vec![("state::get", 3, vec![0]), ("state::set", 3, vec![1])],
    );
    let mut no_match = EvalCase::no_match("none-friendly-reply");
    no_match.split = "holdout".into();
    exact.capabilities = vec!["read state".into()];
    let fixture = EvalFixture {
        version: 1,
        cases: vec![exact, paraphrase, multi, no_match],
    };
    let catalog = vec![ToolSchema {
        name: "state::get".into(),
        description: "Read a state value".into(),
        parameters: serde_json::json!({"type": "object"}),
    }];
    let lexical = vec![
        LaneResult::new(["state::get"], 10.0),
        LaneResult::new(["other::read"], 20.0),
        LaneResult::new(["state::get"], 30.0),
        LaneResult::new(["email::send"], 40.0),
    ];
    let mut hybrid = vec![
        LaneResult::new(["state::get"], 12.0),
        LaneResult::new(["state::get"], 22.0),
        LaneResult::new(["state::get", "state::set"], 32.0),
        LaneResult::new(Vec::<String>::new(), 42.0),
    ];
    for result in &mut hybrid {
        result.production_minilm_complete = Some(true);
    }

    let report = build_minilm_production_benchmark_report(
        &fixture,
        &catalog,
        "catalog-sha",
        "qrels-sha",
        &lexical,
        &hybrid,
    );

    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["benchmark"], "bm25-vs-minilm-production");
    assert_eq!(report["catalog"]["count"], 1);
    assert_eq!(report["catalog"]["sha256"], "catalog-sha");
    assert_eq!(report["qrels"]["case_count"], 4);
    assert_eq!(report["qrels"]["capability_count"], 5);
    assert_eq!(report["qrels"]["sha256"], "qrels-sha");
    assert_eq!(
        report["models"]["embedding"]["repository"],
        "sentence-transformers/all-MiniLM-L6-v2"
    );
    assert_eq!(
        report["models"]["embedding"]["model_sha256"],
        "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452"
    );
    assert_eq!(
        report["models"]["reranker"]["repository"],
        "cross-encoder/ms-marco-MiniLM-L6-v2"
    );
    assert_eq!(report["scope"]["registry_search"], false);
    assert_eq!(report["populations"]["overall"]["case_count"], 4);
    assert_eq!(report["populations"]["overall"]["match_case_count"], 2);
    assert_eq!(report["populations"]["overall"]["multi_case_count"], 1);
    assert_eq!(report["populations"]["overall"]["no_match_case_count"], 1);
    assert_eq!(report["populations"]["overall"]["capability_count"], 5);
    assert_eq!(report["populations"]["paraphrase"]["case_count"], 1);
    assert_eq!(report["metrics"]["exact"]["bm25"]["mrr_at_1"], 1.0);
    assert_eq!(report["metrics"]["paraphrase"]["minilm"]["mrr_at_1"], 1.0);
    assert_eq!(
        report["metrics"]["multi"]["minilm"]["multi_capability_coverage_at_12"],
        1.0
    );
    assert_eq!(
        report["metrics"]["no_match"]["bm25"]["false_positive_rate"],
        1.0
    );
    assert_eq!(
        report["metrics"]["no_match"]["minilm"]["false_positive_rate"],
        0.0
    );
    assert_eq!(report["deltas"]["paraphrase"]["mrr_at_1"], 1.0);
    assert_eq!(report["deltas"]["no_match"]["false_positive_rate"], -1.0);
    assert_eq!(report["bootstrap"]["resamples"], BOOTSTRAP_RESAMPLES);
    assert_eq!(
        report["bootstrap"]["no_match"]["false_positive_rate"]["delta"],
        -1.0
    );
    assert_eq!(
        report["bootstrap"]["no_match"]["false_positive_rate"]["ci95"],
        serde_json::json!([-1.0, -1.0]),
        "a single-case slice resamples to itself"
    );
    assert_eq!(report["latency_ms"]["bm25"]["p50_ms"], 20.0);
    assert_eq!(report["latency_ms"]["minilm"]["p95_ms"], 42.0);
    assert_eq!(report["latency_ms"]["delta"]["p50_ms"], 2.0);
    assert_eq!(report["cases"][1]["id"], "paraphrase-state-read");
    assert_eq!(report["cases"][0]["qrels"][0]["function_id"], "state::get");
    assert_eq!(report["cases"][0]["forbidden_prefixes"][0], "engine::");
    assert_eq!(
        report["cases"][0]["bm25"]["judgment"]["top_1_relevant"],
        true
    );
    assert_eq!(
        report["cases"][2]["minilm"]["judgment"]["complete_capability_coverage_at_12"],
        true
    );
    assert_eq!(
        report["cases"][3]["minilm"]["judgment"]["false_positive"],
        false
    );
    assert_eq!(report["cases"][1]["bm25"]["ranking"][0], "other::read");
    assert_eq!(report["cases"][1]["minilm"]["ranking"][0], "state::get");
    assert_eq!(
        report["cases"][1]["minilm"]["production_minilm_complete"],
        true
    );
}

#[test]
fn benchmark_catalog_loader_parses_live_snapshot_and_hashes_exact_bytes() {
    let directory = tempfile::tempdir().expect("temporary catalog directory");
    let path = directory.path().join("catalog.json");
    let raw = r#"[
  {"name":"state::get","description":"Read state","parameters":{"type":"object"}},
  {"name":"state::set","description":"Write state","parameters":{"type":"object"}}
]
"#;
    std::fs::write(&path, raw).expect("write live catalog fixture");
    std::fs::write(directory.path().join("errors.json"), "[]\n")
        .expect("write clean capture manifest");

    let (catalog, sha256) = load_benchmark_catalog(&path).expect("load live catalog");

    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog[0].name, "state::get");
    assert_eq!(catalog[1].name, "state::set");
    assert_eq!(sha256, sha256_hex(raw.as_bytes()));
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn benchmark_catalog_loader_rejects_an_incomplete_capture_manifest() {
    let directory = tempfile::tempdir().expect("temporary catalog directory");
    let path = directory.path().join("catalog.json");
    std::fs::write(
        &path,
        r#"[{"name":"state::get","description":"Read state","parameters":{"type":"object"}}]"#,
    )
    .expect("write partial catalog");
    std::fs::write(
        directory.path().join("errors.json"),
        r#"[{"function_id":"state::set","error":"missing response"}]"#,
    )
    .expect("write failed capture manifest");

    let error = load_benchmark_catalog(&path).expect_err("incomplete capture must fail");

    assert!(error.contains("capture errors"), "error: {error}");
}

#[test]
fn benchmark_report_writer_creates_parent_and_valid_json_with_newline() {
    let directory = tempfile::tempdir().expect("temporary report directory");
    let path = directory.path().join("nested/benchmark.json");
    let report = serde_json::json!({"benchmark": "bm25-vs-minilm-production"});

    write_benchmark_report(&path, &report).expect("write benchmark report");

    let written = std::fs::read_to_string(&path).expect("read benchmark report");
    assert!(written.ends_with('\n'));
    assert_eq!(
        serde_json::from_str::<Value>(&written).expect("parse benchmark report"),
        report
    );
}

#[tokio::test]
#[ignore = "writes the reviewed lexical baseline under target/search-eval"]
async fn write_search_relevance_baseline() {
    use std::time::Instant;

    let fixture = fixture_qrels();
    validate_qrels(&fixture, &fixture_catalog()).expect("reviewed qrels are valid");
    let deps = fixture_deps();
    let mut measured = Vec::with_capacity(fixture.cases.len());

    for case in &fixture.cases {
        let capabilities: Vec<&str> = case.capabilities.iter().map(String::as_str).collect();
        let started = Instant::now();
        let response = ask_capabilities(&deps, &capabilities).await;
        measured.push(LaneResult::new(
            function_ids(&response),
            started.elapsed().as_secs_f64() * 1_000.0,
        ));
    }

    let empty_semantic = || {
        fixture
            .cases
            .iter()
            .map(|_| LaneResult::new(Vec::<String>::new(), 0.0))
            .collect()
    };
    let timing = evaluate_all(&fixture.cases, measured.clone(), empty_semantic());
    let deterministic: Vec<LaneResult> = measured
        .iter()
        .map(|result| LaneResult::new(result.ranking.clone(), 0.0))
        .collect();
    let comparison = evaluate_all(&fixture.cases, deterministic, empty_semantic());
    let cases: Vec<Value> = fixture
        .cases
        .iter()
        .zip(&measured)
        .map(|(case, result)| {
            serde_json::json!({
                "id": case.id,
                "kind": case.kind,
                "split": case.split,
                "ranking": deduplicated_top_12(&result.ranking),
                "latency_ms": 0.0,
            })
        })
        .collect();
    let report = serde_json::json!({
        "catalog_count": 512,
        "qrels_version": fixture.version,
        "metrics": comparison,
        "cases": cases,
    });
    let timing_cases: Vec<Value> = fixture
        .cases
        .iter()
        .zip(&measured)
        .map(|(case, result)| {
            serde_json::json!({
                "id": case.id,
                "latency_ms": result.latency_ms,
            })
        })
        .collect();
    let timing_report = serde_json::json!({
        "catalog_count": 512,
        "qrels_version": fixture.version,
        "mean_latency_ms": timing.lexical.mean_latency_ms,
        "max_latency_ms": timing.lexical.max_latency_ms,
        "cases": timing_cases,
    });
    let output_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/search-eval");
    std::fs::create_dir_all(&output_dir).expect("create baseline directory");
    for (name, value) in [
        ("lexical-baseline.json", &report),
        ("lexical-baseline-timing.json", &timing_report),
    ] {
        let output = output_dir.join(name);
        let mut json = serde_json::to_string_pretty(value).expect("serialize baseline report");
        json.push('\n');
        std::fs::write(&output, json).expect("write baseline report");
        println!("wrote {}", output.display());
    }
}

#[cfg(all(
    feature = "minilm-production",
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "gnu"
))]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live catalog snapshot and III_DIRECTORY_MINILM_MODEL_PATH"]
async fn benchmark_bm25_against_minilm_production() {
    use std::time::{Duration, Instant};

    let catalog_path = std::env::var("III_DIRECTORY_SEARCH_BENCHMARK_CATALOG_PATH")
        .expect("set III_DIRECTORY_SEARCH_BENCHMARK_CATALOG_PATH to a captured catalog.json");
    let model_path = std::env::var("III_DIRECTORY_MINILM_MODEL_PATH")
        .expect("set III_DIRECTORY_MINILM_MODEL_PATH to the pinned local model bundle");
    let output = std::env::var("III_DIRECTORY_SEARCH_BENCHMARK_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/search-eval/minilm-production-benchmark.json")
        });

    let catalog_path = std::path::PathBuf::from(catalog_path);
    let (catalog, catalog_sha256) =
        load_benchmark_catalog(&catalog_path).expect("load captured live catalog");
    let fixture = fixture_qrels();
    validate_qrels(&fixture, &catalog).expect("live catalog supports the reviewed qrels");
    let catalog_fingerprint = tool_fingerprint(&catalog);
    let semantic = SemanticSearch::new(Some(model_path.into()));
    assert!(
        semantic.is_production_minilm(),
        "model bundle does not match the pinned production MiniLM contract"
    );
    semantic.rebuild(Arc::new(catalog.clone()));

    let readiness_queries = search_queries(&fixture.cases[0].capabilities);
    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            if semantic
                .rank(&catalog_fingerprint, &readiness_queries, -1.0)
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("MiniLM catalog preparation timed out");

    let lexical_deps = fixture_deps_with_mode(
        &catalog,
        FunctionSearchMode::Lexical,
        SemanticSearch::default(),
    );
    let hybrid_deps =
        fixture_deps_with_mode(&catalog, FunctionSearchMode::Hybrid, semantic.clone());
    let _ = evaluate_handler_case(&lexical_deps, &fixture.cases[0]).await;
    let _ = evaluate_handler_case(&hybrid_deps, &fixture.cases[0]).await;

    let benchmark_started = Instant::now();
    let mut lexical = Vec::with_capacity(fixture.cases.len());
    let mut hybrid = Vec::with_capacity(fixture.cases.len());
    for (index, case) in fixture.cases.iter().enumerate() {
        lexical.push(evaluate_handler_case(&lexical_deps, case).await);
        let hybrid_result = evaluate_handler_case(&hybrid_deps, case).await;
        assert_eq!(
            hybrid_result.production_minilm_complete,
            Some(true),
            "production MiniLM fell back while benchmarking {}",
            case.id
        );
        hybrid.push(hybrid_result);
        if (index + 1) % 10 == 0 || index + 1 == fixture.cases.len() {
            println!("benchmarked {}/{} cases", index + 1, fixture.cases.len());
        }
    }

    let mut report = build_minilm_production_benchmark_report(
        &fixture,
        &catalog,
        &catalog_sha256,
        &sha256_hex(QRELS_FIXTURE.as_bytes()),
        &lexical,
        &hybrid,
    );
    report["catalog"]["path"] = serde_json::json!(catalog_path.display().to_string());
    report["qrels"]["path"] = serde_json::json!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/search_qrels.json")
        .display()
        .to_string());
    report["run"] = serde_json::json!({
        "measured_case_count": fixture.cases.len(),
        "wall_time_ms": benchmark_started.elapsed().as_secs_f64() * 1_000.0,
        "warmup_cases": 1,
    });
    write_benchmark_report(&output, &report).expect("write MiniLM production benchmark");
    println!("wrote {}", output.display());
}

/// Diagnostic, not a gate: records the strongest score each stage produced per
/// case so an admission floor can be chosen from data. Reads the same inputs
/// as the benchmark; writes one JSON array to
/// `III_DIRECTORY_SEARCH_ADMISSION_OUTPUT` (default
/// `target/search-eval/admission-scores.json`).
#[cfg(all(
    feature = "minilm-production",
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "gnu"
))]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live catalog snapshot and III_DIRECTORY_MINILM_MODEL_PATH"]
async fn record_admission_scores_per_stage() {
    use std::time::Duration;

    let catalog_path = std::path::PathBuf::from(
        std::env::var("III_DIRECTORY_SEARCH_BENCHMARK_CATALOG_PATH")
            .expect("set III_DIRECTORY_SEARCH_BENCHMARK_CATALOG_PATH to a captured catalog.json"),
    );
    let model_path = std::env::var("III_DIRECTORY_MINILM_MODEL_PATH")
        .expect("set III_DIRECTORY_MINILM_MODEL_PATH to the pinned local model bundle");
    let output = std::env::var("III_DIRECTORY_SEARCH_ADMISSION_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("target/search-eval/admission-scores.json")
        });
    let (catalog, _) = load_benchmark_catalog(&catalog_path).expect("load captured live catalog");
    let fixture = fixture_qrels();
    validate_qrels(&fixture, &catalog).expect("live catalog supports the reviewed qrels");
    let fingerprint = tool_fingerprint(&catalog);
    let semantic = SemanticSearch::new(Some(model_path.into()));
    assert!(semantic.is_production_minilm());
    semantic.rebuild(Arc::new(catalog.clone()));
    let readiness = search_queries(&fixture.cases[0].capabilities);
    tokio::time::timeout(Duration::from_secs(300), async {
        while semantic.rank(&fingerprint, &readiness, -1.0).await.is_err() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("MiniLM catalog preparation timed out");
    let index = Bm25Index::build(&canonical_tools(&catalog));

    fn top_two(lane: &[(String, f64)]) -> (f64, f64) {
        let mut scores: Vec<f64> = lane.iter().map(|(_, score)| *score).collect();
        scores.sort_by(|left, right| right.total_cmp(left));
        (
            scores.first().copied().unwrap_or(f64::NAN),
            scores.get(1).copied().unwrap_or(f64::NAN),
        )
    }
    fn max_finite(values: impl Iterator<Item = f64>) -> Option<f64> {
        values
            .filter(|value| value.is_finite())
            .fold(None, |best, value| {
                Some(best.map_or(value, |best: f64| best.max(value)))
            })
    }

    let mut rows = Vec::with_capacity(fixture.cases.len());
    for case in &fixture.cases {
        let queries = search_queries(&case.capabilities);
        let lexical = lexical_rankings(&index, &queries);
        let dense = semantic
            .rank(&fingerprint, &queries, -1.0)
            .await
            .expect("dense lane");
        let heads: Vec<Vec<String>> = lexical
            .iter()
            .zip(&dense)
            .map(|(lexical, dense)| {
                let retrieval = weighted_rrf(lexical, dense, PRODUCTION_RETRIEVAL_WEIGHT);
                production_rerank_head(&retrieval)
                    .map(str::to_owned)
                    .collect()
            })
            .collect();
        let reranked = semantic
            .rerank(&fingerprint, &queries, &heads)
            .await
            .expect("reranker lane");

        let bm25 = lexical.iter().map(|lane| top_two(lane));
        let dense_scores = dense.iter().map(|lane| top_two(lane));
        let rerank_scores = reranked.iter().map(|lane| top_two(lane));
        let rerank_mean = reranked.iter().map(|lane| {
            if lane.is_empty() {
                f64::NAN
            } else {
                lane.iter().map(|(_, score)| score).sum::<f64>() / lane.len() as f64
            }
        });
        let (bm25_top, _): (Vec<f64>, Vec<f64>) = bm25.unzip();
        let (dense_top, dense_second): (Vec<f64>, Vec<f64>) = dense_scores.unzip();
        let (rerank_top, rerank_second): (Vec<f64>, Vec<f64>) = rerank_scores.unzip();
        rows.push(serde_json::json!({
            "id": case.id,
            "kind": case.kind,
            "split": case.split,
            "capabilities": case.capabilities,
            "bm25_empty": lexical.iter().all(Vec::is_empty),
            "bm25_top": max_finite(bm25_top.into_iter()),
            "dense_top": max_finite(dense_top.iter().copied()),
            "dense_gap": max_finite(dense_top.iter().zip(&dense_second).map(|(a, b)| a - b)),
            "rerank_top": max_finite(rerank_top.iter().copied()),
            "rerank_gap": max_finite(rerank_top.iter().zip(&rerank_second).map(|(a, b)| a - b)),
            "rerank_mean": max_finite(rerank_mean),
        }));
    }
    write_benchmark_report(&output, &Value::Array(rows)).expect("write admission scores");
    println!("wrote {}", output.display());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires III_DIRECTORY_POTION_MODEL_PATH"]
async fn potion_search_rollout_gate() {
    use std::time::{Duration, Instant};

    let Ok(model_path) = std::env::var("III_DIRECTORY_POTION_MODEL_PATH") else {
        return;
    };
    let fixture = fixture_qrels();
    let catalog = fixture_catalog();
    validate_qrels(&fixture, &catalog).expect("reviewed qrels are valid");
    let catalog_fingerprint = tool_fingerprint(&catalog);
    let semantic = SemanticSearch::new(Some(model_path.into()));
    semantic.rebuild(Arc::new(catalog.clone()));

    let calibration_indices: Vec<usize> = fixture
        .cases
        .iter()
        .enumerate()
        .filter_map(|(index, case)| (case.split == "calibration").then_some(index))
        .collect();
    assert_eq!(calibration_indices.len(), 56);
    let readiness_queries = search_queries(&fixture.cases[calibration_indices[0]].capabilities);
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        match semantic
            .rank(&catalog_fingerprint, &readiness_queries, -1.0)
            .await
        {
            Ok(_) => break,
            Err(error) if Instant::now() < deadline => {
                tracing::debug!(%error, "waiting for Potion catalog index");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("Potion catalog index timed out: {error}"),
        }
    }

    // Phase A: calibration queries only. Holdout text is not normalized or encoded
    // until the discovered values match the frozen production constants.
    let lexical_deps = fixture_deps_with_mode(
        &catalog,
        FunctionSearchMode::Lexical,
        SemanticSearch::default(),
    );
    let mut calibration_cases = Vec::with_capacity(calibration_indices.len());
    let mut calibration_lexical = Vec::with_capacity(calibration_indices.len());
    let mut raw_semantic = Vec::with_capacity(calibration_indices.len());
    let mut semantic_latencies = Vec::with_capacity(calibration_indices.len());
    for index in &calibration_indices {
        let case = &fixture.cases[*index];
        calibration_cases.push(case.clone());
        calibration_lexical.push(evaluate_handler_case(&lexical_deps, case).await);
        let queries = search_queries(&case.capabilities);
        let started = Instant::now();
        let ranking = semantic
            .rank(&catalog_fingerprint, &queries, -1.0)
            .await
            .unwrap_or_else(|error| {
                panic!("Potion calibration rank failed for {}: {error}", case.id)
            });
        semantic_latencies.push(started.elapsed().as_secs_f64() * 1_000.0);
        raw_semantic.push(ranking);
    }

    let greatest_no_match_cosine = calibration_cases
        .iter()
        .zip(&raw_semantic)
        .filter(|(case, _)| case.kind == "no_match")
        .flat_map(|(_, rankings)| rankings.iter().filter_map(|ranking| ranking.first()))
        .map(|(_, score)| *score)
        .max_by(f64::total_cmp)
        .expect("calibration no-match queries have semantic candidates");
    let selected_threshold = next_f32_above(greatest_no_match_cosine);

    let mut candidate_reports = Vec::new();
    let mut selected_weight = 0.5;
    let mut selected_ndcg = f64::NEG_INFINITY;
    for weight in [0.5, 0.75, 1.0] {
        let hybrid: Vec<LaneResult> = calibration_cases
            .iter()
            .zip(&raw_semantic)
            .zip(&semantic_latencies)
            .map(|((case, semantic_ranking), latency)| {
                LaneResult::new(
                    hybrid_ranking_for_test(
                        &catalog,
                        &case.capabilities,
                        semantic_ranking.clone(),
                        selected_threshold,
                        weight,
                    ),
                    *latency,
                )
            })
            .collect();
        let comparison = evaluate_all(&calibration_cases, calibration_lexical.clone(), hybrid);
        let ndcg = comparison.semantic.ndcg_at_12;
        candidate_reports.push(serde_json::json!({
            "weight": weight,
            "metrics": comparison.semantic,
        }));
        if ndcg > selected_ndcg {
            selected_ndcg = ndcg;
            selected_weight = weight;
        }
    }

    let mut report = serde_json::json!({
        "phase": "calibration",
        "catalog": {
            "count": catalog.len(),
            "fingerprint": catalog_fingerprint,
        },
        "model": {
            "revision": MODEL_REVISION,
            "sha256": MODEL_SHA256,
            "fingerprint": format!("{MODEL_REVISION}:{MODEL_SHA256}"),
        },
        "threshold": {
            "greatest_calibration_no_match_cosine": greatest_no_match_cosine,
            "selected_decimal": selected_threshold,
            "selected_f32_bits": selected_threshold.to_bits(),
        },
        "candidates": candidate_reports,
        "selected_weight": selected_weight,
    });
    let report_path = write_potion_report(&report);
    println!(
        "Potion calibration selected threshold={selected_threshold:.9} bits={} weight={selected_weight}",
        selected_threshold.to_bits()
    );
    assert_eq!(
        selected_threshold.to_bits(),
        SEMANTIC_MINIMUM_COSINE.to_bits(),
        "freeze the reported threshold in search.rs; report: {}",
        report_path.display()
    );
    assert_eq!(
        selected_weight,
        SEMANTIC_WEIGHT,
        "freeze the reported weight in search.rs; report: {}",
        report_path.display()
    );

    // Phase B: constants are frozen. Only now may holdout queries be encoded.
    let lexical_deps =
        fixture_deps_with_mode(&catalog, FunctionSearchMode::Lexical, semantic.clone());
    let hybrid_deps =
        fixture_deps_with_mode(&catalog, FunctionSearchMode::Hybrid, semantic.clone());
    for index in &calibration_indices {
        let case = &fixture.cases[*index];
        let _ = evaluate_handler_case(&lexical_deps, case).await;
        let _ = evaluate_handler_case(&hybrid_deps, case).await;
    }

    let mut lexical = Vec::with_capacity(fixture.cases.len());
    let mut hybrid = Vec::with_capacity(fixture.cases.len());
    for case in &fixture.cases {
        lexical.push(evaluate_handler_case(&lexical_deps, case).await);
        hybrid.push(evaluate_handler_case(&hybrid_deps, case).await);
    }

    let indices = |split: &str, predicate: fn(&EvalCase) -> bool| {
        fixture
            .cases
            .iter()
            .enumerate()
            .filter_map(|(index, case)| (case.split == split && predicate(case)).then_some(index))
            .collect::<Vec<_>>()
    };
    let every = |_case: &EvalCase| true;
    let exact = |case: &EvalCase| case.kind == "match" && !case.id.starts_with("paraphrase-");
    let multi = |case: &EvalCase| case.kind == "multi";
    let paraphrase = |case: &EvalCase| case.kind == "match" && case.id.starts_with("paraphrase-");
    let calibration =
        comparison_for_indices(&fixture, &lexical, &hybrid, &indices("calibration", every));
    let holdout = comparison_for_indices(&fixture, &lexical, &hybrid, &indices("holdout", every));
    let calibration_exact =
        comparison_for_indices(&fixture, &lexical, &hybrid, &indices("calibration", exact));
    let holdout_exact =
        comparison_for_indices(&fixture, &lexical, &hybrid, &indices("holdout", exact));
    let calibration_multi =
        comparison_for_indices(&fixture, &lexical, &hybrid, &indices("calibration", multi));
    let holdout_multi =
        comparison_for_indices(&fixture, &lexical, &hybrid, &indices("holdout", multi));
    let holdout_paraphrase =
        comparison_for_indices(&fixture, &lexical, &hybrid, &indices("holdout", paraphrase));
    let lexical_p95 = p95(&lexical
        .iter()
        .map(|result| result.latency_ms)
        .collect::<Vec<_>>());
    let hybrid_p95 = p95(&hybrid
        .iter()
        .map(|result| result.latency_ms)
        .collect::<Vec<_>>());

    let gates = serde_json::json!({
        "calibration_forbidden_zero": calibration.semantic.forbidden_contamination == 0,
        "holdout_forbidden_zero": holdout.semantic.forbidden_contamination == 0,
        "calibration_no_match_false_positives_zero": calibration.semantic.false_positive_cases == 0,
        "holdout_no_match_false_positives_zero": holdout.semantic.false_positive_cases == 0,
        "calibration_exact_mrr_at_1_no_regression": calibration_exact.semantic.mrr_at_1 >= calibration_exact.lexical.mrr_at_1,
        "holdout_exact_mrr_at_1_no_regression": holdout_exact.semantic.mrr_at_1 >= holdout_exact.lexical.mrr_at_1,
        "calibration_exact_recall_at_12_no_regression": calibration_exact.semantic.recall_at_12 >= calibration_exact.lexical.recall_at_12,
        "holdout_exact_recall_at_12_no_regression": holdout_exact.semantic.recall_at_12 >= holdout_exact.lexical.recall_at_12,
        "calibration_multi_coverage_no_regression": calibration_multi.semantic.multi_capability_coverage_at_12 >= calibration_multi.lexical.multi_capability_coverage_at_12,
        "holdout_multi_coverage_no_regression": holdout_multi.semantic.multi_capability_coverage_at_12 >= holdout_multi.lexical.multi_capability_coverage_at_12,
        "holdout_paraphrase_ndcg_at_12_gain_0_05": holdout_paraphrase.semantic.ndcg_at_12 >= holdout_paraphrase.lexical.ndcg_at_12 + 0.05,
        "warm_hybrid_p95_within_25_ms": hybrid_p95 <= lexical_p95 + 25.0,
    });
    let rollout_ready = gates
        .as_object()
        .expect("gate object")
        .values()
        .all(|value| value.as_bool() == Some(true));
    let case_reports: Vec<Value> = fixture
        .cases
        .iter()
        .zip(&lexical)
        .zip(&hybrid)
        .map(|((case, lexical), hybrid)| {
            serde_json::json!({
                "id": case.id,
                "kind": case.kind,
                "split": case.split,
                "lexical": { "ranking": lexical.ranking, "latency_ms": lexical.latency_ms },
                "hybrid": { "ranking": hybrid.ranking, "latency_ms": hybrid.latency_ms },
            })
        })
        .collect();
    report["phase"] = serde_json::json!("rollout_gate");
    report["metrics"] = serde_json::json!({
        "calibration": calibration,
        "holdout": holdout,
        "calibration_exact": calibration_exact,
        "holdout_exact": holdout_exact,
        "calibration_multi": calibration_multi,
        "holdout_multi": holdout_multi,
        "holdout_paraphrase": holdout_paraphrase,
    });
    report["latency_p95_ms"] = serde_json::json!({
        "lexical": lexical_p95,
        "hybrid": hybrid_p95,
        "delta": hybrid_p95 - lexical_p95,
    });
    report["gates"] = gates;
    report["rollout_ready"] = serde_json::json!(rollout_ready);
    report["cases"] = Value::Array(case_reports);
    write_potion_report(&report);
    assert!(
        rollout_ready,
        "Potion rollout gates failed; report: {}",
        report_path.display()
    );
}

#[tokio::test]
async fn state_persistence_query_returns_set_and_get() {
    let deps = fixture_deps();
    let response = ask(
        &deps,
        "store a value under a key in the state scope and read it back",
    )
    .await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "state", "ids: {ids:?}");
    assert!(ids.contains(&"state::set"), "ids: {ids:?}");
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
}

#[tokio::test]
async fn shell_command_query_returns_exec() {
    let deps = fixture_deps();
    let response = ask(&deps, "run a shell command on this machine").await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "shell", "ids: {ids:?}");
    assert!(ids.contains(&"shell::exec"), "ids: {ids:?}");
}

#[tokio::test]
async fn github_repository_query_returns_repo_view() {
    let deps = fixture_deps();
    let response = ask(&deps, "check the stargazers count of a github repository").await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "github", "ids: {ids:?}");
    // "stargazers" appears in no request contract (it is response-shape
    // vocabulary), so finding the repo via search or viewing it are both
    // correct lexical resolutions.
    assert!(
        ids.contains(&"github::repo::view") || ids.contains(&"github::search::repos"),
        "ids: {ids:?}"
    );
}

#[tokio::test]
async fn issue_comment_query_returns_issue_comment() {
    let deps = fixture_deps();
    let response = ask(&deps, "comment on an open github issue").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"github::issue::comment"), "ids: {ids:?}");
}

#[tokio::test]
async fn web_page_query_returns_web_fetch() {
    let deps = fixture_deps();
    let response = ask(&deps, "fetch the content of a web page by url").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"web::fetch"), "ids: {ids:?}");
}

#[tokio::test]
async fn python_code_query_returns_code_runner_run() {
    let deps = fixture_deps();
    let response = ask(&deps, "execute a snippet of python code").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"code-runner::run"), "ids: {ids:?}");
}

#[tokio::test]
async fn storage_upload_query_returns_put_object() {
    let deps = fixture_deps();
    let response = ask(&deps, "upload an object into the storage bucket").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"storage::putObject"), "ids: {ids:?}");
}

#[tokio::test]
async fn database_schema_query_returns_describe_functions() {
    let deps = fixture_deps();
    let response = ask(&deps, "describe the database schema and its tables").await;
    let ids = function_ids(&response);
    assert_eq!(workers(&response)[0], "database", "ids: {ids:?}");
    assert!(
        ids.contains(&"database::describeSchema") || ids.contains(&"database::describeTable"),
        "ids: {ids:?}"
    );
}

#[tokio::test]
async fn screenshot_query_returns_browser_screenshot() {
    let deps = fixture_deps();
    let response = ask(&deps, "take a screenshot of the current page").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"browser::screenshot"), "ids: {ids:?}");
}

#[tokio::test]
async fn gibberish_query_returns_refine_guidance() {
    let deps = fixture_deps();
    let response = ask(&deps, "zzzz qqqq wwww").await;
    assert!(response.workers.is_empty());
    assert!(
        response.guidance.contains("No functions matched"),
        "guidance: {}",
        response.guidance
    );
    assert!(response.guidance.contains("next decision point"));
    assert!(response
        .guidance
        .contains("all unmet external capabilities"));
    assert!(response
        .guidance
        .contains("Do not search for intrinsic reasoning, summarization, planning, or formatting"));
    assert!(response
        .guidance
        .contains("do not repeat needs already represented"));
}

#[tokio::test]
async fn engine_and_the_search_itself_never_appear_in_results() {
    let deps = fixture_deps();
    for query in [
        "list every available function on the engine",
        "enqueue a message onto a queue topic",
        "route my objective to the right worker",
    ] {
        let response = ask(&deps, query).await;
        for id in function_ids(&response) {
            assert!(
                !id.starts_with("engine::") && id != crate::functions::search_index::SEARCH_FN,
                "query {query:?} leaked {id}"
            );
        }
    }
}

#[tokio::test]
async fn results_respect_worker_and_function_caps() {
    let deps = fixture_deps();
    let response = ask(
        &deps,
        "read write delete list get set update files values objects",
    )
    .await;
    assert!(
        response.workers.len() <= 6,
        "workers: {:?}",
        workers(&response)
    );
    assert!(function_ids(&response).len() <= 12);
}

#[tokio::test]
async fn clear_leader_queries_do_not_drag_config_handlers() {
    let deps = fixture_deps();
    let response = ask(&deps, "check the stargazers count of a github repository").await;
    for id in function_ids(&response) {
        assert!(
            !id.ends_with("on-config-change"),
            "config handler rode along: {id}"
        );
    }
}

#[tokio::test]
async fn same_query_is_deterministic_across_fresh_deps() {
    let first = ask(
        &fixture_deps(),
        "persist a value under a key and read it back later",
    )
    .await;
    let second = ask(
        &fixture_deps(),
        "persist a value under a key and read it back later",
    )
    .await;
    assert_eq!(
        serde_json::to_value(&first.workers).unwrap(),
        serde_json::to_value(&second.workers).unwrap()
    );
}

#[tokio::test]
async fn repeat_query_in_one_session_omits_delivered_contracts() {
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::{Context, KeyValue};
    let deps = fixture_deps();
    let context =
        Context::current_with_baggage(vec![KeyValue::new("iii.session.id", "relevance-session")]);
    let _guard = context.attach();
    let first = ask(&deps, "persist a value under a key and read it back later").await;
    let first_ids: Vec<&str> = function_ids(&first);
    assert!(first_ids.contains(&"state::set"));
    // A different request overlapping the first: overlapping contracts are
    // omitted and named in the guidance instead.
    let second = ask_capabilities(&deps, &["update persisted value", "list state keys"]).await;
    for id in function_ids(&second) {
        assert!(
            !first_ids.contains(&id),
            "repeat query re-sent an already delivered contract: {id}"
        );
    }
    assert!(
        second
            .guidance
            .contains("Already provided earlier in this session"),
        "guidance: {}",
        second.guidance
    );
    // An IDENTICAL query selects only already-delivered ids: the all-repeat
    // path re-sends the full contracts (compaction recovery), no omission.
    let third = ask(&deps, "persist a value under a key and read it back later").await;
    assert!(
        function_ids(&third).contains(&"state::set"),
        "all-repeat query must re-send full contracts: {:?}",
        function_ids(&third)
    );
    assert!(!third
        .guidance
        .contains("Already provided earlier in this session"));
}

#[tokio::test]
async fn guidance_hands_selected_candidates_to_batched_function_info() {
    let deps = fixture_deps();
    let response = ask(&deps, "run a shell command on this machine").await;
    assert!(response.guidance.contains("candidates"));
    assert!(response.guidance.contains("engine::functions::info"));
    assert!(response.guidance.contains("function_ids"));
    assert!(response.guidance.contains("smallest candidate set"));
    assert!(response
        .guidance
        .contains("all unmet external capabilities"));
    assert!(!response.guidance.contains("listed request fields"));
    assert!(response
        .guidance
        .contains("directory::search_functions again"));
}

#[tokio::test]
#[ignore = "exploratory dump for precision tuning"]
async fn dump_probe_queries() {
    use crate::functions::search_index::{canonical_tools, Bm25Index};
    let corpus = canonical_tools(&fixture_catalog());
    let index = Bm25Index::build(&corpus);
    for query in [
        "persist a value under a key and read it back later",
        "check the stargazers count of a github repository",
        "close a github issue",
        "kill a running process by pid",
        "list the files in a folder on the filesystem",
        "get the value",
        "send a message to a stream group",
        "read the browser console logs",
        "merge a pull request after checks pass",
        "create a new pull request",
    ] {
        let ranked = index.rank_with_matches(query);
        let leader = ranked.first().map(|(_, s, _)| *s).unwrap_or(0.0);
        let rows: Vec<String> = ranked
            .iter()
            .take(10)
            .map(|(id, s, m)| format!("{id}={:.0}%/m{m}", s / leader * 100.0))
            .collect();
        println!("Q: {query}\n   {rows:?}\n");
    }
    let deps = fixture_deps();
    for query in [
        "kill a running process by pid",
        "list the files in a directory",
        "close a github issue",
        "create a new pull request",
        "delete a stored value",
        "send a message to a stream group",
        "schedule a recurring job",
        "search for a text pattern in files",
        "start and stop a managed worker",
        "presign a temporary download url",
        "count the tokens in the context window",
        "get the value",
        "read the browser console logs",
        "merge a pull request after checks pass",
        "compare and set a state key atomically",
    ] {
        let response = ask(&deps, query).await;
        let ids = function_ids(&response);
        println!("Q: {query}\n   -> {} fns: {ids:?}\n", ids.len());
    }
}

// ---- precision battery: sharp queries must not drag whole families or
// ---- cross-worker tails. Added while tuning the scorer (camelCase split +
// ---- name-token boost + function floor); these pin the *desired* shape.

#[tokio::test]
async fn presign_url_query_finds_the_camel_cased_function() {
    let deps = fixture_deps();
    let response = ask(&deps, "presign a temporary download url").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"storage::presignUrl"), "ids: {ids:?}");
}

#[tokio::test]
async fn file_listing_query_returns_filesystem_listers_not_registries() {
    let deps = fixture_deps();
    // "directory" collides with the literal `directory` worker, which no
    // lexical scorer can untangle — real fs-listing intents phrase it as
    // folder/filesystem, which is what this pins.
    let response = ask(&deps, "list the files in a folder on the filesystem").await;
    let ids = function_ids(&response);
    assert!(
        ids.iter()
            .any(|id| id.ends_with("fs::ls") || id.ends_with("list-folder")),
        "no filesystem lister in: {ids:?}"
    );
    assert!(!ids.contains(&"state::list_keys"), "ids: {ids:?}");
}

#[tokio::test]
async fn close_issue_query_prunes_the_issue_family() {
    let deps = fixture_deps();
    let response = ask(&deps, "close a github issue").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"github::issue::close"), "ids: {ids:?}");
    assert!(!ids.contains(&"github::issue::create"), "ids: {ids:?}");
    assert!(!ids.contains(&"github::issue::list"), "ids: {ids:?}");
    assert!(ids.len() <= 4, "family rode along: {ids:?}");
}

#[tokio::test]
async fn create_pr_query_stays_within_github() {
    let deps = fixture_deps();
    let response = ask(&deps, "create a new pull request").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"github::pr::create"), "ids: {ids:?}");
    assert!(
        !ids.iter().any(|id| id.starts_with("directory::")),
        "directory rode along: {ids:?}"
    );
}

#[tokio::test]
async fn merge_pr_query_keeps_merge_and_checks() {
    let deps = fixture_deps();
    let response = ask(&deps, "merge a pull request after checks pass").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"github::pr::merge"), "ids: {ids:?}");
    assert!(ids.contains(&"github::pr::checks"), "ids: {ids:?}");
    assert!(!ids.contains(&"github::pr::create"), "ids: {ids:?}");
    assert!(ids.len() <= 6, "family rode along: {ids:?}");
}

#[tokio::test]
async fn console_logs_query_stays_tight() {
    let deps = fixture_deps();
    let response = ask(&deps, "read the browser console logs").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"browser::console::read"), "ids: {ids:?}");
    assert!(!ids.contains(&"browser::sessions::start"), "ids: {ids:?}");
    assert!(!ids.contains(&"browser::sessions::attach"), "ids: {ids:?}");
    assert!(ids.len() <= 6, "family rode along: {ids:?}");
}

#[tokio::test]
async fn generic_get_value_returns_getters_not_the_fp_family() {
    let deps = fixture_deps();
    let response = ask(&deps, "get the value").await;
    let ids = function_ids(&response);
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
    for tail in ["fp::drop", "fp::take", "fp::when", "fp::sortBy", "fp::nth"] {
        assert!(!ids.contains(&tail), "fp tail rode along: {ids:?}");
    }
}

#[tokio::test]
async fn stream_message_query_prefers_the_available_sender() {
    let deps = fixture_deps();
    let response = ask(&deps, "send a message to a stream group").await;
    let ids = function_ids(&response);
    // `stream` is an engine builtin and is intentionally absent from the
    // public function catalog. Pin the best callable sender in the fixture.
    assert_eq!(ids.first(), Some(&"hermes::send"), "ids: {ids:?}");
    assert!(
        !ids.contains(&"iii::queue::redrive_message"),
        "ids: {ids:?}"
    );
}

#[tokio::test]
async fn kill_process_query_skips_the_status_tail() {
    let deps = fixture_deps();
    let response = ask(&deps, "kill a running process by pid").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"shell::kill"), "ids: {ids:?}");
    // worker::status legitimately covers process/pid vocabulary the kill
    // contract lacks ("job"); pure lexical ranking cannot exclude it, so
    // pin the tail to that single semi-relevant survivor.
    assert!(ids.len() <= 2, "ids: {ids:?}");
}

#[tokio::test]
async fn compare_and_set_query_puts_the_atomic_function_first() {
    let deps = fixture_deps();
    let response = ask(&deps, "compare and set a state key atomically").await;
    let ids = function_ids(&response);
    assert_eq!(ids.first(), Some(&"state::compare-and-set"), "ids: {ids:?}");
    assert!(ids.len() <= 6, "family rode along: {ids:?}");
}

#[tokio::test]
async fn multiple_capabilities_return_every_need_in_one_call() {
    let deps = fixture_deps();
    let response = ask_capabilities(
        &deps,
        &[
            "register javascript functions on the engine bus",
            "read and write persistent state values",
            "take a screenshot of the page",
        ],
    )
    .await;
    let ids = function_ids(&response);
    assert!(
        ids.contains(&"code-runner::register_function"),
        "ids: {ids:?}"
    );
    assert!(ids.contains(&"state::set"), "ids: {ids:?}");
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
    assert!(ids.contains(&"browser::screenshot"), "ids: {ids:?}");
    assert!(
        response.workers.len() <= 6,
        "workers: {:?}",
        workers(&response)
    );
}

#[tokio::test]
async fn todo_app_capabilities_resolve_in_one_call() {
    let deps = fixture_deps();
    let response = ask_capabilities(
        &deps,
        &[
            "register todo CRUD functions on the bus with the code runner",
            "read and write persistent todo state under a scope",
        ],
    )
    .await;
    let ids = function_ids(&response);
    assert!(
        ids.contains(&"code-runner::register_function"),
        "ids: {ids:?}"
    );
    assert!(ids.contains(&"state::set"), "ids: {ids:?}");
    assert!(ids.contains(&"state::get"), "ids: {ids:?}");
}

#[tokio::test]
async fn repeated_empty_searches_stay_empty() {
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::{Context, KeyValue};
    let deps = fixture_deps();
    let context =
        Context::current_with_baggage(vec![KeyValue::new("iii.session.id", "empty-session")]);
    let _guard = context.attach();
    assert!(ask(&deps, "zzz qqq").await.workers.is_empty());
    assert!(ask(&deps, "xxx yyy").await.workers.is_empty());
    assert!(ask(&deps, "zzz presign qqq").await.workers.is_empty());
}
