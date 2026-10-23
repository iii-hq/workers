//! What people decide about a suggestion: its lifecycle, the criterion a
//! validation is judged by, the evidence computed in code from the attached
//! E2E runs and the person's verdict, plus the recurrence query after a
//! release. Rows live in the `eval_suggestion` scope, apart from the analysis.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::contract::*;
use crate::diagnostics;
use crate::error::EvalError;
use crate::runtime::{suggestion_at, Deps};
use crate::{ids, state};

const MAX_TEXT_CHARS: usize = 500;
const MAX_RATIONALE_CHARS: usize = 4_000;
const MAX_AUTHOR_CHARS: usize = 80;
const MAX_CONTROLS: usize = 20;
/// Most runs of a scenario one side of a validation may ask for.
pub const MAX_RUNS: u32 = 20;

fn invalid(message: impl Into<String>) -> EvalError {
    EvalError::InvalidRequest(message.into())
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// The analysis and its assets, once it has ended. `action` completes the
/// refusal ("attach E2E results" → "… after the analysis finishes").
pub async fn terminal_analysis(
    deps: &Deps,
    evaluation_id: &str,
    action: &str,
) -> Result<(AnalysisRecordV1, AnalysisAssetsV1), EvalError> {
    let record = state::get_record(&deps.iii, evaluation_id)
        .await?
        .ok_or_else(|| EvalError::NotFound(evaluation_id.into()))?;
    if !record.status.is_terminal() {
        return Err(EvalError::Conflict(format!(
            "{action} after the analysis finishes"
        )));
    }
    let assets = state::get_assets(&deps.iii, evaluation_id).await?;
    Ok((record, assets))
}

/// The row of one suggestion: the stored one, or the `new` row an untouched
/// suggestion reads as (not saved until somebody acts on it).
pub async fn row_for(
    deps: &Deps,
    assets: &AnalysisAssetsV1,
    evaluation_id: &str,
    index: usize,
) -> Result<SuggestionReviewV1, EvalError> {
    if let Some(row) = state::get_review(&deps.iii, evaluation_id, index).await? {
        return Ok(row);
    }
    let suggestion = suggestion_at(assets, index)?;
    // The diagnostics whose evidence the suggestion cites.
    let cited: Vec<(&str, &str)> = suggestion
        .evidence
        .iter()
        .map(|entry| (entry.session_id.as_str(), entry.entry_id.as_str()))
        .collect();
    let mut patterns: Vec<String> = assets
        .snapshot
        .iter()
        .flat_map(|snapshot| &snapshot.diagnostics)
        .filter(|diagnostic| {
            diagnostic
                .evidence
                .iter()
                .any(|entry| cited.contains(&(entry.session_id.as_str(), entry.entry_id.as_str())))
        })
        .map(DiagnosticV1::pattern)
        .collect();
    patterns.sort();
    patterns.dedup();
    Ok(SuggestionReviewV1 {
        evaluation_id: evaluation_id.into(),
        suggestion_index: index,
        title: suggestion.title.clone(),
        harness_component: suggestion.harness_component.clone(),
        scenario_id: suggestion.validation.scenario_id.clone(),
        patterns,
        lifecycle: LifecycleV1 {
            status: LifecycleStatusV1::New,
            pr: None,
            version: None,
            reason: None,
            duplicate_of: None,
            history: Vec::new(),
        },
        criterion: None,
        run: None,
        first_run_at: None,
        evidence: None,
        verdict: None,
        reproductions: Vec::new(),
    })
}

/// One row per suggestion of the analysis, in order.
pub async fn rows_for(
    deps: &Deps,
    assets: &AnalysisAssetsV1,
    evaluation_id: &str,
) -> Result<Vec<SuggestionReviewV1>, EvalError> {
    let count = assets
        .investigation
        .as_ref()
        .map_or(0, |investigation| investigation.suggestions.len());
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        rows.push(row_for(deps, assets, evaluation_id, index).await?);
    }
    Ok(rows)
}

/// Whether results of the suggestion's validation already exist: a pair was
/// attached or an execution was started for it.
pub fn data_exists(row: &SuggestionReviewV1, assets: &AnalysisAssetsV1) -> bool {
    first_data_at(row, assets).is_some()
}

/// When results first existed, as early as the evidence allows: results
/// cannot predate the start of an execution, so each attached execution counts
/// from its own `started_at` (an old pair attached today was not unseen until
/// today), besides the attach itself, the executions this worker requested
/// and the ones a restart replaced.
fn first_data_at(row: &SuggestionReviewV1, assets: &AnalysisAssetsV1) -> Option<i64> {
    let requested = row
        .run
        .as_ref()
        .filter(|run| run.baseline_execution_id.is_some() || run.candidate_execution_id.is_some())
        .map(|run| run.started_at);
    assets
        .validations
        .iter()
        .filter(|link| link.suggestion_index == row.suggestion_index)
        .flat_map(|link| {
            [
                Some(link.attached_at),
                link.baseline.started_at,
                link.candidate.started_at,
            ]
        })
        .flatten()
        .chain(requested)
        .chain(row.first_run_at)
        .min()
}

// ---------------------------------------------------------------------------
// Authors and text
// ---------------------------------------------------------------------------

/// The user the eval worker runs as: `$USER`, else `$LOGNAME`.
pub fn host_user() -> Option<String> {
    ["USER", "LOGNAME"].into_iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

/// Who made a change: the name the console sends (`by`), else the host's user,
/// else the engine-stamped caller worker id. A browser's worker id is an opaque
/// uuid, so it names nobody and comes last.
pub fn author(
    by: Option<&str>,
    host_user: Option<&str>,
    caller: Option<&str>,
) -> Result<String, EvalError> {
    let name = [by, host_user, caller]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .ok_or_else(|| {
            invalid("`by` is required: the host has no user and the caller no identity")
        })?;
    if name.chars().count() > MAX_AUTHOR_CHARS {
        return Err(invalid(format!(
            "the author is longer than {MAX_AUTHOR_CHARS} characters"
        )));
    }
    Ok(name.into())
}

/// Trimmed text; blank is absent.
fn text(field: &str, value: Option<&str>, max: usize) -> Result<Option<String>, EvalError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > max {
        return Err(invalid(format!("{field} is longer than {max} characters")));
    }
    Ok(Some(value.into()))
}

pub(crate) fn required(field: &str, value: Option<&str>, max: usize) -> Result<String, EvalError> {
    text(field, value, max)?.ok_or_else(|| invalid(format!("{field} is required")))
}

// ---------------------------------------------------------------------------
// Semantic versions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Identifier {
    Number(u64),
    Text(String),
}

