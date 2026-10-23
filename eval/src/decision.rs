//! The loop's decision on a baseline/candidate pair, computed in code, never
//! by a model: the comparison of the means, the minimum effect and n, behind
//! the gates that make it mean something. The plan is frozen before any
//! result; an incompatible pair, a technical failure or a missing measure is
//! inconclusive; a gain that loses a preserved correctness condition is
//! rejected.

use crate::contract::*;
use crate::error::EvalError;
use crate::runtime;

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

pub fn compare(
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
// The frozen plan
// ---------------------------------------------------------------------------

/// Validates `plan` and freezes it in `record`, before any result. Freezing
/// the same plan again is a no-op; a different plan, or any plan once a
/// decision is recorded, is refused.
pub fn freeze(record: &mut CycleRecordV1, plan: CyclePlanV1) -> Result<(), EvalError> {
    validate(&plan)?;
    match &record.plan {
        Some(frozen) if same_plan(frozen, &plan) => return Ok(()),
        Some(_) => {
            return Err(EvalError::Conflict(format!(
                "cycle {} has a frozen plan; it cannot change",
                record.cycle_id
            )))
        }
        None => {}
    }
    if record.decision.is_some() {
        return Err(EvalError::Conflict(format!(
            "cycle {} already has a decision; a plan is frozen before any result",
            record.cycle_id
        )));
    }
    record.plan = Some(plan);
    Ok(())
}

/// The same plan, whenever it was frozen.
fn same_plan(frozen: &CyclePlanV1, plan: &CyclePlanV1) -> bool {
    let again = CyclePlanV1 {
        frozen_at: frozen.frozen_at,
        ..plan.clone()
    };
    again == *frozen
}

fn validate(plan: &CyclePlanV1) -> Result<(), EvalError> {
    let invalid = |reason: &str| Err(EvalError::InvalidRequest(format!("cycle plan: {reason}")));
    if plan.objective.trim().is_empty() {
        return invalid("the objective is empty");
    }
    if plan.policy_revision.trim().is_empty() {
        return invalid("the policy_revision is empty");
    }
    if plan.sampling.runs_per_side == 0 {
        return invalid("sampling.runs_per_side must be at least 1");
    }
    for criterion in std::iter::once(&plan.criterion).chain(&plan.preserved) {
        if !criterion.min_effect.is_finite() || criterion.min_effect < 0.0 {
            return invalid("a min_effect must be a finite number, 0 or more");
        }
        if criterion.metric == CriterionMetricV1::SignalPerRun && criterion.pattern.is_none() {
            return invalid("a signal_per_run criterion needs its pattern");
        }
    }
    Ok(())
}

/// The guard every write of a cycle record passes: no decision without a
/// frozen plan, a frozen plan never changes or disappears, and a recorded
/// decision is never replaced.
pub fn check_write(stored: Option<&CycleRecordV1>, next: &CycleRecordV1) -> Result<(), EvalError> {
    if next.decision.is_some() && next.plan.is_none() {
        return Err(EvalError::Conflict(format!(
            "cycle {} has no frozen plan, so it cannot record a decision",
            next.cycle_id
        )));
    }
    let Some(stored) = stored else {
        return Ok(());
    };
    if stored.plan.is_some() && next.plan != stored.plan {
        return Err(EvalError::Conflict(format!(
            "the frozen plan of cycle {} cannot change",
            next.cycle_id
        )));
    }
    if stored.decision.is_some() && next.decision != stored.decision {
        return Err(EvalError::Conflict(format!(
            "the decision of cycle {} is recorded and cannot change",
            next.cycle_id
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The gated decision
// ---------------------------------------------------------------------------

fn sorted(values: &[String]) -> Option<String> {
    let mut values = values.to_vec();
    values.sort();
    (!values.is_empty()).then(|| values.join(", "))
}

/// Everything that must match between the two sides: the E2E's own
/// comparability (scenario, behavior, contract, model, provider, E2E
/// revision, engine) plus the inputs/seeds and the policy, both of which must
/// also be the frozen plan's. Unknown values never match.
pub fn compatibility(
    plan: &CyclePlanV1,
    baseline: &DecisionSideV1,
    candidate: &DecisionSideV1,
) -> ComparabilityV1 {
    let mut comparability = runtime::comparability(&baseline.execution, &candidate.execution);
    // The E2E's comparability reads a scenario's unknown behavior or
    // contract as an empty value, which matches another unknown one here.
    let unknown = |missing: fn(&E2eScenarioV1) -> bool| {
        [&baseline.execution, &candidate.execution]
            .iter()
            .any(|execution| execution.scenarios.iter().any(missing))
    };
    for check in &mut comparability.checks {
        let missing = match check.field.as_str() {
            "behavior_sha256" => unknown(|scenario| scenario.behavior_sha256.is_none()),
            "contract_fingerprint" => unknown(|scenario| scenario.contract_fingerprint.is_none()),
            _ => false,
        };
        if missing {
            check.matches = false;
        }
    }
    let extra = [
        (
            "seeds",
            sorted(&baseline.seeds),
            sorted(&candidate.seeds),
            sorted(&plan.sampling.seeds),
        ),
        (
            "policy_revision",
            baseline.policy_revision.clone(),
            candidate.policy_revision.clone(),
            Some(plan.policy_revision.clone()),
        ),
    ];
    for (field, baseline, candidate, planned) in extra {
        let matches = baseline.is_some()
            && baseline == candidate
            && (planned.is_none() || baseline == planned);
        comparability.checks.push(ComparabilityCheckV1 {
            field: field.into(),
            baseline,
            candidate,
            matches,
        });
    }
    comparability.comparable = comparability.checks.iter().all(|check| check.matches);
    comparability
}

/// The decision on a pair by the frozen `plan`. `evidence` holds the runs of
/// each scenario (see [`crate::review::evidence`]); evidence of another
/// execution than the pair's is never mixed in. Each criterion is judged
/// with at least the plan's `runs_per_side`. Gates, in order: an incompatible
/// pair is inconclusive; a lost preserved correctness condition is rejected,
/// whatever the metric did; an unmeasured one is inconclusive; then the
/// metric's outcome decides (an improvement of the minimum effect is
/// accepted, no improvement or a regression rejected, missing data or a
/// technical failure inconclusive).
pub fn decide(
    plan: &CyclePlanV1,
    baseline: &DecisionSideV1,
    candidate: &DecisionSideV1,
    evidence: &[EvidenceV1],
    now: i64,
) -> CycleDecisionV1 {
    let comparability = compatibility(plan, baseline, candidate);
    let (baseline_id, candidate_id) = (
        baseline.execution.execution_id.as_str(),
        candidate.execution.execution_id.as_str(),
    );
    let empty = |execution_id: &str| EvidenceSideV1 {
        execution_id: execution_id.into(),
        runs: Vec::new(),
        n: 0,
        mean: None,
    };
    let judged = |criterion: &CriterionV1| -> EvidenceV1 {
        let criterion = CriterionV1 {
            min_runs: criterion.min_runs.max(plan.sampling.runs_per_side),
            ..criterion.clone()
        };
        let mut judged = evidence
            .iter()
            .find(|found| {
                found.scenario_id == criterion.scenario_id
                    && found.baseline.execution_id == baseline_id
                    && found.candidate.execution_id == candidate_id
            })
            .cloned()
            .unwrap_or_else(|| EvidenceV1 {
                scenario_id: criterion.scenario_id.clone(),
                computed_at: now,
                baseline: empty(baseline_id),
                candidate: empty(candidate_id),
                computed_outcome: ValidationOutcomeV1::Inconclusive,
                reason: String::new(),
            });
        judged.computed_at = now;
        judge(&mut judged, Some(&criterion));
        judged
    };
    let primary = judged(&plan.criterion);
    let preserved: Vec<EvidenceV1> = plan.preserved.iter().map(judged).collect();
    let (outcome, reason) = verdict(&comparability, &primary, &preserved);
    CycleDecisionV1 {
        outcome,
        reason,
        policy_revision: plan.policy_revision.clone(),
        comparability,
        primary,
        preserved,
        decided_at: now,
    }
}

fn verdict(
    comparability: &ComparabilityV1,
    primary: &EvidenceV1,
    preserved: &[EvidenceV1],
) -> (DecisionOutcomeV1, String) {
    use DecisionOutcomeV1::*;
    if !comparability.comparable {
        let differing: Vec<&str> = comparability
            .checks
            .iter()
            .filter(|check| !check.matches)
            .map(|check| check.field.as_str())
            .collect();
        return (
            Inconclusive,
            format!(
                "the pair is not comparable: {} differ or are unknown",
                differing.join(", ")
            ),
        );
    }
    let lost = |wanted: ValidationOutcomeV1| {
        preserved
            .iter()
            .find(move |condition| condition.computed_outcome == wanted)
    };
    if let Some(condition) = lost(ValidationOutcomeV1::Regression) {
        return (
            Rejected,
            format!(
                "a preserved correctness condition was lost, whatever the metric did: {}",
                condition.reason
            ),
        );
    }
    if let Some(condition) = lost(ValidationOutcomeV1::Inconclusive) {
        return (
            Inconclusive,
            format!(
                "a preserved correctness condition was not measured: {}",
                condition.reason
            ),
        );
    }
    match primary.computed_outcome {
        ValidationOutcomeV1::ValidatedImprovement => (Accepted, primary.reason.clone()),
        ValidationOutcomeV1::Regression => (Rejected, format!("regression: {}", primary.reason)),
        ValidationOutcomeV1::NoImprovement => (
            Rejected,
            format!("no improvement of the minimum effect: {}", primary.reason),
        ),
        ValidationOutcomeV1::Inconclusive => (Inconclusive, primary.reason.clone()),
    }
}

/// Decides the cycle's pair once, by its frozen plan, and stores the decision
/// in the record. A cycle already decided keeps its decision: a repeated
/// event never starts another comparison. Without a frozen plan nothing is
/// decided.
pub fn decide_cycle(
    record: &mut CycleRecordV1,
    baseline: &DecisionSideV1,
    candidate: &DecisionSideV1,
    evidence: &[EvidenceV1],
    now: i64,
) -> Result<CycleDecisionV1, EvalError> {
    let Some(plan) = &record.plan else {
        return Err(EvalError::InvalidRequest(format!(
            "cycle {} has no frozen plan; freeze it before recording results",
            record.cycle_id
        )));
    };
    if let Some(decided) = &record.decision {
        return Ok(decided.clone());
    }
    let decision = decide(plan, baseline, candidate, evidence, now);
    record.decision = Some(decision.clone());
    record.updated_at = now;
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

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
}
