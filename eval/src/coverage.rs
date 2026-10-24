//! An objective mapped onto the E2E catalog, in code: the objective names a
//! need, a need names the words a test's spec must contain (criteria,
//! summaries and the catalog's invariants; never the prompt, whose code and
//! paths match anything, nor the title or id) and
//! the metric it is measured by. A test is selected only in its CURRENT
//! definition: the `tests-list` row's `current_version` must be the
//! `scenarios-list` scenario's `behavior_sha256`. Selection is a coverage
//! hypothesis; the baseline and the decision prove it.

use serde_json::Value;

use crate::contract::*;

/// What an objective asks for and what proves a test covers it.
pub struct Need {
    pub id: &'static str,
    /// Objective words that name the need (any).
    triggers: &'static [&'static [&'static str]],
    /// Spec word groups a covering test must contain (one of each group).
    evidence: &'static [&'static [&'static str]],
    pub metric: CriterionMetricV1,
    pub direction: DirectionV1,
    gap: &'static str,
}

/// Checked in order: the first whose every trigger group matches wins.
pub const NEEDS: [Need; 3] = [
    Need {
        id: "provider_fallback",
        triggers: &[&["fallback"], &["provider", "providers", "provedor", "model", "models", "modelo"]],
        evidence: &[&["fallback"], &["provider", "providers", "model", "models"]],
        metric: CriterionMetricV1::PassRate,
        direction: DirectionV1::Increase,
        gap: "no E2E test spec exercises a provider/model fallback (fallback together with a provider or model in its criteria, summary or invariants); tool_contract_recovery covers a changed tool contract, not provider/model fallback",
    },
    Need {
        id: "skills",
        triggers: &[&["skill", "skills"]],
        evidence: &[&["skill", "skills"]],
        metric: CriterionMetricV1::Tokens,
        direction: DirectionV1::Decrease,
        gap: "no E2E test spec measures skill effect, selection or cost (no skill in its criteria, summary or invariants)",
    },
    Need {
        id: "context_consumption",
        triggers: &[&["context", "contexto", "token", "tokens", "memory", "memória", "memoria", "recall"]],
        evidence: &[&["context"], &["distractor", "recall", "memory", "pressure", "load"]],
        metric: CriterionMetricV1::Tokens,
        direction: DirectionV1::Decrease,
        gap: "no E2E test spec puts the context under load (context together with distractor, recall, memory, pressure or load)",
    },
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn matches(words: &[String], groups: &[&[&str]]) -> Option<Vec<String>> {
    groups
        .iter()
        .map(|group| {
            group
                .iter()
                .find(|term| words.iter().any(|word| word == *term))
                .map(|term| term.to_string())
        })
        .collect()
}

/// The need an objective names, if any.
pub fn need_of(objective: &str) -> Option<&'static Need> {
    let objective = words(objective);
    NEEDS
        .iter()
        .find(|need| matches(&objective, need.triggers).is_some())
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// The spec text a test is selected on: criteria descriptions, summaries
/// and the current definition's invariants. Never the prompt: its code
/// snippets and workspace paths would match any need.
fn spec_text(row: &Value, scenario: &Value) -> String {
    let mut parts = vec![text(&row["spec"]["summary"])];
    for criterion in row["spec"]["criteria"].as_array().into_iter().flatten() {
        parts.push(text(&criterion["description"]));
    }
    parts.push(text(&scenario["summary"]));
    for invariant in scenario["deliverable_contract"]["invariants"]
        .as_array()
        .into_iter()
        .flatten()
    {
        parts.push(text(&invariant["description"]));
    }
    parts.join("\n")
}

/// A catalog row in its CURRENT executable definition, with its canonical
/// case and spec criteria; the reason it cannot be used otherwise.
fn current_test(
    row: &Value,
    scenario: &Value,
    matched: Vec<String>,
    rationale: impl FnOnce(&[TestCriterionV1]) -> String,
) -> Result<SelectedTestV1, String> {
    let test_id = text(&row["test_id"]);
    let current = text(&row["current_version"]);
    let behavior = text(&scenario["behavior_sha256"]);
    if current.is_empty() || current != behavior {
        return Err(format!(
            "{test_id}: the catalog's current version {current:?} is not the executable definition {behavior:?}"
        ));
    }
    let (Some(seed), false, false) = (
        scenario["seed"].as_u64(),
        text(&scenario["case_id"]).is_empty(),
        text(&scenario["contract_sha256"]).is_empty(),
    ) else {
        return Err(format!(
            "{test_id}: the catalog lacks its canonical case, seed or contract"
        ));
    };
    let criteria: Vec<TestCriterionV1> = row["spec"]["criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|criterion| {
            Some(TestCriterionV1 {
                id: criterion["id"].as_str()?.into(),
                description: text(&criterion["description"]),
                gate: criterion["gate"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    if criteria.is_empty() {
        return Err(format!(
            "{test_id}: its spec declares no criteria to preserve"
        ));
    }
    let selected_version = text(&row["selected_version"]);
    Ok(SelectedTestV1 {
        scenario_id: test_id,
        case_id: text(&scenario["case_id"]),
        seed,
        behavior_sha256: behavior,
        contract_sha256: text(&scenario["contract_sha256"]),
        inputs_sha256: text(&scenario["inputs_sha256"]),
        invariants: scenario["deliverable_contract"]["invariants"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|invariant| invariant["id"].as_str().map(str::to_string))
            .collect(),
        rationale: rationale(&criteria),
        criteria,
        matched_terms: matched,
        history_note: (!selected_version.is_empty() && selected_version != current).then(|| {
            format!(
                "the row's history is of version {selected_version}, not the current {current}: it is not evidence for the current definition"
            )
        }),
    })
}

fn row_of<'a>(
    rows: &'a [Value],
    catalog: &'a Value,
    test_id: &str,
) -> Option<(&'a Value, &'a Value)> {
    let row = rows.iter().find(|row| row["test_id"] == test_id)?;
    let scenario = catalog["scenarios"]
        .as_array()?
        .iter()
        .find(|scenario| scenario["scenario_id"] == test_id)?;
    Some((row, scenario))
}

fn empty(rows: &[Value], tests_revision: &str, catalog: &Value, now: i64) -> CoverageV1 {
    CoverageV1 {
        need: "unmapped".into(),
        covered: false,
        selected: Vec::new(),
        controls: Vec::new(),
        gaps: Vec::new(),
        tests_total: rows.len() as u32,
        tests_revision: tests_revision.into(),
        catalog_sha256: text(&catalog["catalog_sha256"]),
        selected_at: now,
    }
}

/// The configured controls in their current definition; a control that
/// cannot run as current leaves the coverage uncovered with the reason.
fn add_controls(
    coverage: &mut CoverageV1,
    rows: &[Value],
    catalog: &Value,
    controls: &[String],
    max_scenarios: usize,
) {
    for control in controls {
        if coverage
            .selected
            .first()
            .is_some_and(|test| &test.scenario_id == control)
            || coverage
                .controls
                .iter()
                .any(|test| &test.scenario_id == control)
        {
            continue;
        }
        coverage
            .selected
            .retain(|test| &test.scenario_id != control);
        let found = row_of(rows, catalog, control)
            .ok_or_else(|| format!("control {control}: not an active test of the catalog"))
            .and_then(|(row, scenario)| {
                current_test(row, scenario, Vec::new(), |criteria| {
                    format!(
                        "control: its pass rate and every spec criterion ({}) must hold",
                        criteria
                            .iter()
                            .map(|criterion| criterion.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
            });
        match found {
            Ok(test) => coverage.controls.push(test),
            Err(reason) => {
                coverage.covered = false;
                coverage.gaps.push(reason);
            }
        }
    }
    coverage
        .selected
        .truncate(max_scenarios.saturating_sub(coverage.controls.len()));
    if coverage.selected.is_empty() {
        coverage.covered = false;
        coverage
            .gaps
            .push("configured controls leave no primary scenario within max_scenarios".into());
    }
}

/// Selects the tests covering `objective` from every `tests-list` row and
/// the `scenarios-list` catalog, plus the configured controls. At most
/// `max_scenarios` are kept, those with the most current runs first.
pub fn select(
    objective: &str,
    rows: &[Value],
    tests_revision: &str,
    catalog: &Value,
    max_scenarios: usize,
    controls: &[String],
    now: i64,
) -> CoverageV1 {
    let mut coverage = empty(rows, tests_revision, catalog, now);
    let Some(need) = need_of(objective) else {
        coverage.gaps.push(format!(
            "the objective names no need this policy measures ({}); no test is selected by a similar title",
            NEEDS.iter().map(|need| need.id).collect::<Vec<_>>().join(", ")
        ));
        return coverage;
    };
    coverage.need = need.id.into();
    let mut candidates: Vec<(u64, SelectedTestV1)> = Vec::new();
    let mut stale = Vec::new();
    for row in rows {
        let Some((row, scenario)) = row_of(rows, catalog, &text(&row["test_id"])) else {
            continue;
        };
        let Some(matched) = matches(&words(&spec_text(row, scenario)), need.evidence) else {
            continue;
        };
        let terms = matched.join(" + ");
        match current_test(row, scenario, matched, |criteria| {
            format!(
                "its spec puts {terms} in play; measured by {:?} per run while every spec criterion ({}) is preserved",
                need.metric,
                criteria.iter().map(|criterion| criterion.id.as_str()).collect::<Vec<_>>().join(", ")
            )
        }) {
            Ok(test) => candidates.push((row["runs_current"].as_u64().unwrap_or(0), test)),
            Err(reason) => stale.push(format!("{reason}; not selected")),
        }
    }
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.scenario_id.cmp(&right.1.scenario_id))
    });
    let kept = candidates.len().min(max_scenarios.max(1));
    for (_, test) in candidates.iter().skip(kept) {
        coverage.gaps.push(format!(
            "{} also covers the need; left out by max_scenarios",
            test.scenario_id
        ));
    }
    coverage.selected = candidates
        .into_iter()
        .take(kept)
        .map(|(_, test)| test)
        .collect();
    coverage.covered = !coverage.selected.is_empty();
    if !coverage.covered {
        coverage.gaps.push(need.gap.into());
    }
    coverage.gaps.extend(stale);
    if coverage.covered {
        add_controls(&mut coverage, rows, catalog, controls, max_scenarios);
    }
    coverage
}

/// What a source execution's own runs show was lost, as a criterion to
/// restore: the criterion with the lowest mean award, else the pass rate
/// when a run did not complete and pass. `None`: nothing measurable.
pub(crate) fn lost(
    bundle: &Value,
    scenario_id: &str,
) -> Option<(CriterionMetricV1, Option<String>, String)> {
    let runs = crate::evidence::runs_of(bundle, scenario_id, &[]);
    let mut awards: std::collections::BTreeMap<&str, Vec<f64>> = Default::default();
    for run in runs.iter().filter(|run| run.measurable()) {
        for (id, award) in &run.criteria {
            awards.entry(id).or_default().push(*award);
        }
    }
    let worst = awards
        .iter()
        .map(|(id, values)| (*id, values.iter().sum::<f64>() / values.len() as f64))
        .filter(|(_, mean)| *mean < 1.0)
        .min_by(|left, right| left.1.total_cmp(&right.1));
    if let Some((id, mean)) = worst {
        return Some((
            CriterionMetricV1::CriterionScore,
            Some(id.to_string()),
            format!(
                "criterion {id} of {scenario_id} was awarded {:.0}% on average",
                mean * 100.0
            ),
        ));
    }
    runs.iter()
        .any(|run| run.measurable() && !(run.completed && run.status.as_deref() == Some("passed")))
        .then(|| {
            (
                CriterionMetricV1::PassRate,
                None,
                format!("a run of {scenario_id} did not complete and pass"),
            )
        })
}

/// The goal and coverage of a source execution with pertinent findings: its
/// executed scenarios, each still the catalog's current definition, and the
/// criterion its own runs lost. Uncovered: the exact reason, nothing spent.
pub fn derive(
    source: &ExecutionEvidenceV1,
    rows: &[Value],
    tests_revision: &str,
    catalog: &Value,
    max_scenarios: usize,
    controls: &[String],
    now: i64,
) -> (
    CoverageV1,
    Option<(String, CriterionMetricV1, Option<String>)>,
) {
    let mut coverage = empty(rows, tests_revision, catalog, now);
    coverage.need = "source_execution".into();
    let mut goal = None;
    for executed in &source.identity.scenarios {
        let id = &executed.scenario_id;
        let Some((row, scenario)) = row_of(rows, catalog, id) else {
            coverage
                .gaps
                .push(format!("{id}: not an active test of the catalog"));
            continue;
        };
        if executed.behavior_sha256.as_deref() != Some(text(&scenario["behavior_sha256"]).as_str())
        {
            coverage.gaps.push(format!(
                "{id}: execution {} ran behavior {:?}, not the current {:?}",
                source.execution_id,
                executed.behavior_sha256,
                text(&scenario["behavior_sha256"])
            ));
            continue;
        }
        let Some((metric, criterion_id, why)) = lost(&source.bundle, id) else {
            coverage
                .gaps
                .push(format!("{id}: its runs lost no measurable criterion"));
            continue;
        };
        match current_test(row, scenario, Vec::new(), |_| why.clone()) {
            Ok(test) if coverage.selected.len() < max_scenarios.max(1) => {
                if goal.is_none() {
                    goal = Some((
                        format!("Restore what execution {} lost: {why}", source.execution_id),
                        metric,
                        criterion_id,
                    ));
                }
                coverage.selected.push(test);
            }
            Ok(test) => coverage
                .gaps
                .push(format!("{}: left out by max_scenarios", test.scenario_id)),
            Err(reason) => coverage.gaps.push(reason),
        }
    }
    coverage.covered = goal.is_some();
    if coverage.covered {
        add_controls(&mut coverage, rows, catalog, controls, max_scenarios);
    }
    if !coverage.covered && coverage.gaps.is_empty() {
        coverage.gaps.push(format!(
            "execution {} reported no scenario",
            source.execution_id
        ));
    }
    let goal = if coverage.covered { goal } else { None };
    (coverage, goal)
}

/// The plan frozen from a coverage, before any result: `metric` on the
/// first selected scenario, and the pass rate and every spec criterion of
/// every selected scenario and control as preserved correctness (any loss
/// rejects). Every selected test and control is frozen in `tests` with its
/// case, seed, behavior, inputs and contract digests. With a validation
/// config the baseline runs the candidate's base commit on the pinned
/// stack, so the only intended difference of a later candidate is its
/// Harness source.
pub fn plan(
    objective: &str,
    coverage: &CoverageV1,
    goal: (CriterionMetricV1, DirectionV1, Option<String>),
    stored: &StoredConfigV1,
    now: i64,
) -> Option<CyclePlanV1> {
    let first = coverage.selected.first()?;
    let config = &stored.config;
    let runs = config.runs_per_side;
    let (metric, direction, criterion_id) = goal;
    let min_effect = match metric {
        CriterionMetricV1::PassRate | CriterionMetricV1::CriterionScore => {
            config.min_effect.pass_rate_pp
        }
        _ => config.min_effect.tokens,
    };
    let criterion =
        |metric, criterion_id: Option<String>, scenario_id: &str, direction, min_effect| {
            CriterionV1 {
                metric,
                pattern: None,
                criterion_id,
                direction,
                min_effect,
                min_runs: runs,
                scenario_id: scenario_id.into(),
            }
        };
    let tests: Vec<&SelectedTestV1> = coverage.selected.iter().chain(&coverage.controls).collect();
    let mut preserved = Vec::new();
    for test in &tests {
        let primary = test.scenario_id == first.scenario_id;
        if !(primary && metric == CriterionMetricV1::PassRate) {
            preserved.push(criterion(
                CriterionMetricV1::PassRate,
                None,
                &test.scenario_id,
                DirectionV1::Increase,
                0.0,
            ));
        }
        for spec in &test.criteria {
            if primary && criterion_id.as_deref() == Some(spec.id.as_str()) {
                continue;
            }
            preserved.push(criterion(
                CriterionMetricV1::CriterionScore,
                Some(spec.id.clone()),
                &test.scenario_id,
                DirectionV1::Increase,
                0.0,
            ));
        }
    }
    let scenarios: Vec<String> = tests.iter().map(|test| test.scenario_id.clone()).collect();
    let (where_, stack) = match (&config.validation, &config.candidate) {
        (Some(validation), Some(candidate)) => (
            validation.where_,
            Some(validation.source_stack(&candidate.base_sha)),
        ),
        _ => (config.where_, config.stack.clone()),
    };
    Some(CyclePlanV1 {
        objective: objective.into(),
        criterion: criterion(
            metric,
            criterion_id,
            &first.scenario_id,
            direction,
            min_effect,
        ),
        preserved,
        controls: coverage
            .controls
            .iter()
            .map(|test| test.scenario_id.clone())
            .collect(),
        // The executable identity every side must run, frozen with the plan:
        // a catalog that moves after selection never moves the plan.
        tests: tests.iter().map(|test| (*test).clone()).collect(),
        sampling: SamplingPlanV1 {
            runs_per_side: runs,
            seeds: tests.iter().map(|test| test.case_id.clone()).collect(),
        },
        // Every technical retry may rerun a run: the bound counts them.
        estimated_usd: config.estimated_usd_per_run
            * f64::from(runs)
            * scenarios.len() as f64
            * f64::from(1 + u32::from(config.technical_retries)),
        parameters: ExecutionParametersV1 {
            provider: config.subject.provider.clone(),
            model: config.subject.model.clone(),
            runs,
            scenarios,
            technical_retries: config.technical_retries,
            where_,
            stack,
            agent: config.subject.agent.clone(),
        },
        policy_revision: stored.revision.clone(),
        frozen_at: now,
    })
}

/// The goal of an objective's need.
pub fn need_goal(
    coverage: &CoverageV1,
) -> Option<(CriterionMetricV1, DirectionV1, Option<String>)> {
    let need = NEEDS.iter().find(|need| need.id == coverage.need)?;
    Some((need.metric, need.direction, None))
}