/// `major.minor.patch` with an optional `-pre.release`; a leading `v` and
/// `+build` metadata are ignored. The Harness's versions are all of this
/// shape, so no dependency is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: (u64, u64, u64),
    pre: Vec<Identifier>,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = core.split('.').map(|part| part.parse::<u64>().ok());
        let core = (numbers.next()??, numbers.next()??, numbers.next()??);
        if numbers.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|part| {
                    (!part.is_empty()).then(|| {
                        part.parse()
                            .map_or_else(|_| Identifier::Text(part.into()), Identifier::Number)
                    })
                })
                .collect::<Option<_>>()?,
        };
        Some(Self { core, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.core.cmp(&other.core).then_with(|| {
            // A release is above its own pre-releases.
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// `eval::review`: validates, applies one action under the analysis lock and
/// returns the row. Only terminal analyses are reviewed.
pub async fn review(
    deps: &Deps,
    request: ReviewRequestV1,
) -> Result<SuggestionReviewV1, EvalError> {
    let by = author(
        request.by.as_deref(),
        deps.host_user.as_deref(),
        request.caller_worker_id.as_deref(),
    )?;
    let _guard = deps.locks.guard(&request.evaluation_id).await;
    let (_, assets) = terminal_analysis(deps, &request.evaluation_id, "review suggestions").await?;
    let mut row = row_for(
        deps,
        &assets,
        &request.evaluation_id,
        request.suggestion_index,
    )
    .await?;
    let now = ids::now_ms();
    match request.action {
        ReviewActionV1::SetLifecycle => set_lifecycle(&mut row, &request, by, now)?,
        ReviewActionV1::SetCriterion => {
            let input = request
                .criterion
                .as_ref()
                .ok_or_else(|| invalid("criterion is required for set_criterion"))?;
            let results_exist = data_exists(&row, &assets);
            register_criterion(
                &mut row,
                input,
                request.scenario_id.as_deref(),
                results_exist,
                by,
                now,
            )?;
        }
        ReviewActionV1::SetVerdict => set_verdict(&mut row, &assets, &request, by, now)?,
    }
    state::put_review(&deps.iii, &row).await?;
    Ok(row)
}

fn set_lifecycle(
    row: &mut SuggestionReviewV1,
    request: &ReviewRequestV1,
    by: String,
    now: i64,
) -> Result<(), EvalError> {
    let status = request
        .status
        .ok_or_else(|| invalid("status is required for set_lifecycle"))?;
    let current = &row.lifecycle;
    if status == LifecycleStatusV1::New {
        return Err(invalid("a suggestion cannot go back to new"));
    }
    if current.status == LifecycleStatusV1::Shipped && status != LifecycleStatusV1::Shipped {
        return Err(EvalError::Conflict(
            "a shipped suggestion stays shipped; recurrence is measured from its version".into(),
        ));
    }
    let shipped = status == LifecycleStatusV1::Shipped;
    let pr = text("pr", request.pr.as_deref(), MAX_TEXT_CHARS)?.or_else(|| current.pr.clone());
    let version = text("version", request.version.as_deref(), MAX_TEXT_CHARS)?;
    if version.is_some() && !shipped {
        return Err(invalid("version only applies to a shipped suggestion"));
    }
    let version = version.or_else(|| current.version.clone().filter(|_| shipped));
    if let Some(version) = &version {
        if Version::parse(version).is_none() {
            return Err(invalid(format!(
                "version `{version}` is not a semantic version like 1.8.43"
            )));
        }
    }
    let reason = text("reason", request.reason.as_deref(), MAX_TEXT_CHARS)?;
    let duplicate_of = text(
        "duplicate_of",
        request.duplicate_of.as_deref(),
        MAX_TEXT_CHARS,
    )?;
    let (reason, duplicate_of) = match status {
        LifecycleStatusV1::Rejected => (
            Some(reason.ok_or_else(|| invalid("a rejection needs a reason"))?),
            None,
        ),
        LifecycleStatusV1::Duplicate => (
            None,
            Some(duplicate_of.ok_or_else(|| invalid("a duplicate needs duplicate_of"))?),
        ),
        _ => (None, None),
    };
    if shipped && pr.is_none() {
        return Err(invalid("a shipped suggestion needs the pr that shipped it"));
    }
    let note = text("note", request.note.as_deref(), MAX_TEXT_CHARS)?;
    let mut history = std::mem::take(&mut row.lifecycle.history);
    history.push(LifecycleEventV1 {
        status,
        at: now,
        by,
        note,
    });
    row.lifecycle = LifecycleV1 {
        status,
        pr,
        version,
        reason,
        duplicate_of,
        history,
    };
    Ok(())
}

/// Registers the criterion a validation is judged by, before its results.
/// The same criterion again keeps its registration time; a different one is
/// refused once results exist, because a criterion chosen after seeing them
/// proves nothing.
pub fn register_criterion(
    row: &mut SuggestionReviewV1,
    input: &CriterionInputV1,
    scenario: Option<&str>,
    results_exist: bool,
    by: String,
    now: i64,
) -> Result<(), EvalError> {
    let scenario_id = match text("scenario_id", scenario, MAX_TEXT_CHARS)? {
        Some(scenario) => scenario,
        None => row.scenario_id.clone().ok_or_else(|| {
            invalid("scenario_id is required: the suggestion's plan names no scenario")
        })?,
    };
    if !input.min_effect.is_finite() || input.min_effect <= 0.0 {
        return Err(invalid("min_effect must be a number above 0"));
    }
    if input.metric == CriterionMetricV1::PassRate && input.min_effect > 100.0 {
        return Err(invalid(
            "a pass rate cannot move by more than 100 percentage points: min_effect is at most 100",
        ));
    }
    if !(1..=MAX_RUNS).contains(&input.min_runs) {
        return Err(invalid(format!(
            "min_runs must be between 1 and {MAX_RUNS}"
        )));
    }
    let pattern = text("pattern", input.pattern.as_deref(), MAX_TEXT_CHARS)?;
    match (input.metric, &pattern) {
        (CriterionMetricV1::SignalPerRun, Some(pattern)) if row.patterns.contains(pattern) => {}
        (CriterionMetricV1::SignalPerRun, _) => {
            return Err(invalid(format!(
                "signal_per_run needs a pattern the suggestion cites: {}",
                if row.patterns.is_empty() {
                    "it cites none, so choose another metric".into()
                } else {
                    row.patterns.join(", ")
                }
            )))
        }
        (_, Some(_)) => return Err(invalid("pattern only applies to signal_per_run")),
        (_, None) => {}
    }
    let next = CriterionV1 {
        metric: input.metric,
        pattern,
        direction: input.direction,
        min_effect: input.min_effect,
        min_runs: input.min_runs,
        scenario_id,
        registered_at: now,
        registered_by: by,
    };
    if let Some(existing) = &row.criterion {
        let same = CriterionV1 {
            registered_at: existing.registered_at,
            registered_by: existing.registered_by.clone(),
            ..next.clone()
        } == *existing;
        if same {
            return Ok(());
        }
        if results_exist {
            return Err(EvalError::Conflict(
                "criterion_frozen: results already exist for this suggestion, so its criterion \
                 can no longer change"
                    .into(),
            ));
        }
    }
    row.criterion = Some(next);
    if let Some(evidence) = row.evidence.as_mut() {
        judge(evidence, row.criterion.as_ref());
    }
    Ok(())
}

fn set_verdict(
    row: &mut SuggestionReviewV1,
    assets: &AnalysisAssetsV1,
    request: &ReviewRequestV1,
    by: String,
    now: i64,
) -> Result<(), EvalError> {
    let outcome = request
        .outcome
        .ok_or_else(|| invalid("outcome is required for set_verdict"))?;
    let rationale = required(
        "rationale",
        request.rationale.as_deref(),
        MAX_RATIONALE_CHARS,
    )?;
    if request.controls_checked.len() > MAX_CONTROLS {
        return Err(invalid(format!(
            "controls_checked has more than {MAX_CONTROLS} items"
        )));
    }
    let controls_checked = request
        .controls_checked
        .iter()
        .map(|control| required("a control", Some(control), MAX_TEXT_CHARS))
        .collect::<Result<Vec<_>, _>>()?;
    if outcome == ValidationOutcomeV1::ValidatedImprovement {
        if let Some(reason) = improvement_refusal(row, assets) {
            return Err(EvalError::Conflict(format!("verdict_refused: {reason}")));
        }
    }
    row.verdict = Some(VerdictV1 {
        outcome,
        rationale,
        controls_checked,
        by,
        at: now,
        criterion_snapshot: row.criterion.clone(),
    });
    Ok(())
}

/// Why a verdict of improvement cannot be recorded, if it cannot: it needs a
/// criterion registered before any result existed and the runs to match it.
fn improvement_refusal(row: &SuggestionReviewV1, assets: &AnalysisAssetsV1) -> Option<String> {
    let Some(criterion) = &row.criterion else {
        return Some("no criterion was registered; register one before the E2E results".into());
    };
    let Some(first) = first_data_at(row, assets) else {
        return Some("no E2E results are attached or running for this suggestion".into());
    };
    if criterion.registered_at > first {
        return Some(
            "the criterion was registered after the first E2E execution started, so results \
             could already be seen; one chosen after them cannot validate"
                .into(),
        );
    }
    let Some(evidence) = &row.evidence else {
        return Some("no evidence was computed from the attached runs".into());
    };
    if evidence.scenario_id != criterion.scenario_id {
        return Some(format!(
            "the evidence is of scenario {}, the criterion measures {}",
            evidence.scenario_id, criterion.scenario_id
        ));
    }
    for (side, evidence) in [
        ("baseline", &evidence.baseline),
        ("candidate", &evidence.candidate),
    ] {
        if evidence.n < criterion.min_runs {
            return Some(format!(
                "the {side} has {} completed run(s), the criterion needs {}",
                evidence.n, criterion.min_runs
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// The scenario whose runs the evidence reads.
pub fn target_scenario(row: &SuggestionReviewV1) -> Option<&str> {
    row.criterion
        .as_ref()
        .map(|criterion| criterion.scenario_id.as_str())
        .or(row.scenario_id.as_deref())
}

/// Every run of `scenario_id` in an `execution-get` bundle. Detectors run on
/// each run's own transcript; a run that did not complete is kept and
/// identified, never dropped.
fn runs_of(bundle: &Value, scenario_id: &str, patterns: &[String]) -> Vec<EvidenceRunV1> {
    let text = |value: &Value| value.as_str().map(str::to_string);
    let reports = bundle["detail"]["reports"].as_array().into_iter().flatten();
    let scenarios = reports
        .flat_map(|report| {
            report["report"]["scenarios"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter(|scenario| scenario["scenario_id"] == scenario_id);
    scenarios
        .flat_map(|scenario| scenario["runs"].as_array().into_iter().flatten())
        .map(|run| {
            let (completion, technical) = (text(&run["completion"]), text(&run["technical"]));
            let signals = run["transcript"]["messages"].as_array().map(|messages| {
                let mut counts: BTreeMap<String, u32> = patterns
                    .iter()
                    .map(|pattern| (pattern.clone(), 0))
                    .collect();
                let session = run["session_id"].as_str().unwrap_or_default();
                for diagnostic in diagnostics::detect(session, messages).diagnostics {
                    if let Some(count) = counts.get_mut(&diagnostic.pattern()) {
                        *count += 1;
                    }
                }
                counts
            });
            EvidenceRunV1 {
                run_id: run["run_id"].as_str().unwrap_or_default().into(),
                completed: completion.as_deref() == Some("completed")
                    && technical.as_deref() == Some("valid"),
                completion,
                technical,
                status: text(&run["status"]),
                cost_usd: run["cost"]["total_usd"].as_f64(),
                wall_time_ms: run["wall_time_ms"].as_f64(),
                total_tokens: run["efficiency"]["total_tokens"].as_f64(),
                function_calls: run["efficiency"]["function_calls"].as_f64(),
                signals,
            }
        })
        .collect()
}

/// The pair's evidence for `scenario_id`: one `(execution id, bundle)` per
/// side, judged by the row's criterion.
pub fn evidence(
    row: &SuggestionReviewV1,
    scenario_id: &str,
    baseline: (&str, &Value),
    candidate: (&str, &Value),
    now: i64,
) -> EvidenceV1 {
    let side = |(execution_id, bundle): (&str, &Value)| EvidenceSideV1 {
        execution_id: execution_id.into(),
        runs: runs_of(bundle, scenario_id, &row.patterns),
        n: 0,
        mean: None,
    };
    let mut evidence = EvidenceV1 {
        scenario_id: scenario_id.into(),
        computed_at: now,
        baseline: side(baseline),
        candidate: side(candidate),
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    };
    judge(&mut evidence, row.criterion.as_ref());
    evidence
}

/// Stores the evidence of an attached pair into the suggestion's row. Called
/// under the analysis lock. `attached` marks the row's own run as attached.
pub async fn record_evidence(
    deps: &Deps,
    assets: &AnalysisAssetsV1,
    link: &ValidationLinkV1,
    bundles: (&Value, &Value),
    attached: bool,
) -> Result<(), EvalError> {
    let evaluation_id = &assets.evaluation_id;
    let mut row = row_for(deps, assets, evaluation_id, link.suggestion_index).await?;
    let mut changed = attached;
    if let Some(scenario) = target_scenario(&row) {
        let computed = evidence(
            &row,
            scenario,
            (&link.baseline.execution_id, bundles.0),
            (&link.candidate.execution_id, bundles.1),
            ids::now_ms(),
        );
        row.evidence = Some(computed);
        changed = true;
    }
    if attached {
        if let Some(run) = row.run.as_mut() {
            run.state = ValidationRunStateV1::Attached;
            run.error = None;
        }
    }
    if changed {
        state::put_review(&deps.iii, &row).await?;
    }
    Ok(())
}

fn metric_value(run: &EvidenceRunV1, criterion: &CriterionV1) -> Option<f64> {
    match criterion.metric {
        CriterionMetricV1::SignalPerRun => run
            .signals
            .as_ref()?
            .get(criterion.pattern.as_deref()?)
            .map(|count| f64::from(*count)),
        // The E2E may call an unfinished run `passed`: it never passed.
        CriterionMetricV1::PassRate if !run.completed => Some(0.0),
        CriterionMetricV1::PassRate => match run.status.as_deref()? {
            "passed" => Some(1.0),
            "failed" => Some(0.0),
            _ => None,
        },
        CriterionMetricV1::CostUsd => run.cost_usd,
        CriterionMetricV1::Duration => run.wall_time_ms,
        CriterionMetricV1::Tokens => run.total_tokens,
        CriterionMetricV1::FunctionCalls => run.function_calls,
    }
}

fn metric_label(criterion: &CriterionV1) -> String {
    match (criterion.metric, &criterion.pattern) {
        (CriterionMetricV1::SignalPerRun, Some(pattern)) => format!("signal per run {pattern}"),
        (CriterionMetricV1::SignalPerRun, None) => "signal per run".into(),
        (CriterionMetricV1::PassRate, _) => "pass rate".into(),
        (CriterionMetricV1::CostUsd, _) => "cost (USD)".into(),
        (CriterionMetricV1::Duration, _) => "wall time (ms)".into(),
        (CriterionMetricV1::Tokens, _) => "tokens".into(),
        (CriterionMetricV1::FunctionCalls, _) => "function calls".into(),
    }
}

/// A difference that equals `min_effect` on paper can land a few ulps under it
/// in floating point (0.6 - 0.5 is not 0.1).
const EFFECT_TOLERANCE: f64 = 1e-9;

/// How `min_effect` is written for a metric: its unit, and how many of the
/// mean's own units one of it is. A pass-rate mean is a fraction and its effect
/// is in percentage points; a duration mean is in milliseconds and its effect
/// in seconds.
fn effect_unit(metric: CriterionMetricV1) -> (&'static str, f64) {
    match metric {
        CriterionMetricV1::SignalPerRun => ("signals per run", 1.0),
        CriterionMetricV1::PassRate => ("pp", 0.01),
        CriterionMetricV1::CostUsd => ("USD", 1.0),
        CriterionMetricV1::Duration => ("s", 1_000.0),
        CriterionMetricV1::Tokens => ("tokens", 1.0),
        CriterionMetricV1::FunctionCalls => ("calls", 1.0),
    }
}

fn number(value: f64) -> String {
    let shown = format!("{value:.3}");
    shown
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// Fills each side's `n` and `mean` for the criterion, then the outcome:
/// code compares the baseline and candidate means, never a model. Runs with a
/// technical failure are left out and identified; an unfinished run the
/// infrastructure did not break counts, as a fail of the pass rate. The result
/// is inconclusive, with the reason, when a side has no run, more than half
/// of its runs did not complete (technical failures and incomplete runs), too
/// few runs carry the metric. Otherwise the difference of the means, in the
/// unit `min_effect` is written in (see [`effect_unit`]), decides.
pub fn judge(evidence: &mut EvidenceV1, criterion: Option<&CriterionV1>) {
    for side in [&mut evidence.baseline, &mut evidence.candidate] {
        // A run the infrastructure broke measured nothing; one the agent left
        // unfinished is a result of the Harness and counts (as a fail).
        let measured = side.runs.iter().filter(|run| run.measurable());
        let values: Option<Vec<f64>> = criterion.map(|criterion| {
            measured
                .clone()
                .filter_map(|run| metric_value(run, criterion))
                .collect()
        });
        side.n = values.as_ref().map_or_else(|| measured.count(), Vec::len) as u32;
        side.mean = values
            .filter(|values| !values.is_empty())
            .map(|values| values.iter().sum::<f64>() / values.len() as f64);
    }
    let (outcome, reason) = compare(evidence, criterion);
    evidence.computed_outcome = outcome;
    evidence.reason = reason;
}

fn compare(
    evidence: &EvidenceV1,
    criterion: Option<&CriterionV1>,
) -> (ValidationOutcomeV1, String) {
    let inconclusive = |reason: String| (ValidationOutcomeV1::Inconclusive, reason);
    let Some(criterion) = criterion else {
        return inconclusive("no criterion was registered, so there is nothing to judge by".into());
    };
    if criterion.scenario_id != evidence.scenario_id {
        return inconclusive(format!(
            "the criterion measures scenario {}, the evidence is of {}",
            criterion.scenario_id, evidence.scenario_id
        ));
    }
    for (name, side) in [
        ("baseline", &evidence.baseline),
        ("candidate", &evidence.candidate),
    ] {
        let total = side.runs.len();
        let incomplete = side.runs.iter().filter(|run| !run.completed).count();
        if total == 0 {
            return inconclusive(format!(
                "the {name} execution has no run of {}",
                evidence.scenario_id
            ));
        }
        if incomplete * 2 > total {
            return inconclusive(format!(
                "{incomplete} of {total} {name} runs did not complete (technical failure or \
                 incomplete), more than half"
            ));
        }
        if side.n < criterion.min_runs {
            return inconclusive(format!(
                "the {name} has {} completed run(s) with {}, below the {} the criterion needs",
                side.n,
                metric_label(criterion),
                criterion.min_runs
            ));
        }
    }
    let (Some(baseline), Some(candidate)) = (evidence.baseline.mean, evidence.candidate.mean)
    else {
        return inconclusive("a side has no value of the metric".into());
    };
    let (unit, scale) = effect_unit(criterion.metric);
    let change = (candidate - baseline) / scale;
    let wanted = match criterion.direction {
        DirectionV1::Decrease => -change,
        DirectionV1::Increase => change,
    };
    let outcome = if wanted + EFFECT_TOLERANCE >= criterion.min_effect {
        ValidationOutcomeV1::ValidatedImprovement
    } else if wanted - EFFECT_TOLERANCE <= -criterion.min_effect {
        ValidationOutcomeV1::Regression
    } else {
        ValidationOutcomeV1::NoImprovement
    };
    let reason = format!(
        "{}: baseline {} → candidate {} ({}{} {unit}), n={}/{}; an effect is {} {} {unit} or \
         more with at least {} runs each side",
        metric_label(criterion),
        number(baseline),
        number(candidate),
        if change > 0.0 { "+" } else { "" },
        number(change),
        evidence.baseline.n,
        evidence.candidate.n,
        match criterion.direction {
            DirectionV1::Decrease => "a decrease of",
            DirectionV1::Increase => "an increase of",
        },
        number(criterion.min_effect),
        criterion.min_runs
    );
    (outcome, reason)
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// `eval::reviews`: the stored rows and, per analysis with suggestions, how
/// many sit at each lifecycle status.
pub async fn reviews(
    deps: &Deps,
    request: ReviewsRequestV1,
) -> Result<ReviewsResponseV1, EvalError> {
    let mut rows = state::list_reviews(&deps.iii).await?;
    let mut records = state::list_records(&deps.iii).await?;
    if let Some(evaluation_id) = &request.evaluation_id {
        rows.retain(|row| &row.evaluation_id == evaluation_id);
        records.retain(|record| &record.evaluation_id == evaluation_id);
    }
    rows.sort_by(|left, right| {
        (&left.evaluation_id, left.suggestion_index)
            .cmp(&(&right.evaluation_id, right.suggestion_index))
    });
    records.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.evaluation_id.cmp(&left.evaluation_id))
    });
    let summaries = records
        .iter()
        .filter(|record| record.counters.suggestions > 0)
        .map(|record| {
            let mut summary = ReviewSummaryV1 {
                evaluation_id: record.evaluation_id.clone(),
                suggestions: record.counters.suggestions,
                ..ReviewSummaryV1::default()
            };
            for index in 0..record.counters.suggestions as usize {
                let status = rows
                    .iter()
                    .find(|row| {
                        row.evaluation_id == record.evaluation_id && row.suggestion_index == index
                    })
                    .map_or(LifecycleStatusV1::New, |row| row.lifecycle.status);
                *match status {
                    LifecycleStatusV1::New => &mut summary.new,
                    LifecycleStatusV1::Accepted => &mut summary.accepted,
                    LifecycleStatusV1::InProgress => &mut summary.in_progress,
                    LifecycleStatusV1::Shipped => &mut summary.shipped,
                    LifecycleStatusV1::Rejected => &mut summary.rejected,
                    LifecycleStatusV1::Duplicate => &mut summary.duplicate,
                } += 1;
            }
            summary
        })
        .collect();
    Ok(ReviewsResponseV1 {
        reviews: rows,
        summaries,
    })
}

/// `eval::recurrence`: per pattern of a shipped suggestion, how often the
/// analyses on Harness versions before the shipped one found it against the
/// ones from it on. Only analyses whose collection finished count (the others
/// measured nothing), and analyses with no semantic `harness_version` are
/// left out and counted.
pub async fn recurrence(
    deps: &Deps,
    request: RecurrenceRequestV1,
) -> Result<RecurrenceResponseV1, EvalError> {
    let row = state::get_review(&deps.iii, &request.evaluation_id, request.suggestion_index)
        .await?
        .ok_or_else(|| invalid("recurrence_unavailable: the suggestion is not marked shipped"))?;
    let (shipped, version) = row
        .lifecycle
        .version
        .as_deref()
        .filter(|_| row.lifecycle.status == LifecycleStatusV1::Shipped)
        .and_then(|version| Some((Version::parse(version)?, version.to_string())))
        .ok_or_else(|| {
            invalid("recurrence_unavailable: mark the suggestion shipped with its version first")
        })?;
    let records = state::list_records(&deps.iii).await?;
    let (before, from_version, without_version) =
        recurrence_windows(&records, &row.patterns, &shipped);
    Ok(RecurrenceResponseV1 {
        evaluation_id: row.evaluation_id,
        suggestion_index: row.suggestion_index,
        version,
        before,
        from_version,
        without_version,
    })
}

fn recurrence_windows(
    records: &[AnalysisRecordV1],
    patterns: &[String],
    shipped: &Version,
) -> (RecurrenceWindowV1, RecurrenceWindowV1, u32) {
    let mut windows = [(); 2].map(|()| RecurrenceWindowV1 {
        analyses: 0,
        patterns: patterns
            .iter()
            .map(|pattern| RecurrencePatternV1 {
                pattern: pattern.clone(),
                occurrences: 0,
                analyses_with: 0,
                per_analysis: None,
            })
            .collect(),
    });
    let mut without_version = 0;
    // `coverage` is set once collection read the evidence: before it,
    // an empty `signals` means nothing was measured.
    for record in records.iter().filter(|record| record.coverage.is_some()) {
        let Some(version) = record.harness_version.as_deref().and_then(Version::parse) else {
            without_version += 1;
            continue;
        };
        let window = &mut windows[usize::from(version >= *shipped)];
        window.analyses += 1;
        for pattern in &mut window.patterns {
            let count = record.signals.get(&pattern.pattern).copied().unwrap_or(0);
            pattern.occurrences += count;
            pattern.analyses_with += u32::from(count > 0);
        }
    }
    for window in &mut windows {
        for pattern in &mut window.patterns {
            pattern.per_analysis = (window.analyses > 0)
                .then(|| f64::from(pattern.occurrences) / f64::from(window.analyses));
        }
    }
    let [before, from_version] = windows;
    (before, from_version, without_version)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text} parses"))
    }

    #[test]
    fn versions_compare_like_semver() {
        for (lower, higher) in [
            ("1.8.9", "1.8.10"),
            ("1.8.43", "1.9.0"),
            ("1.9.48", "2.0.0"),
            ("1.8.43-rc.1", "1.8.43"),
            ("1.8.43-rc.2", "1.8.43-rc.10"),
            ("1.8.43-rc.1", "1.8.43-rc.1.1"),
            ("1.8.43-1", "1.8.43-alpha"),
            ("v1.8.42", "1.8.43+build.5"),
        ] {
            assert!(version(lower) < version(higher), "{lower} < {higher}");
            assert!(version(higher) > version(lower), "{higher} > {lower}");
        }
        assert_eq!(
            version("1.8.43+a").cmp(&version("v1.8.43")),
            std::cmp::Ordering::Equal
        );
        for invalid in ["", "1.8", "1.8.x", "1.8.43.1", "1.8.43-", "latest", "1..3"] {
            assert!(Version::parse(invalid).is_none(), "{invalid:?}");
        }
    }

    fn row() -> SuggestionReviewV1 {
        serde_json::from_value(json!({
            "evaluation_id": "eval_1", "suggestion_index": 0, "title": "t",
            "harness_component": "c", "scenario_id": "s",
            "patterns": ["repeated_tool_error:crm::schedule"],
            "lifecycle": {"status": "new", "history": []}
        }))
        .unwrap()
    }

    fn input(metric: CriterionMetricV1, pattern: Option<&str>) -> CriterionInputV1 {
        CriterionInputV1 {
            metric,
            pattern: pattern.map(str::to_string),
            direction: DirectionV1::Decrease,
            min_effect: 0.5,
            min_runs: 3,
        }
    }

    fn criterion(metric: CriterionMetricV1, direction: DirectionV1) -> CriterionV1 {
        CriterionV1 {
            metric,
            pattern: (metric == CriterionMetricV1::SignalPerRun)
                .then(|| "repeated_tool_error:crm::schedule".into()),
            direction,
            min_effect: 2.0,
            min_runs: 3,
            scenario_id: "s".into(),
            registered_at: 10,
            registered_by: "ana".into(),
        }
    }

    fn run(completed: bool, signal: Option<u32>, status: &str) -> EvidenceRunV1 {
        EvidenceRunV1 {
            run_id: "r".into(),
            completed,
            completion: Some(if completed { "completed" } else { "incomplete" }.into()),
            technical: Some(if completed { "valid" } else { "infrastructure" }.into()),
            status: Some(status.into()),
            cost_usd: Some(0.5),
            wall_time_ms: None,
            total_tokens: None,
            function_calls: None,
            signals: signal.map(|count| {
                BTreeMap::from([("repeated_tool_error:crm::schedule".to_string(), count)])
            }),
        }
    }

    fn side(runs: Vec<EvidenceRunV1>) -> EvidenceSideV1 {
        EvidenceSideV1 {
            execution_id: "x".into(),
            runs,
            n: 0,
            mean: None,
        }
    }

    fn evidence_of(baseline: Vec<EvidenceRunV1>, candidate: Vec<EvidenceRunV1>) -> EvidenceV1 {
        EvidenceV1 {
            scenario_id: "s".into(),
            computed_at: 0,
            baseline: side(baseline),
            candidate: side(candidate),
            computed_outcome: ValidationOutcomeV1::Inconclusive,
            reason: String::new(),
        }
    }

    fn signals(counts: &[u32]) -> Vec<EvidenceRunV1> {
        counts
            .iter()
            .map(|count| run(true, Some(*count), "passed"))
            .collect()
    }

    fn outcome(
        baseline: Vec<EvidenceRunV1>,
        candidate: Vec<EvidenceRunV1>,
        criterion: &CriterionV1,
    ) -> (ValidationOutcomeV1, String, (u32, u32)) {
        let mut evidence = evidence_of(baseline, candidate);
        judge(&mut evidence, Some(criterion));
        (
            evidence.computed_outcome,
            evidence.reason,
            (evidence.baseline.n, evidence.candidate.n),
        )
    }

    #[test]
    fn the_outcome_comes_from_the_means_and_the_minimum_effect() {
        use ValidationOutcomeV1::*;
        let decrease = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Decrease);
        // 3.0 → 0.0 is 3 signals fewer per run: above the 2 asked for.
        let (found, reason, n) = outcome(signals(&[3, 3, 3]), signals(&[0, 0, 0]), &decrease);
        assert_eq!((found, n), (ValidatedImprovement, (3, 3)));
        assert!(
            reason.contains("baseline 3 → candidate 0 (-3 signals per run), n=3/3")
                && reason.contains("a decrease of 2 signals per run or more"),
            "{reason}"
        );
        // Exactly the minimum counts; one fewer than that does not.
        assert_eq!(
            outcome(signals(&[3, 3, 3]), signals(&[1, 1, 1]), &decrease).0,
            ValidatedImprovement
        );
        assert_eq!(
            outcome(signals(&[3, 3, 3]), signals(&[2, 2, 2]), &decrease).0,
            NoImprovement
        );
        // From nothing there is nothing to reduce, and no relative change to be
        // undefined: the same rule applies.
        assert_eq!(
            outcome(signals(&[0, 0, 0]), signals(&[0, 0, 0]), &decrease).0,
            NoImprovement
        );
        // The wrong way by at least the minimum is a regression; by less is not.
        assert_eq!(
            outcome(signals(&[2, 2, 2]), signals(&[4, 4, 4]), &decrease).0,
            Regression
        );
        assert_eq!(
            outcome(signals(&[2, 2, 2]), signals(&[3, 3, 3]), &decrease).0,
            NoImprovement
        );
        // The same data under an increase criterion reads the other way.
        let increase = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Increase);
        assert_eq!(
            outcome(signals(&[2, 2, 2]), signals(&[4, 4, 4]), &increase).0,
            ValidatedImprovement
        );
        // Pass rate is the share of completed runs that passed.
        let pass = CriterionV1 {
            min_effect: 50.0,
            ..criterion(CriterionMetricV1::PassRate, DirectionV1::Increase)
        };
        let runs = |passed: usize| -> Vec<EvidenceRunV1> {
            (0..4)
                .map(|at| run(true, None, if at < passed { "passed" } else { "failed" }))
                .collect()
        };
        assert_eq!(outcome(runs(2), runs(4), &pass).0, ValidatedImprovement);
        assert_eq!(outcome(runs(2), runs(2), &pass).0, NoImprovement);
    }

    #[test]
    fn the_minimum_effect_is_in_the_metrics_own_unit() {
        use CriterionMetricV1::*;
        use ValidationOutcomeV1::*;
        // `wall_time_ms`, `cost_usd`, `total_tokens` or `function_calls` of
        // every run on a side.
        let measured = |metric: CriterionMetricV1, value: f64| -> Vec<EvidenceRunV1> {
            (0..3)
                .map(|_| {
                    let mut run = run(true, None, "passed");
                    match metric {
                        CostUsd => run.cost_usd = Some(value),
                        Duration => run.wall_time_ms = Some(value),
                        Tokens => run.total_tokens = Some(value),
                        _ => run.function_calls = Some(value),
                    }
                    run
                })
                .collect()
        };
        // (metric, minimum in its unit, baseline mean, candidate mean in the
        // mean's own unit): each a decrease of exactly the minimum.
        for (metric, min_effect, baseline, candidate, unit) in [
            (CostUsd, 0.01, 0.07, 0.06, "USD"),
            (Duration, 5.0, 30_000.0, 25_000.0, "s"),
            (Tokens, 1_000.0, 9_000.0, 8_000.0, "tokens"),
            (FunctionCalls, 2.0, 9.0, 7.0, "calls"),
        ] {
            let wanted = CriterionV1 {
                min_effect,
                ..criterion(metric, DirectionV1::Decrease)
            };
            let (found, reason, _) = outcome(
                measured(metric, baseline),
                measured(metric, candidate),
                &wanted,
            );
            assert_eq!(found, ValidatedImprovement, "{metric:?}: {reason}");
            assert!(reason.contains(&format!("{unit} or more")), "{reason}");
            // A smaller drop falls short; the same drop upwards is a regression.
            let half = baseline - (baseline - candidate) / 2.0;
            assert_eq!(
                outcome(measured(metric, baseline), measured(metric, half), &wanted).0,
                NoImprovement,
                "{metric:?}"
            );
            assert_eq!(
                outcome(
                    measured(metric, candidate),
                    measured(metric, baseline),
                    &wanted
                )
                .0,
                Regression,
                "{metric:?}"
            );
        }
        // Pass rate is in percentage points: 5 pp is 0.05 of the share, and 10
        // runs a side put the means one ulp off the paper figure.
        let pass = CriterionV1 {
            min_effect: 10.0,
            ..criterion(PassRate, DirectionV1::Increase)
        };
        let passing = |passed: usize| -> Vec<EvidenceRunV1> {
            (0..10)
                .map(|at| run(true, None, if at < passed { "passed" } else { "failed" }))
                .collect()
        };
        let (found, reason, _) = outcome(passing(5), passing(6), &pass);
        assert_eq!(found, ValidatedImprovement, "{reason}");
        assert!(
            reason.contains("baseline 0.5 → candidate 0.6 (+10 pp)")
                && reason.contains("an increase of 10 pp or more"),
            "{reason}"
        );
        assert_eq!(outcome(passing(5), passing(5), &pass).0, NoImprovement);
        assert_eq!(outcome(passing(6), passing(5), &pass).0, Regression);
    }

    #[test]
    fn an_unfinished_run_the_infrastructure_did_not_break_counts_against_the_pass_rate() {
        use ValidationOutcomeV1::*;
        let pass = criterion(CriterionMetricV1::PassRate, DirectionV1::Increase);
        let runs = |passed: usize, failed: usize| -> Vec<EvidenceRunV1> {
            (0..passed)
                .map(|_| run(true, None, "passed"))
                .chain((0..failed).map(|_| run(true, None, "failed")))
                .collect()
        };
        // The agent gave up: technically valid, not completed, and the E2E may
        // still call its status `passed`.
        let gave_up = || EvidenceRunV1 {
            completed: false,
            completion: Some("task_incomplete".into()),
            ..run(true, None, "passed")
        };
        // 3 of 5 passed on both sides: two unfinished runs are not a better
        // Harness.
        let mut candidate = runs(3, 0);
        candidate.extend([gave_up(), gave_up()]);
        let (found, reason, n) = outcome(runs(3, 2), candidate, &pass);
        assert_eq!((found, n), (NoImprovement, (5, 5)), "{reason}");
        assert!(reason.contains("baseline 0.6 → candidate 0.6"), "{reason}");
        // A technical failure is not the Harness's: it stays out of the count.
        let mut broken = runs(3, 0);
        broken.extend([run(false, None, "failed"), run(false, None, "failed")]);
        assert_eq!(outcome(runs(3, 2), broken, &pass).2, (5, 3));
        // The signal of an unfinished run is a measurement too.
        let signal = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Decrease);
        let mut quiet = signals(&[0, 0]);
        quiet.push(EvidenceRunV1 {
            completed: false,
            completion: Some("task_incomplete".into()),
            ..run(true, Some(9), "passed")
        });
        let (_, _, n) = outcome(signals(&[3, 3, 3]), quiet, &signal);
        assert_eq!(n.1, 3);
    }

    #[test]
    fn missing_data_is_inconclusive_with_the_reason() {
        use ValidationOutcomeV1::*;
        let decrease = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Decrease);
        // Fewer completed runs than min_runs on a side.
        let (found, reason, n) = outcome(signals(&[3, 3, 3]), signals(&[0, 0]), &decrease);
        assert_eq!((found, n), (Inconclusive, (3, 2)));
        assert!(
            reason.contains("the candidate has 2 completed run(s)"),
            "{reason}"
        );
        // More than half of a side's runs did not complete: identified.
        let mut flaky = signals(&[0, 0]);
        flaky.extend([
            run(false, None, "failed"),
            run(false, None, "failed"),
            run(false, None, "failed"),
        ]);
        let (found, reason, _) = outcome(signals(&[3, 3, 3]), flaky, &decrease);
        assert_eq!(found, Inconclusive);
        assert!(
            reason.contains("3 of 5 candidate runs did not complete"),
            "{reason}"
        );
        // Exactly half is tolerated while enough completed runs remain.
        let mut half = signals(&[0, 0, 0]);
        half.extend([
            run(false, None, "failed"),
            run(false, None, "failed"),
            run(false, None, "failed"),
        ]);
        assert_eq!(
            outcome(signals(&[3, 3, 3]), half, &decrease).0,
            ValidatedImprovement
        );
        // A completed run with no transcript has no signal count.
        let blind: Vec<_> = (0..3).map(|_| run(true, None, "passed")).collect();
        assert_eq!(outcome(signals(&[3, 3, 3]), blind, &decrease).2, (3, 0));
        // No run of the scenario.
        assert!(outcome(vec![], signals(&[0, 0, 0]), &decrease)
            .1
            .contains("no run of s"));
        // No criterion: counted, not judged.
        let mut evidence = evidence_of(signals(&[1, 1]), signals(&[0]));
        judge(&mut evidence, None);
        assert_eq!(evidence.computed_outcome, Inconclusive);
        assert_eq!((evidence.baseline.n, evidence.baseline.mean), (2, None));
    }

    #[test]
    fn a_criterion_is_checked_against_what_it_can_measure() {
        let mut row = row();
        let (by, now) = ("ana".to_string(), 10);
        let signal = |pattern: Option<&str>| input(CriterionMetricV1::SignalPerRun, pattern);
        for bad in [
            signal(None),
            signal(Some("repeated_tool_error:other")),
            input(
                CriterionMetricV1::PassRate,
                Some("repeated_tool_error:crm::schedule"),
            ),
            CriterionInputV1 {
                min_effect: 0.0,
                ..signal(Some("repeated_tool_error:crm::schedule"))
            },
            CriterionInputV1 {
                min_effect: f64::NAN,
                ..input(CriterionMetricV1::CostUsd, None)
            },
            CriterionInputV1 {
                min_effect: -1.0,
                ..input(CriterionMetricV1::CostUsd, None)
            },
            CriterionInputV1 {
                min_effect: f64::INFINITY,
                ..input(CriterionMetricV1::CostUsd, None)
            },
            CriterionInputV1 {
                min_effect: 100.5,
                ..input(CriterionMetricV1::PassRate, None)
            },
            CriterionInputV1 {
                min_runs: 0,
                ..input(CriterionMetricV1::CostUsd, None)
            },
            CriterionInputV1 {
                min_runs: 21,
                ..input(CriterionMetricV1::CostUsd, None)
            },
        ] {
            assert!(
                register_criterion(&mut row, &bad, None, false, by.clone(), now).is_err(),
                "{bad:?}"
            );
        }
        assert_eq!(row.criterion, None);
        // The effect is in the metric's unit, so a decrease may exceed 1: 30
        // seconds, 5000 tokens.
        for (metric, min_effect) in [
            (CriterionMetricV1::Duration, 30.0),
            (CriterionMetricV1::Tokens, 5_000.0),
        ] {
            let big = CriterionInputV1 {
                min_effect,
                ..input(metric, None)
            };
            register_criterion(&mut row, &big, None, false, by.clone(), now).unwrap();
            assert_eq!(row.criterion.as_ref().unwrap().min_effect, min_effect);
        }
        row.criterion = None;
        register_criterion(
            &mut row,
            &signal(Some("repeated_tool_error:crm::schedule")),
            None,
            false,
            by.clone(),
            now,
        )
        .unwrap();
        let first = row.criterion.clone().unwrap();
        assert_eq!((first.scenario_id.as_str(), first.registered_at), ("s", 10));
        // Without results it may change; the same one again keeps its time.
        register_criterion(
            &mut row,
            &signal(Some("repeated_tool_error:crm::schedule")),
            None,
            true,
            by.clone(),
            99,
        )
        .unwrap();
        assert_eq!(row.criterion.as_ref().unwrap().registered_at, 10);
        let other = input(CriterionMetricV1::CostUsd, None);
        let frozen = register_criterion(&mut row, &other, None, true, by.clone(), 99).unwrap_err();
        assert!(frozen.to_string().contains("criterion_frozen:"), "{frozen}");
        register_criterion(&mut row, &other, None, false, by, 99).unwrap();
        assert_eq!(
            row.criterion.as_ref().unwrap().metric,
            CriterionMetricV1::CostUsd
        );
        // A plan with no scenario needs one named.
        let mut unplanned = SuggestionReviewV1 {
            scenario_id: None,
            ..row.clone()
        };
        unplanned.criterion = None;
        assert!(register_criterion(&mut unplanned, &other, None, false, "ana".into(), 1).is_err());
        register_criterion(
            &mut unplanned,
            &other,
            Some(" timer_wake "),
            false,
            "ana".into(),
            1,
        )
        .unwrap();
        assert_eq!(unplanned.criterion.unwrap().scenario_id, "timer_wake");
    }

    fn lifecycle(
        row: &mut SuggestionReviewV1,
        status: LifecycleStatusV1,
        edit: impl FnOnce(&mut ReviewRequestV1),
    ) -> Result<(), EvalError> {
        let mut request: ReviewRequestV1 = serde_json::from_value(json!({
            "evaluation_id": "eval_1", "suggestion_index": 0, "action": "set_lifecycle"
        }))
        .unwrap();
        request.status = Some(status);
        edit(&mut request);
        set_lifecycle(row, &request, "ana".into(), 5)
    }

    #[test]
    fn the_lifecycle_records_who_moved_it_and_what_each_status_needs() {
        use LifecycleStatusV1::*;
        let mut row = row();
        assert!(
            lifecycle(&mut row, New, |_| {}).is_err(),
            "never back to new"
        );
        lifecycle(&mut row, Accepted, |request| {
            request.note = Some(" worth it ".into())
        })
        .unwrap();
        assert_eq!(row.lifecycle.history[0].note.as_deref(), Some("worth it"));
        lifecycle(&mut row, InProgress, |request| {
            request.pr = Some("#1300".into())
        })
        .unwrap();
        // A pr is kept across statuses unless replaced.
        assert!(
            lifecycle(&mut row, Rejected, |_| {}).is_err(),
            "a rejection needs a reason"
        );
        lifecycle(&mut row, Rejected, |request| {
            request.reason = Some("not worth it".into())
        })
        .unwrap();
        assert_eq!(
            (row.lifecycle.pr.as_deref(), row.lifecycle.reason.as_deref()),
            (Some("#1300"), Some("not worth it"))
        );
        // Reopening clears the reason.
        lifecycle(&mut row, Accepted, |_| {}).unwrap();
        assert_eq!(row.lifecycle.reason, None);
        assert!(lifecycle(&mut row, Duplicate, |_| {}).is_err());
        lifecycle(&mut row, Duplicate, |request| {
            request.duplicate_of = Some("#1290".into())
        })
        .unwrap();
        assert_eq!(row.lifecycle.duplicate_of.as_deref(), Some("#1290"));
        // Shipping needs the pr (already recorded here) and a semantic version.
        assert!(lifecycle(&mut row, Shipped, |request| request.version =
            Some("latest".into()))
        .is_err());
        assert!(lifecycle(&mut row, InProgress, |request| request.version =
            Some("1.8.43".into()))
        .is_err());
        lifecycle(&mut row, Shipped, |_| {}).unwrap();
        assert_eq!(
            (row.lifecycle.status, row.lifecycle.duplicate_of.clone()),
            (Shipped, None)
        );
        // The version can be added later; a shipped suggestion stays shipped.
        lifecycle(&mut row, Shipped, |request| {
            request.version = Some("1.8.43".into())
        })
        .unwrap();
        assert_eq!(row.lifecycle.version.as_deref(), Some("1.8.43"));
        let stuck = lifecycle(&mut row, Rejected, |request| {
            request.reason = Some("x".into())
        })
        .unwrap_err();
        assert!(matches!(stuck, EvalError::Conflict(_)), "{stuck:?}");
        let statuses: Vec<_> = row
            .lifecycle
            .history
            .iter()
            .map(|event| event.status)
            .collect();
        assert_eq!(
            statuses,
            [Accepted, InProgress, Rejected, Accepted, Duplicate, Shipped, Shipped]
        );
        assert!(row
            .lifecycle
            .history
            .iter()
            .all(|event| event.by == "ana" && event.at == 5));
        // A shipped suggestion with no pr anywhere is refused.
        let mut fresh = self::row();
        assert!(lifecycle(&mut fresh, Shipped, |_| {}).is_err());
        lifecycle(&mut fresh, Shipped, |request| {
            request.pr = Some("#1".into())
        })
        .unwrap();
    }

    fn link(attached_at: i64, started: [Option<i64>; 2]) -> ValidationLinkV1 {
        let side = |id: &str, started_at: Option<i64>| {
            json!({"execution_id": id, "reports_available": true, "reports": [],
                "scenarios": [], "started_at": started_at})
        };
        serde_json::from_value(json!({
            "suggestion_index": 0, "attached_at": attached_at,
            "comparability": {"comparable": true, "checks": []},
            "baseline": side("b", started[0]), "candidate": side("c", started[1]),
        }))
        .unwrap()
    }

    fn assets(links: Vec<ValidationLinkV1>) -> AnalysisAssetsV1 {
        AnalysisAssetsV1 {
            validations: links,
            ..AnalysisAssetsV1::default()
        }
    }

    #[test]
    fn results_first_existed_when_the_first_execution_started_not_when_it_was_attached() {
        let mut row = row();
        assert_eq!(first_data_at(&row, &assets(vec![])), None);
        // An old pair attached today: its executions started long before.
        let old = assets(vec![link(1_000, [Some(200), Some(150)])]);
        assert_eq!(first_data_at(&row, &old), Some(150));
        // No start time reported: the attach itself is all there is.
        let unknown = assets(vec![link(1_000, [None, None])]);
        assert_eq!(first_data_at(&row, &unknown), Some(1_000));
        // Another suggestion's pair says nothing about this one.
        let mut other = link(1, [Some(1), Some(1)]);
        other.suggestion_index = 1;
        assert_eq!(first_data_at(&row, &assets(vec![other])), None);
        // A criterion written after the old pair's executions cannot validate,
        // however recently the pair was attached.
        let criterion = CriterionV1 {
            registered_at: 500,
            ..criterion(CriterionMetricV1::PassRate, DirectionV1::Increase)
        };
        row.criterion = Some(criterion);
        let refusal = improvement_refusal(&row, &old).unwrap();
        assert!(
            refusal.contains("registered after the first E2E execution started"),
            "{refusal}"
        );
        let later = assets(vec![link(1_000, [Some(600), Some(700)])]);
        assert!(!improvement_refusal(&row, &later)
            .unwrap()
            .contains("registered after"));
    }

    #[test]
    fn a_restart_does_not_unsee_the_executions_it_replaced() {
        let mut row = row();
        // The earlier run's executions are gone from `run`, not from the past.
        row.first_run_at = Some(40);
        assert_eq!(first_data_at(&row, &assets(vec![])), Some(40));
        let run: ValidationRunV1 = serde_json::from_value(json!({
            "baseline_commit": "a", "candidate_commit": "b", "scenario_id": "s", "runs": 1,
            "model": "m", "provider": "p", "started_at": 90, "state": "starting"
        }))
        .unwrap();
        row.run = Some(run);
        // No execution was accepted by the E2E: no result can exist for it.
        assert_eq!(first_data_at(&row, &assets(vec![])), Some(40));
        row.first_run_at = None;
        assert_eq!(first_data_at(&row, &assets(vec![])), None);
    }

    #[test]
    fn authors_are_the_name_sent_then_the_hosts_user_then_the_caller() {
        let uuid = Some("fee30d6b-1890-4a8e-9d51-0c6e5d1f3a77");
        // The name the console sends wins over both.
        assert_eq!(author(Some(" ana "), Some("layon"), uuid).unwrap(), "ana");
        // Without one, the host's user: a browser's worker id names nobody.
        assert_eq!(author(None, Some("layon"), uuid).unwrap(), "layon");
        assert_eq!(author(Some(" "), Some("layon"), uuid).unwrap(), "layon");
        // No user on the host (a container without env vars): the caller.
        assert_eq!(author(None, None, Some("worker-7")).unwrap(), "worker-7");
        assert_eq!(
            author(None, Some(" "), Some("worker-7")).unwrap(),
            "worker-7"
        );
        assert!(author(None, None, None).is_err());
        assert!(author(Some(" "), Some(" "), Some(" ")).is_err());
        assert!(author(Some(&"x".repeat(81)), None, None).is_err());
    }

    /// A transcript the way the turn loop stores it: a contract fetched
    /// twice with nothing changed in between is the detectors'
    /// `repeated_contract_discovery`.
    fn transcript(repeats: bool) -> Vec<Value> {
        use harness::types::content::ContentBlock;
        use harness::types::event::StopReason;
        use harness::types::message::{
            empty_assistant, AgentMessage, FunctionResultMessage, FunctionResultRoleTag,
        };
        let info = "engine::functions::info";
        let contract = json!({"function_id": "crm::profile", "request_schema": {"type": "object"},
            "response_schema": {"type": "object"}});
        let call = |step: u32, id: &str| {
            let mut message = empty_assistant("p", "m");
            message.stop_reason = StopReason::FunctionCall;
            message.content = vec![ContentBlock::FunctionCall {
                id: id.into(),
                function_id: "agent_trigger".into(),
                arguments: json!({"function": info, "description": "d",
                    "payload": {"function_id": "crm::profile"}}),
            }];
            json!({"entry_id": format!("e_t_a_{step}_assistant"),
                "message": AgentMessage::Assistant(message)})
        };
        let result = |id: &str, shown: &Value| {
            json!({"entry_id": format!("e_t_a_{id}"), "message": AgentMessage::FunctionResult(
            FunctionResultMessage {
                role: FunctionResultRoleTag::FunctionResult,
                function_call_id: id.into(),
                function_id: info.into(),
                content: vec![ContentBlock::text(shown.to_string())],
                details: contract.clone(),
                is_error: false,
                timestamp: 1,
            })})
        };
        let mut entries = vec![call(0, "c1"), result("c1", &contract)];
        if repeats {
            entries.push(call(1, "c2"));
            entries.push(result(
                "c2",
                &json!({"function_id": "crm::profile",
                "contract_status": "unchanged_in_context", "source_function_call_id": "c1"}),
            ));
        }
        entries
    }

    fn bundle() -> Value {
        json!({"detail": {"reports": [
            {"scenario_id": "s", "report": {"scenarios": [
                {"scenario_id": "s", "runs": [
                    {"run_id": "r1", "session_id": "e2e_1", "status": "passed",
                     "completion": "completed", "technical": "valid", "wall_time_ms": 1500,
                     "cost": {"total_usd": 0.25},
                     "efficiency": {"total_tokens": 900, "function_calls": 4},
                     "transcript": {"messages": transcript(true)}},
                    {"run_id": "r2", "session_id": "e2e_2", "status": "passed",
                     "completion": "completed", "technical": "valid",
                     "transcript": {"messages": transcript(false)}},
                    {"run_id": "r3", "session_id": "e2e_3", "status": "failed",
                     "completion": "completed", "technical": "infrastructure_error"}]},
                {"scenario_id": "other", "runs": [{"run_id": "x"}]}]}}]}})
    }

    #[test]
    fn runs_come_from_the_report_with_the_detectors_counts_per_run() {
        let patterns = ["repeated_contract_discovery:engine::functions::info".to_string()];
        let runs = runs_of(&bundle(), "s", &patterns);
        assert_eq!(runs.len(), 3, "only the scenario's runs");
        let counted = |at: usize| {
            runs[at]
                .signals
                .as_ref()
                .map(|signals| signals[&patterns[0]])
        };
        assert_eq!(
            (counted(0), counted(1), counted(2)),
            (Some(1), Some(0), None)
        );
        assert_eq!(
            runs.iter().map(|run| run.completed).collect::<Vec<_>>(),
            [true, true, false],
            "a technical failure is identified, not counted"
        );
        assert_eq!(runs[2].technical.as_deref(), Some("infrastructure_error"));
        assert_eq!(
            (
                runs[0].cost_usd,
                runs[0].wall_time_ms,
                runs[0].total_tokens,
                runs[0].function_calls
            ),
            (Some(0.25), Some(1500.0), Some(900.0), Some(4.0))
        );
        assert_eq!(
            (runs[1].cost_usd, runs[1].total_tokens),
            (None, None),
            "an unreported value is never zero"
        );
        assert!(runs_of(&bundle(), "absent", &patterns).is_empty());
        assert!(runs_of(&json!({}), "s", &patterns).is_empty());
    }

    fn analysis(
        version: Option<&str>,
        collected: bool,
        signals: &[(&str, u32)],
    ) -> AnalysisRecordV1 {
        let mut value = json!({
            "schema_version": 1, "evaluation_id": "e", "observation_key": "k",
            "origin": "automatic", "session_id": "s", "turn_id": "t",
            "model": {"model": "m", "provider": "p"},
            "config_revision": "r", "rules_version": "v", "criteria_version": "c",
            "status": "completed", "step": 0, "created_at": 0, "updated_at": 0,
            "deadline": 0, "observe_since": 0,
            "counters": {"sessions": 0, "entries": 0, "diagnostics": 0, "suggestions": 0,
                         "rejected_suggestions": 0, "validations": 0},
            "stages": [], "usage": {"judge_calls": 0, "judge_input_tokens": 0,
                "judge_output_tokens": 0, "judge_usage_complete": true},
            "signals": signals.iter().cloned().collect::<BTreeMap<_, _>>()
        });
        if let Some(version) = version {
            value["harness_version"] = json!(version);
        }
        if collected {
            value["coverage"] = json!("complete");
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn recurrence_splits_the_analyses_at_the_shipped_version() {
        let pattern = "repeated_tool_error:crm::schedule";
        let records = [
            analysis(Some("1.8.41"), true, &[(pattern, 3)]),
            analysis(Some("1.8.42"), true, &[(pattern, 1), ("other:x", 9)]),
            analysis(Some("1.8.43-rc.1"), true, &[(pattern, 2)]),
            analysis(Some("1.8.43"), true, &[]),
            analysis(Some("1.9.0"), true, &[]),
            analysis(Some("1.9.0"), true, &[(pattern, 1)]),
            // No version, or not a semantic one: left out and counted.
            analysis(None, true, &[(pattern, 5)]),
            analysis(Some("nightly"), true, &[]),
            // Collection never finished: nothing was measured.
            analysis(Some("1.9.0"), false, &[]),
        ];
        let (before, from, without) =
            recurrence_windows(&records, &[pattern.to_string()], &version("1.8.43"));
        assert_eq!(without, 2);
        assert_eq!((before.analyses, from.analyses), (3, 3));
        let (earlier, later) = (&before.patterns[0], &from.patterns[0]);
        assert_eq!(
            (
                earlier.occurrences,
                earlier.analyses_with,
                earlier.per_analysis
            ),
            (6, 3, Some(2.0))
        );
        assert_eq!(
            (later.occurrences, later.analyses_with, later.per_analysis),
            (1, 1, Some(1.0 / 3.0))
        );
        // A window with no analysis has no rate.
        let (_, empty, _) =
            recurrence_windows(&records[..2], &[pattern.to_string()], &version("1.8.43"));
        assert_eq!((empty.analyses, empty.patterns[0].per_analysis), (0, None));
    }
}
