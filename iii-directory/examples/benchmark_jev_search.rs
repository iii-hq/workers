//! Opt-in installed-search evaluation. See architecture/jev-search-evaluation.md.
//! No engine, registry, model downloads, or remote warmup.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use clap::{Parser, ValueEnum};
use iii_directory::config::{FunctionSearchMode, SkillsConfig};
use iii_directory::functions::registry::RegistryCache;
use iii_directory::functions::search::{
    benchmark_installed, lexical_candidate_ids, BenchmarkOutcome, Deps,
};
use iii_directory::functions::search_index::{tool_fingerprint, ToolSchema};
use iii_directory::functions::search_jev::JevSearch;
use iii_directory::functions::search_semantic::{bundle_complete, SemanticSearch};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

const CUTOFF: usize = 12;
const LANES_PER_BATCH: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    Lexical,
    Hybrid,
    Jev,
    JevShortlist,
}

impl Mode {
    fn remote(self) -> bool {
        matches!(self, Self::Jev | Self::JevShortlist)
    }
}

#[derive(Debug, Parser)]
#[command(about = "Evaluate installed function search; remote calls require --allow-remote")]
struct Args {
    #[arg(long)]
    cases: PathBuf,
    #[arg(long)]
    catalog: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_enum, value_delimiter = ',', default_value = "lexical")]
    modes: Vec<Mode>,
    #[arg(long)]
    allow_remote: bool,
    /// Existing pinned MiniLM + reranker bundle. Never downloaded by this example.
    #[arg(long)]
    model_path: Option<PathBuf>,
    #[arg(long, default_value = "3")]
    repetitions: NonZeroUsize,
    #[arg(long, default_value = "24")]
    shortlist_depth: NonZeroUsize,
    #[arg(long, default_value_t = 30_000, value_parser = clap::value_parser!(u64).range(1..=300_000))]
    hybrid_ready_timeout_ms: u64,
    #[arg(long, default_value = "jev-1.13.0")]
    jev_model: String,
    #[arg(long, default_value_t = 3_000, value_parser = clap::value_parser!(u64).range(1..=30_000))]
    jev_timeout_ms: u64,
    #[arg(long, default_value_t = 0.5)]
    jev_min_relevance: f64,
    /// Pricing assumption from the plan, not a live quote.
    #[arg(long, default_value_t = 0.042)]
    input_usd_per_million: f64,
    #[arg(long, default_value_t = 0.0)]
    output_usd_per_million: f64,
}

fn validate_args(args: &Args) -> Result<()> {
    ensure!(!args.modes.is_empty(), "at least one mode is required");
    for (i, mode) in args.modes.iter().enumerate() {
        ensure!(!args.modes[..i].contains(mode), "duplicate mode: {mode:?}");
    }
    ensure!(
        !args.modes.iter().any(|m| m.remote()) || args.allow_remote,
        "remote modes require --allow-remote and TYPESAFE_API_KEY"
    );
    ensure!(
        !args.jev_model.trim().is_empty(),
        "--jev-model must not be empty"
    );
    ensure!(
        args.jev_min_relevance.is_finite() && (0.0..=1.0).contains(&args.jev_min_relevance),
        "--jev-min-relevance must be finite and in [0, 1]"
    );
    for rate in [args.input_usd_per_million, args.output_usd_per_million] {
        ensure!(
            rate.is_finite() && rate >= 0.0,
            "prices must be finite and nonnegative"
        );
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    split: Split,
    tags: Vec<String>,
    source: String,
    capabilities: Vec<String>,
    qrels: Vec<Qrels>,
    #[serde(default)]
    description_overrides: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Split {
    Calibration,
    Holdout,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Qrels {
    acceptable_ids: Vec<String>,
    required_groups: Vec<Vec<String>>,
    expect_empty: bool,
}

fn parse_cases(bytes: &[u8], catalog: &[ToolSchema]) -> Result<Vec<Case>> {
    let cases: Vec<Case> = serde_json::from_slice(bytes).context("invalid cases JSON")?;
    ensure!(!cases.is_empty(), "cases must not be empty");
    let catalog_ids: HashSet<&str> = catalog.iter().map(|t| t.name.as_str()).collect();
    ensure!(!catalog_ids.is_empty(), "catalog must not be empty");
    ensure!(catalog_ids.len() == catalog.len(), "duplicate catalog IDs");
    let mut case_ids = HashSet::new();
    for case in &cases {
        ensure!(
            !case.id.trim().is_empty() && case_ids.insert(&case.id),
            "empty or duplicate case ID"
        );
        ensure!(
            !case.source.trim().is_empty() && !case.tags.is_empty(),
            "{}: source and tags required",
            case.id
        );
        ensure!(
            (1..=18).contains(&case.capabilities.len()),
            "{}: need 1..18 capabilities",
            case.id
        );
        ensure!(
            case.capabilities.len() == case.qrels.len(),
            "{}: one qrels entry per capability required",
            case.id
        );
        let mut queries = HashSet::new();
        for (query, qrels) in case.capabilities.iter().zip(&case.qrels) {
            ensure!(
                !query.trim().is_empty()
                    && queries.insert(query.split_whitespace().collect::<Vec<_>>().join(" ")),
                "{}: empty or duplicate capability",
                case.id
            );
            let acceptable: HashSet<&String> = qrels.acceptable_ids.iter().collect();
            ensure!(
                acceptable.len() == qrels.acceptable_ids.len(),
                "{}: duplicate acceptable IDs",
                case.id
            );
            ensure!(
                qrels.expect_empty == acceptable.is_empty(),
                "{}: expect_empty must agree with acceptable_ids",
                case.id
            );
            ensure!(
                qrels.expect_empty == qrels.required_groups.is_empty(),
                "{}: match lanes need required_groups; empty lanes cannot have them",
                case.id
            );
            for id in &qrels.acceptable_ids {
                ensure!(
                    catalog_ids.contains(id.as_str()),
                    "{}: unknown qrel ID {id}",
                    case.id
                );
            }
            for group in &qrels.required_groups {
                ensure!(
                    !group.is_empty() && group.iter().all(|id| acceptable.contains(id)),
                    "{}: each group must contain acceptable IDs",
                    case.id
                );
                ensure!(
                    group.iter().collect::<HashSet<_>>().len() == group.len(),
                    "{}: duplicate IDs in group",
                    case.id
                );
            }
        }
        for (id, description) in &case.description_overrides {
            ensure!(
                catalog_ids.contains(id.as_str()) && !description.trim().is_empty(),
                "{}: override needs existing ID and nonempty description",
                case.id
            );
        }
    }
    Ok(cases)
}

#[derive(Debug, Serialize)]
struct LaneMetrics {
    capability_index: usize,
    batch_index: usize,
    recall_at_12: Option<f64>,
    reciprocal_rank: Option<f64>,
    required_groups: usize,
    covered_groups_at_12: usize,
    covered_groups_in_response: usize,
    no_match: bool,
    false_positive_candidates_at_12: usize,
}

#[derive(Debug, Serialize)]
struct Metrics {
    lanes: Vec<LaneMetrics>,
    batches: Vec<Value>,
    recall_at_12: Option<f64>,
    mrr: Option<f64>,
    required_group_coverage: Option<f64>,
    all_required_groups_covered: Option<bool>,
    no_match_response: Option<bool>,
    false_positive_response_candidates: Option<usize>,
    candidate_count: usize,
    selected_ids_json_bytes: usize,
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, count) = values.fold((0.0, 0), |(sum, n), v| (sum + v, n + 1));
    (count > 0).then(|| sum / count as f64)
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn metrics(case: &Case, outcome: &BenchmarkOutcome) -> Result<Metrics> {
    ensure!(
        case.qrels.len() == outcome.rankings.len(),
        "{}: adapter returned {} lanes for {} capabilities",
        case.id,
        outcome.rankings.len(),
        case.qrels.len()
    );
    let selected: HashSet<&str> = outcome.selected.iter().map(String::as_str).collect();
    ensure!(
        selected.len() == outcome.selected.len(),
        "adapter selected duplicate IDs"
    );
    let mut lanes = Vec::new();
    for (index, (qrels, ranking)) in case.qrels.iter().zip(&outcome.rankings).enumerate() {
        let mut unique = HashSet::new();
        for (id, score) in ranking {
            ensure!(
                score.is_finite() && unique.insert(id),
                "invalid score or duplicate ID in lane {index}"
            );
        }
        let relevant: HashSet<&str> = qrels.acceptable_ids.iter().map(String::as_str).collect();
        let top: HashSet<&str> = ranking
            .iter()
            .take(CUTOFF)
            .map(|(id, _)| id.as_str())
            .collect();
        let group_hits = |ids: &HashSet<&str>| {
            qrels
                .required_groups
                .iter()
                .filter(|g| g.iter().any(|id| ids.contains(id.as_str())))
                .count()
        };
        lanes.push(LaneMetrics {
            capability_index: index,
            batch_index: index / LANES_PER_BATCH,
            recall_at_12: ratio(top.intersection(&relevant).count(), relevant.len()),
            reciprocal_rank: (!qrels.expect_empty).then(|| {
                ranking
                    .iter()
                    .position(|(id, _)| relevant.contains(id.as_str()))
                    .map_or(0.0, |i| 1.0 / (i + 1) as f64)
            }),
            required_groups: qrels.required_groups.len(),
            covered_groups_at_12: group_hits(&top),
            covered_groups_in_response: group_hits(&selected),
            no_match: qrels.expect_empty,
            false_positive_candidates_at_12: if qrels.expect_empty { top.len() } else { 0 },
        });
    }
    let total_groups: usize = lanes.iter().map(|l| l.required_groups).sum();
    let covered: usize = lanes.iter().map(|l| l.covered_groups_in_response).sum();
    let batches = lanes.chunks(LANES_PER_BATCH).enumerate().map(|(index, batch)| json!({
        "batch_index": index,
        "recall_at_12": mean(batch.iter().filter_map(|l| l.recall_at_12)),
        "mrr": mean(batch.iter().filter_map(|l| l.reciprocal_rank)),
        "required_group_coverage_in_response": ratio(batch.iter().map(|l| l.covered_groups_in_response).sum(), batch.iter().map(|l| l.required_groups).sum()),
        "no_match_lanes": batch.iter().filter(|l| l.no_match).count(),
        "false_positive_lanes_at_12": batch.iter().filter(|l| l.false_positive_candidates_at_12 > 0).count(),
    })).collect();
    let no_match = case.qrels.iter().all(|q| q.expect_empty);
    Ok(Metrics {
        recall_at_12: mean(lanes.iter().filter_map(|l| l.recall_at_12)),
        mrr: mean(lanes.iter().filter_map(|l| l.reciprocal_rank)),
        required_group_coverage: ratio(covered, total_groups),
        all_required_groups_covered: (total_groups > 0).then_some(covered == total_groups),
        no_match_response: no_match.then_some(outcome.selected.is_empty()),
        false_positive_response_candidates: no_match.then_some(outcome.selected.len()),
        candidate_count: outcome.selected.len(),
        selected_ids_json_bytes: serde_json::to_vec(&outcome.selected)?.len(),
        lanes,
        batches,
    })
}

/// Nearest-rank empirical quantile; null for no observations (never a made-up zero).
fn percentile(values: &[f64], quantile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 * quantile).ceil() as usize).saturating_sub(1);
    sorted.get(index.min(sorted.len() - 1)).copied()
}

#[derive(Debug, Serialize)]
struct Run {
    case_id: String,
    split: Split,
    tags: Vec<String>,
    mode: Mode,
    capability_count: usize,
    repetition: usize,
    status: &'static str,
    reason: Option<String>,
    effective_catalog_fingerprint: String,
    evaluated_catalog_entries: usize,
    shortlist_preparation_ms: f64,
    installed_wall_ms: Option<f64>,
    jev_stage_ms: Option<u64>,
    usage_status: Option<&'static str>,
    accounted_cost_usd: Option<f64>,
    accounted_cost_is_lower_bound: Option<bool>,
    metrics: Option<Metrics>,
    outcome: Option<BenchmarkOutcome>,
}

fn remote_stage_ms(mode: Mode, outcome: &BenchmarkOutcome) -> Option<u64> {
    // A successful exact/intrinsic/empty-pool bypass is not an observation of
    // remote latency. Zero-request, zero-elapsed failures still count against
    // completion, but provide no observation of remote-stage latency.
    (mode.remote()
        && (outcome.jev_requests > 0 || (!outcome.jev_complete && outcome.jev_elapsed_ms > 0)))
        .then_some(outcome.jev_elapsed_ms)
}

fn record_remote_measurements(args: &Args, run: &mut Run, outcome: &BenchmarkOutcome) {
    run.jev_stage_ms = remote_stage_ms(run.mode, outcome);
    if !run.mode.remote() {
        return;
    }
    let known_usage = outcome.jev_complete
        || outcome.jev_requests > 0
        || outcome.jev_questions > 0
        || outcome.input_tokens > 0
        || outcome.output_tokens > 0;
    run.usage_status = Some(if outcome.jev_complete {
        "complete"
    } else if known_usage {
        "partial"
    } else {
        "unavailable"
    });
    run.accounted_cost_is_lower_bound = Some(!outcome.jev_complete);
    run.accounted_cost_usd = known_usage.then(|| {
        (outcome.input_tokens as f64 * args.input_usd_per_million
            + outcome.output_tokens as f64 * args.output_usd_per_million)
            / 1_000_000.0
    });
}

fn configuration(args: &Args, mode: Mode) -> SkillsConfig {
    SkillsConfig {
        registry_search: false,
        function_search_mode: match mode {
            Mode::Lexical => FunctionSearchMode::Lexical,
            Mode::Hybrid => FunctionSearchMode::Hybrid,
            Mode::Jev | Mode::JevShortlist => FunctionSearchMode::Jev,
        },
        function_search_model_path: if mode == Mode::Hybrid {
            args.model_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        },
        function_search_model_download: false,
        function_search_jev_model: args.jev_model.clone(),
        function_search_jev_timeout_ms: args.jev_timeout_ms,
        function_search_jev_min_relevance: args.jev_min_relevance,
        ..SkillsConfig::default()
    }
}

fn deps(
    config: SkillsConfig,
    catalog: Arc<Vec<ToolSchema>>,
    semantic: SemanticSearch,
    jev: JevSearch,
) -> Deps {
    Deps {
        config: config.into_shared(),
        catalog: Arc::new(RwLock::new(catalog)),
        sessions: Arc::default(),
        registry_cache: RegistryCache::new(Duration::ZERO),
        semantic,
        jev,
    }
}

async fn wait_for_hybrid(deps: &Deps, probe: &[String], timeout_ms: u64) -> bool {
    // Only called with Hybrid deps. Requests/usage from this local warmup are
    // excluded. The readiness flag comes from the real production evaluator.
    tokio::time::timeout(Duration::from_millis(timeout_ms), async {
        loop {
            if benchmark_installed(deps, probe).await.hybrid_complete {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or(false)
}

fn summarize(runs: &[&Run]) -> Value {
    let measured: Vec<&Run> = runs
        .iter()
        .copied()
        .filter(|r| r.metrics.is_some())
        .collect();
    let lanes: Vec<&LaneMetrics> = measured
        .iter()
        .flat_map(|r| &r.metrics.as_ref().unwrap().lanes)
        .collect();
    let latencies: Vec<f64> = measured
        .iter()
        .filter_map(|r| r.installed_wall_ms)
        .collect();
    let jev_latencies: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.jev_stage_ms.map(|ms| ms as f64))
        .collect();
    let outcomes: Vec<&BenchmarkOutcome> = runs.iter().filter_map(|r| r.outcome.as_ref()).collect();
    let groups: usize = lanes.iter().map(|l| l.required_groups).sum();
    let covered: usize = lanes.iter().map(|l| l.covered_groups_in_response).sum();
    let no_match: Vec<&LaneMetrics> = lanes.iter().copied().filter(|l| l.no_match).collect();
    let no_match_responses: Vec<usize> = measured
        .iter()
        .filter_map(|r| r.metrics.as_ref()?.false_positive_response_candidates)
        .collect();
    let remote_evaluations: Vec<&BenchmarkOutcome> = runs
        .iter()
        .filter(|r| r.mode.remote())
        .filter_map(|r| r.outcome.as_ref())
        // Failures may report zero successful requests; they still belong in
        // the completion denominator. Only successful zero-request bypasses
        // (exact/intrinsic queries or empty shortlist) are excluded.
        .filter(|o| o.jev_requests > 0 || !o.jev_complete)
        .collect();
    json!({
        "runs": runs.len(), "measured_runs": measured.len(),
        "complete_runs": runs.iter().filter(|r| r.status == "complete").count(),
        "fallback_runs": runs.iter().filter(|r| r.status == "fallback" || r.reason.as_deref() == Some("hybrid_incomplete")).count(),
        "skipped_runs": runs.iter().filter(|r| r.status == "skipped").count(),
        "recall_at_12_macro_lanes": mean(lanes.iter().filter_map(|l| l.recall_at_12)),
        "mrr_macro_lanes": mean(lanes.iter().filter_map(|l| l.reciprocal_rank)),
        "required_group_coverage": ratio(covered, groups),
        "all_groups_covered_rate": mean(measured.iter().filter_map(|r| r.metrics.as_ref()?.all_required_groups_covered.map(|b| if b {1.0} else {0.0}))),
        "no_match_lanes": no_match.len(),
        "false_positive_lane_rate_at_12": ratio(no_match.iter().filter(|l| l.false_positive_candidates_at_12 > 0).count(), no_match.len()),
        "false_positive_candidates_at_12": no_match.iter().map(|l| l.false_positive_candidates_at_12).sum::<usize>(),
        "no_match_responses": no_match_responses.len(),
        "false_positive_response_rate": ratio(no_match_responses.iter().filter(|n| **n > 0).count(), no_match_responses.len()),
        "mean_candidates": mean(measured.iter().map(|r| r.metrics.as_ref().unwrap().candidate_count as f64)),
        "mean_selected_ids_json_bytes": mean(measured.iter().map(|r| r.metrics.as_ref().unwrap().selected_ids_json_bytes as f64)),
        "installed_wall_p50_ms": percentile(&latencies, 0.5),
        "installed_wall_p95_ms": percentile(&latencies, 0.95),
        "jev_stage_samples": jev_latencies.len(),
        "jev_stage_p50_ms": percentile(&jev_latencies, 0.5),
        "jev_stage_p95_ms": percentile(&jev_latencies, 0.95),
        "reported_jev_requests": outcomes.iter().map(|o| o.jev_requests).sum::<usize>(),
        "reported_jev_questions": outcomes.iter().map(|o| o.jev_questions).sum::<usize>(),
        "available_input_tokens": outcomes.iter().map(|o| o.input_tokens).sum::<u64>(),
        "available_output_tokens": outcomes.iter().map(|o| o.output_tokens).sum::<u64>(),
        "accounted_cost_usd": mean(runs.iter().filter_map(|r| r.accounted_cost_usd)).map(|_| runs.iter().filter_map(|r| r.accounted_cost_usd).sum::<f64>()),
        "accounted_cost_is_lower_bound": runs.iter().any(|r| r.accounted_cost_is_lower_bound.is_some()).then(|| runs.iter().any(|r| r.accounted_cost_is_lower_bound == Some(true))),
        "complete_usage_runs": runs.iter().filter(|r| r.usage_status == Some("complete")).count(),
        "partial_usage_runs": runs.iter().filter(|r| r.usage_status == Some("partial")).count(),
        "unavailable_usage_runs": runs.iter().filter(|r| r.usage_status == Some("unavailable")).count(),
        "remote_evaluation_runs": remote_evaluations.len(),
        "remote_evaluation_completion_rate": ratio(remote_evaluations.iter().filter(|o| o.jev_complete).count(), remote_evaluations.len()),
        "hybrid_complete_runs": runs.iter().filter(|r| r.mode == Mode::Hybrid).filter_map(|r| r.outcome.as_ref()).filter(|o| o.hybrid_complete).count(),
        "remote_successful_bypass_runs": runs.iter().filter(|r| r.mode.remote()).filter_map(|r| r.outcome.as_ref()).filter(|o| o.jev_requests == 0 && o.jev_complete).count(),
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn bundle_identity(root: &Path) -> Result<Value> {
    let mut hashes = BTreeMap::new();
    for prefix in ["", "reranker/"] {
        for name in [
            "onnx/model.onnx",
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            let relative = format!("{prefix}{name}");
            // SHA-256 identity for reproduction; production also checks its pinned hashes.
            hashes.insert(
                relative.clone(),
                sha256(&std::fs::read(root.join(relative))?),
            );
        }
    }
    Ok(
        json!({"path": root, "files_sha256": hashes, "verification": "pinned hashes checked by production loader; completeness checked per measured run"}),
    )
}

async fn evaluate(
    args: &Args,
    cases: &[Case],
    catalog: Arc<Vec<ToolSchema>>,
    api_key: Option<String>,
) -> Result<(Vec<Run>, Vec<Value>)> {
    let jev = JevSearch::new(api_key);
    let mut runs = Vec::new();
    let mut warmups = Vec::new();
    for &mode in &args.modes {
        let semantic = SemanticSearch::new(if mode == Mode::Hybrid {
            args.model_path.clone()
        } else {
            None
        });
        let hybrid_problem = if mode != Mode::Hybrid {
            None
        } else {
            match &args.model_path {
                None => Some("hybrid_requires_model_path"),
                Some(path) if !bundle_complete(path) => Some("hybrid_bundle_missing_or_wrong_size"),
                Some(_) if !cfg!(minilm) => Some("hybrid_not_compiled_for_target"),
                Some(_) => None,
            }
        };
        let mut previous_fingerprint = String::new();
        let mut ready = false;
        for case in cases {
            let mut full_catalog = catalog.as_ref().clone();
            for tool in &mut full_catalog {
                if let Some(description) = case.description_overrides.get(&tool.name) {
                    tool.description.clone_from(description);
                }
            }
            let full_catalog = Arc::new(full_catalog);
            let fingerprint = tool_fingerprint(&full_catalog);
            if mode == Mode::Hybrid
                && hybrid_problem.is_none()
                && previous_fingerprint != fingerprint
            {
                let warmup_deps = deps(
                    configuration(args, mode),
                    full_catalog.clone(),
                    semantic.clone(),
                    jev.clone(),
                );
                semantic.rebuild(full_catalog.clone());
                // Use a non-exact catalog operation, not an intrinsic/exact-only case
                // whose complete flag could be vacuously true without a loaded model.
                let probe = full_catalog
                    .iter()
                    .find(|t| {
                        !t.description.trim().is_empty()
                            && !lexical_candidate_ids(
                                &full_catalog,
                                std::slice::from_ref(&t.name),
                                1,
                            )
                            .is_empty()
                    })
                    .map(|t| vec![format!("Find an operation to {}", t.description)])
                    .context("hybrid needs an eligible catalog operation to probe readiness")?;
                let start = Instant::now();
                ready = wait_for_hybrid(&warmup_deps, &probe, args.hybrid_ready_timeout_ms).await;
                warmups.push(json!({"mode": mode, "catalog_fingerprint": fingerprint, "ready": ready, "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0}));
                previous_fingerprint.clone_from(&fingerprint);
            }
            for repetition in 1..=args.repetitions.get() {
                let mut run = Run {
                    case_id: case.id.clone(),
                    split: case.split,
                    tags: case.tags.clone(),
                    mode,
                    capability_count: case.capabilities.len(),
                    repetition,
                    status: "skipped",
                    reason: None,
                    effective_catalog_fingerprint: fingerprint.clone(),
                    evaluated_catalog_entries: full_catalog.len(),
                    shortlist_preparation_ms: 0.0,
                    installed_wall_ms: None,
                    jev_stage_ms: None,
                    usage_status: None,
                    accounted_cost_usd: None,
                    accounted_cost_is_lower_bound: None,
                    metrics: None,
                    outcome: None,
                };
                if let Some(reason) = hybrid_problem {
                    run.reason = Some(reason.into());
                } else if mode == Mode::Hybrid && !ready {
                    run.reason = Some("hybrid_readiness_deadline".into());
                } else {
                    let start = Instant::now();
                    let evaluated_catalog = if mode == Mode::JevShortlist {
                        let ids: HashSet<String> = lexical_candidate_ids(
                            &full_catalog,
                            &case.capabilities,
                            args.shortlist_depth.get(),
                        )
                        .into_iter()
                        .collect();
                        let filtered = Arc::new(
                            full_catalog
                                .iter()
                                .filter(|t| ids.contains(&t.name))
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                        run.shortlist_preparation_ms = start.elapsed().as_secs_f64() * 1000.0;
                        filtered
                    } else {
                        full_catalog.clone()
                    };
                    run.evaluated_catalog_entries = evaluated_catalog.len();
                    run.effective_catalog_fingerprint = tool_fingerprint(&evaluated_catalog);
                    let execution = deps(
                        configuration(args, mode),
                        evaluated_catalog,
                        semantic.clone(),
                        jev.clone(),
                    );
                    let outcome = benchmark_installed(&execution, &case.capabilities).await;
                    run.installed_wall_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
                    record_remote_measurements(args, &mut run, &outcome);
                    ensure!(
                        outcome.elapsed_ms.is_finite() && outcome.elapsed_ms >= 0.0,
                        "invalid adapter latency"
                    );
                    if mode == Mode::Hybrid && !outcome.hybrid_complete {
                        run.reason = Some("hybrid_incomplete".into());
                    } else {
                        run.status = if mode.remote() && !outcome.jev_complete {
                            "fallback"
                        } else {
                            "complete"
                        };
                        if run.status == "fallback" {
                            run.reason = Some(
                                "jev_incomplete; adapter does not expose failure reason".into(),
                            );
                        }
                        run.metrics = Some(metrics(case, &outcome)?);
                    }
                    run.outcome = Some(outcome);
                }
                runs.push(run);
            }
        }
    }
    Ok((runs, warmups))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    validate_args(&args)?;
    // Validation/opt-in precedes even reading the key; lexical/hybrid never read it.
    let api_key = if args.modes.iter().any(|m| m.remote()) {
        let key = std::env::var("TYPESAFE_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty());
        ensure!(
            key.is_some(),
            "remote modes require a nonempty TYPESAFE_API_KEY"
        );
        key
    } else {
        None
    };
    let cases_bytes = std::fs::read(&args.cases).context("reading --cases")?;
    let catalog_bytes = std::fs::read(&args.catalog).context("reading --catalog")?;
    let catalog: Vec<ToolSchema> =
        serde_json::from_slice(&catalog_bytes).context("invalid catalog JSON")?;
    let cases = parse_cases(&cases_bytes, &catalog)?;
    // Avoid overwriting either input, including via symlinks.
    if let Ok(output) = args.output.canonicalize() {
        for input in [&args.cases, &args.catalog] {
            ensure!(
                output != input.canonicalize()?,
                "--output must differ from input files"
            );
        }
    }
    let hybrid_model = match &args.model_path {
        Some(path) if args.modes.contains(&Mode::Hybrid) && bundle_complete(path) => {
            Some(bundle_identity(path)?)
        }
        _ => None,
    };
    let (runs, warmups) = evaluate(&args, &cases, Arc::new(catalog), api_key).await?;
    let mut summaries = Vec::new();
    for &mode in &args.modes {
        for split in [Split::Calibration, Split::Holdout] {
            for capability_count in cases
                .iter()
                .map(|c| c.capabilities.len())
                .collect::<std::collections::BTreeSet<_>>()
            {
                for status in ["complete", "fallback", "skipped", "all"] {
                    let group: Vec<&Run> = runs
                        .iter()
                        .filter(|r| {
                            r.mode == mode
                                && r.split == split
                                && r.capability_count == capability_count
                                && (status == "all" || r.status == status)
                        })
                        .collect();
                    if !group.is_empty() {
                        summaries.push(json!({"mode": mode, "split": split, "capability_count": capability_count, "status": status, "metrics": summarize(&group)}));
                    }
                }
            }
        }
    }
    let report = json!({
        "schema_version": 3,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "scope": "installed production selection only; registry disabled; fresh session per repetition",
        "inputs": {"cases": {"path": args.cases, "sha256": sha256(&cases_bytes)}, "catalog": {"path": args.catalog, "sha256": sha256(&catalog_bytes)}},
        "build": {"package_version": env!("CARGO_PKG_VERSION"), "target": env!("TARGET"), "minilm_compiled": cfg!(minilm)},
        "algorithm_sources_sha256": {
            "search.rs": sha256(include_bytes!("../src/functions/search.rs")),
            "search_index.rs": sha256(include_bytes!("../src/functions/search_index.rs")),
            "search_semantic.rs": sha256(include_bytes!("../src/functions/search_semantic.rs")),
            "search_jev.rs": sha256(include_bytes!("../src/functions/search_jev.rs")),
            "benchmark_jev_search.rs": sha256(include_bytes!("benchmark_jev_search.rs")),
        },
        "settings": {
            "modes": args.modes, "repetitions": args.repetitions, "allow_remote": args.allow_remote,
            "shortlist_depth_per_query": args.shortlist_depth, "shortlist_pool": "union across all capabilities, then same production batching on reduced catalog",
            "capabilities_per_batch": LANES_PER_BATCH, "max_batches": 3, "candidate_budget_per_batch": CUTOFF,
            "hybrid_ready_timeout_ms": args.hybrid_ready_timeout_ms, "hybrid_model": hybrid_model,
            "requested_jev_model": args.jev_model, "effective_jev_model": null,
            "jev_timeout_ms": args.jev_timeout_ms, "jev_min_relevance": args.jev_min_relevance,
            "input_usd_per_million": args.input_usd_per_million, "output_usd_per_million": args.output_usd_per_million,
            "pricing_basis": "user-supplied assumption; defaults from 2026-09-17 plan, no live pricing lookup",
            "usage_basis": "known usage from validated blocks, including blocks preceding a batch failure; partial/unavailable usage makes cost a lower bound; failed runs with no known usage have null cost; failed/cancelled attempts may incur additional unreported charges",
            "request_count_basis": "validated requests/questions, including blocks preceding a batch failure; not all attempts; failed zero-count runs still count against completion",
            "response_size_basis": "selected_ids_json_bytes is a proxy only; full public response bytes and tokenizer counts unavailable",
            "latency_basis": "installed wall includes shortlist preparation and dependency setup; excludes model warmup and registry; jev_stage_ms is adapter-measured Jev elapsed summed across batches, including failed stages; successful zero-request bypasses and zero-request zero-elapsed failures excluded from Jev percentiles, but failures always count against completion",
            "percentile_method": "nearest rank; repetitions are not independent relevance judgments",
        },
        "warmups": warmups, "summaries": summaries, "runs": runs,
    });
    let mut output = serde_json::to_vec_pretty(&report)?;
    output.push(b'\n');
    std::fs::write(&args.output, output).context("writing --output")?;
    eprintln!(
        "Wrote {} runs for {} cases to {}",
        runs.len(),
        cases.len(),
        args.output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<ToolSchema> {
        serde_json::from_str(include_str!("../tests/fixtures/discover_catalog.json")).unwrap()
    }

    fn fixture() -> Vec<Case> {
        parse_cases(
            include_bytes!("../tests/fixtures/jev_search_cases.json"),
            &catalog(),
        )
        .unwrap()
    }

    fn try_cli(extra: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(
            [
                "benchmark",
                "--cases",
                "cases.json",
                "--catalog",
                "catalog.json",
                "--output",
                "out.json",
            ]
            .into_iter()
            .chain(extra.iter().copied()),
        )
    }

    fn cli(extra: &[&str]) -> Args {
        try_cli(extra).unwrap()
    }

    #[test]
    fn cli_defaults_are_local_and_remote_requires_explicit_opt_in() {
        let local = cli(&[]);
        assert_eq!(local.modes, [Mode::Lexical]);
        assert_eq!(local.repetitions.get(), 3);
        validate_args(&local).unwrap();
        assert!(validate_args(&cli(&["--modes", "jev"])).is_err());
        assert!(validate_args(&cli(&["--modes", "jev-shortlist"])).is_err());
        validate_args(&cli(&["--modes", "lexical,jev", "--allow-remote"])).unwrap();
        assert!(validate_args(&cli(&["--modes", "lexical,lexical"])).is_err());
        for flag in ["--repetitions", "--shortlist-depth"] {
            for invalid in ["0", "-1"] {
                assert!(try_cli(&[flag, invalid]).is_err());
            }
        }
        let mut args = cli(&[]);
        args.jev_min_relevance = f64::NAN;
        assert!(validate_args(&args).is_err());
    }

    #[test]
    fn fixture_has_real_ids_splits_and_one_six_eighteen_lanes() {
        let cases = fixture();
        assert!(cases.len() >= 20);
        for count in [1, 6, 18] {
            assert!(cases.iter().any(|c| c.capabilities.len() == count));
        }
        for split in [Split::Calibration, Split::Holdout] {
            assert!(cases.iter().any(|c| c.split == split));
        }
        assert!(cases.iter().any(|c| !c.description_overrides.is_empty()));
    }

    #[test]
    fn strict_fixture_rejects_typos_unknown_ids_and_misaligned_qrels() {
        let base: Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/jev_search_cases.json"))
                .unwrap();
        for mutation in 0..5 {
            let mut value = base.clone();
            match mutation {
                0 => value[0]["typo"] = json!(true),
                1 => value[0]["qrels"][0]["typo"] = json!(true),
                2 => value[0]["qrels"][0]["acceptable_ids"] = json!(["missing::function"]),
                3 => value[0]["qrels"] = json!([]),
                _ => value[0]["qrels"][0]["expect_empty"] = json!(true),
            }
            assert!(parse_cases(&serde_json::to_vec(&value).unwrap(), &catalog()).is_err());
        }
    }

    #[test]
    fn metrics_use_each_lane_ranking_and_response_group_coverage() {
        let cases = fixture();
        let case = cases.iter().find(|c| c.id == "multi-three").unwrap();
        let outcome = BenchmarkOutcome {
            selected: vec![
                "browser::screenshot".into(),
                "state::get".into(),
                "code-runner::register_function".into(),
            ],
            rankings: vec![
                vec![("code-runner::register_function".into(), 1.0)],
                vec![("state::delete".into(), 1.0), ("state::get".into(), 0.9)],
                vec![("browser::screenshot".into(), 1.0)],
            ],
            ..BenchmarkOutcome::default()
        };
        let m = metrics(case, &outcome).unwrap();
        assert_eq!(m.lanes[1].recall_at_12, Some(0.5));
        assert_eq!(m.lanes[1].reciprocal_rank, Some(0.5));
        assert_eq!(m.mrr, Some(2.5 / 3.0));
        assert_eq!(m.required_group_coverage, Some(0.75));
        assert_eq!(m.all_required_groups_covered, Some(false));
    }

    #[test]
    fn recall_cutoff_and_mrr_are_distinct() {
        let case = fixture().remove(1); // shell-command
        let mut ranking: Vec<(String, f64)> =
            (0..12).map(|n| (format!("unrelated::{n}"), 1.0)).collect();
        ranking.push(("shell::exec".into(), 0.5));
        let m = metrics(
            &case,
            &BenchmarkOutcome {
                rankings: vec![ranking],
                ..BenchmarkOutcome::default()
            },
        )
        .unwrap();
        assert_eq!(m.recall_at_12, Some(0.0));
        assert_eq!(m.mrr, Some(1.0 / 13.0));
        assert_eq!(m.required_group_coverage, Some(0.0));
    }

    #[test]
    fn no_match_denominators_are_null_and_false_positives_are_counted() {
        let cases = fixture();
        let case = cases.iter().find(|c| c.id == "gibberish").unwrap();
        let m = metrics(
            case,
            &BenchmarkOutcome {
                selected: vec!["state::get".into()],
                rankings: vec![vec![("state::get".into(), 1.0)]],
                ..BenchmarkOutcome::default()
            },
        )
        .unwrap();
        assert_eq!(m.recall_at_12, None);
        assert_eq!(m.mrr, None);
        assert_eq!(m.required_group_coverage, None);
        assert_eq!(m.lanes[0].false_positive_candidates_at_12, 1);
        assert_eq!(m.false_positive_response_candidates, Some(1));
        assert!(metrics(case, &BenchmarkOutcome::default()).is_err());
    }

    #[test]
    fn alternative_ids_satisfy_one_group_without_claiming_full_recall() {
        let case = fixture().remove(2); // repo-stars
        let m = metrics(
            &case,
            &BenchmarkOutcome {
                selected: vec!["github::repo::view".into()],
                rankings: vec![vec![("github::repo::view".into(), 1.0)]],
                ..BenchmarkOutcome::default()
            },
        )
        .unwrap();
        assert_eq!(m.required_group_coverage, Some(1.0));
        assert_eq!(m.recall_at_12, Some(0.5));
    }

    #[test]
    fn nearest_rank_percentiles_and_sha256_are_reproducible() {
        assert_eq!(percentile(&[], 0.95), None);
        assert_eq!(percentile(&[30.0, 10.0, 20.0], 0.5), Some(20.0));
        assert_eq!(percentile(&[30.0, 10.0, 20.0], 0.95), Some(30.0));
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn remote_stage_uses_measured_elapsed_including_zero_usage_failures() {
        let mut outcome = BenchmarkOutcome {
            jev_elapsed_ms: 2_900,
            elapsed_ms: 4_200.0,
            ..BenchmarkOutcome::default()
        };
        assert_eq!(remote_stage_ms(Mode::Jev, &outcome), Some(2_900));
        assert_eq!(remote_stage_ms(Mode::JevShortlist, &outcome), Some(2_900));
        assert_eq!(remote_stage_ms(Mode::Lexical, &outcome), None);
        assert_eq!(remote_stage_ms(Mode::Hybrid, &outcome), None);
        outcome.jev_complete = true;
        assert_eq!(remote_stage_ms(Mode::Jev, &outcome), None);
        outcome.jev_requests = 1;
        outcome.jev_elapsed_ms = 500;
        assert_eq!(remote_stage_ms(Mode::Jev, &outcome), Some(500));
        outcome.jev_elapsed_ms = 0;
        assert_eq!(remote_stage_ms(Mode::Jev, &outcome), Some(0));
        outcome.jev_complete = false;
        outcome.jev_requests = 0;
        outcome.jev_elapsed_ms = 0;
        assert_eq!(remote_stage_ms(Mode::Jev, &outcome), None);
    }

    #[tokio::test]
    async fn missing_hybrid_bundle_is_skipped_without_quality_or_latency() {
        let args = cli(&["--modes", "hybrid", "--repetitions", "1"]);
        let cases = fixture();
        let (runs, warmups) = evaluate(&args, &cases[..1], Arc::new(catalog()), None)
            .await
            .unwrap();
        assert!(warmups.is_empty());
        assert_eq!(runs[0].status, "skipped");
        assert_eq!(
            runs[0].reason.as_deref(),
            Some("hybrid_requires_model_path")
        );
        assert!(runs[0].metrics.is_none() && runs[0].installed_wall_ms.is_none());
        let summary = summarize(&[&runs[0]]);
        assert!(summary["mrr_macro_lanes"].is_null());
        assert!(summary["installed_wall_p95_ms"].is_null());
        assert!(summary["jev_stage_p95_ms"].is_null());
    }

    #[tokio::test]
    async fn eighteen_lane_adapter_smoke_and_summary_accounting_are_offline() {
        let args = cli(&["--repetitions", "1", "--output-usd-per-million", "0.1"]);
        let cases = fixture();
        let case = cases.iter().find(|c| c.id == "multi-eighteen").unwrap();
        let (mut runs, _) = evaluate(&args, std::slice::from_ref(case), Arc::new(catalog()), None)
            .await
            .unwrap();
        assert_eq!(runs[0].status, "complete");
        let measured = runs[0].metrics.as_ref().unwrap();
        assert_eq!(measured.lanes.len(), 18);
        assert_eq!(measured.batches.len(), 3);
        assert_eq!(runs[0].outcome.as_ref().unwrap().jev_requests, 0);
        assert!(runs[0].jev_stage_ms.is_none());
        assert!(runs[0].usage_status.is_none());

        // Synthetic counters exercise report accounting, not a remote client.
        let (mut failed, _) =
            evaluate(&args, std::slice::from_ref(case), Arc::new(catalog()), None)
                .await
                .unwrap();
        runs[0].mode = Mode::Jev;
        let mut success = runs[0].outcome.take().unwrap();
        success.jev_complete = true;
        success.jev_requests = 4;
        success.jev_questions = 96;
        success.input_tokens = 120;
        success.output_tokens = 30;
        success.jev_elapsed_ms = 100;
        record_remote_measurements(&args, &mut runs[0], &success);
        runs[0].outcome = Some(success);
        failed[0].mode = Mode::Jev;
        failed[0].status = "fallback";
        let mut partial = failed[0].outcome.take().unwrap();
        partial.jev_elapsed_ms = 2_900;
        partial.jev_requests = 1;
        partial.jev_questions = 24;
        partial.input_tokens = 50;
        partial.output_tokens = 10;
        record_remote_measurements(&args, &mut failed[0], &partial);
        failed[0].outcome = Some(partial);
        assert_eq!(failed[0].usage_status, Some("partial"));
        assert!(failed[0].accounted_cost_usd.unwrap() > 0.0);
        assert_eq!(failed[0].accounted_cost_is_lower_bound, Some(true));
        assert_eq!(runs[0].usage_status, Some("complete"));
        assert_eq!(runs[0].accounted_cost_is_lower_bound, Some(false));

        let (mut unknown, _) =
            evaluate(&args, std::slice::from_ref(case), Arc::new(catalog()), None)
                .await
                .unwrap();
        unknown[0].mode = Mode::Jev;
        unknown[0].status = "fallback";
        let unavailable = unknown[0].outcome.take().unwrap();
        record_remote_measurements(&args, &mut unknown[0], &unavailable);
        unknown[0].outcome = Some(unavailable);
        assert_eq!(unknown[0].usage_status, Some("unavailable"));
        assert_eq!(unknown[0].accounted_cost_usd, None);
        assert_eq!(unknown[0].jev_stage_ms, None);
        let unknown_summary = summarize(&[&unknown[0]]);
        assert!(unknown_summary["accounted_cost_usd"].is_null());
        assert!(unknown_summary["jev_stage_p95_ms"].is_null());
        assert_eq!(unknown_summary["remote_evaluation_completion_rate"], 0.0);
        assert_eq!(unknown_summary["accounted_cost_is_lower_bound"], true);

        let summary = summarize(&[&runs[0], &failed[0], &unknown[0]]);
        assert_eq!(summary["remote_evaluation_completion_rate"], 1.0 / 3.0);
        assert_eq!(summary["reported_jev_requests"], 5);
        assert_eq!(summary["reported_jev_questions"], 120);
        assert_eq!(summary["available_input_tokens"], 170);
        assert_eq!(summary["available_output_tokens"], 40);
        let expected_cost =
            (170.0 * args.input_usd_per_million + 40.0 * args.output_usd_per_million) / 1_000_000.0;
        assert!((summary["accounted_cost_usd"].as_f64().unwrap() - expected_cost).abs() < 1e-12);
        assert_eq!(summary["accounted_cost_is_lower_bound"], true);
        assert_eq!(summary["complete_usage_runs"], 1);
        assert_eq!(summary["partial_usage_runs"], 1);
        assert_eq!(summary["unavailable_usage_runs"], 1);
        assert_eq!(summary["fallback_runs"], 2);
        assert_eq!(summary["jev_stage_samples"], 2);
        assert_eq!(summary["jev_stage_p50_ms"], 100.0);
        assert_eq!(summary["jev_stage_p95_ms"], 2_900.0);
        let complete_summary = summarize(&[&runs[0]]);
        assert_eq!(complete_summary["accounted_cost_is_lower_bound"], false);
        assert_eq!(complete_summary["jev_stage_p95_ms"], 100.0);
    }
}
