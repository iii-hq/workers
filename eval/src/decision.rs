//! The loop's decision on a baseline/candidate pair, computed in code, never
//! by a model: the comparison of the means, the minimum effect and n, behind
//! the gates that make it mean something. The plan is frozen before any
//! result, with the executable identity of every test it runs; each side
//! must have run exactly that coverage, on the same subject artifact and the
//! same non-Harness stack. An incompatible pair, a technical failure, a
//! missing measure or a sample other than the frozen one is inconclusive; a
//! gain that loses a preserved correctness condition is rejected. A mean
//! improvement is accepted only with a positive frozen minimum effect and a
//! one-sided exact sign test over the same-case run pairs (p <= 0.05): five
//! consistent pairs can pass, one lucky pair cannot. The sign test is
//! evidence of a consistent direction, not a confidence interval for the
//! mean.

use crate::contract::*;
use crate::cost;
use crate::error::EvalError;

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
            _ => Some(0.0),
        },
        CriterionMetricV1::CriterionScore => run
            .criteria
            .get(criterion.criterion_id.as_deref()?)
            .copied(),
        // Effort only counts for a task done: fewer tokens from an unfinished
        // or failed run are no gain.
        _ if !run.completed || run.status.as_deref() != Some("passed") => None,
        CriterionMetricV1::CostUsd => run.cost_usd,
        CriterionMetricV1::Duration => run.wall_time_ms,
        CriterionMetricV1::Tokens => run.total_tokens,
        CriterionMetricV1::FunctionCalls => run.function_calls,
        CriterionMetricV1::FunctionCallErrors => run.function_call_errors,
    }
}

fn metric_label(criterion: &CriterionV1) -> String {
    match (criterion.metric, &criterion.pattern) {
        (CriterionMetricV1::SignalPerRun, Some(pattern)) => format!("signal per run {pattern}"),
        (CriterionMetricV1::SignalPerRun, None) => "signal per run".into(),
        (CriterionMetricV1::PassRate, _) => "pass rate".into(),
        (CriterionMetricV1::CriterionScore, _) => format!(
            "criterion {}",
            criterion.criterion_id.as_deref().unwrap_or_default()
        ),
        (CriterionMetricV1::CostUsd, _) => "cost (USD)".into(),
        (CriterionMetricV1::Duration, _) => "wall time (ms)".into(),
        (CriterionMetricV1::Tokens, _) => "tokens".into(),
        (CriterionMetricV1::FunctionCalls, _) => "function calls".into(),
        (CriterionMetricV1::FunctionCallErrors, _) => "function call errors".into(),
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
        CriterionMetricV1::PassRate | CriterionMetricV1::CriterionScore => ("pp", 0.01),
        CriterionMetricV1::CostUsd => ("USD", 1.0),
        CriterionMetricV1::Duration => ("s", 1_000.0),
        CriterionMetricV1::Tokens => ("tokens", 1.0),
        CriterionMetricV1::FunctionCalls => ("calls", 1.0),
        CriterionMetricV1::FunctionCallErrors => ("errors", 1.0),
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
/// of its runs failed technically, or too few runs carry the metric. Otherwise the difference of the means, in the
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
        // Only what the infrastructure broke measured nothing; a run the agent
        // left unfinished is a result (a fail), never a reason to stop judging.
        let unmeasured = side.runs.iter().filter(|run| !run.measurable()).count();
        if total == 0 {
            return inconclusive(format!(
                "the {name} execution has no run of {}",
                evidence.scenario_id
            ));
        }
        if unmeasured * 2 > total {
            return inconclusive(format!(
                "{unmeasured} of {total} {name} runs failed technically, more than half"
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
    // With a zero minimum effect an unchanged mean is held (no improvement),
    // never a gain; any loss is a regression.
    let outcome = if criterion.min_effect <= 0.0 {
        if wanted > EFFECT_TOLERANCE {
            ValidationOutcomeV1::ValidatedImprovement
        } else if wanted < -EFFECT_TOLERANCE {
            ValidationOutcomeV1::Regression
        } else {
            ValidationOutcomeV1::NoImprovement
        }
    } else if wanted + EFFECT_TOLERANCE >= criterion.min_effect {
        ValidationOutcomeV1::ValidatedImprovement
    } else if wanted - EFFECT_TOLERANCE <= -criterion.min_effect {
        ValidationOutcomeV1::Regression
    } else {
        ValidationOutcomeV1::NoImprovement
    };
    let mut reason = format!(
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
    // A mean gain of the minimum effect is not enough on its own: the paired
    // runs must show it in a consistent direction (see [`directional`]).
    if outcome == ValidationOutcomeV1::ValidatedImprovement && criterion.min_effect > 0.0 {
        match directional(evidence, criterion) {
            Ok(test) if test.p <= SIGN_TEST_ALPHA + EFFECT_TOLERANCE => {
                reason = format!("{reason}; {}", test.describe(criterion));
            }
            Ok(test) => {
                return inconclusive(format!(
                    "{reason}; but {}, above {SIGN_TEST_ALPHA}: the mean effect is not shown in a \
                     consistent direction",
                    test.describe(criterion)
                ));
            }
            Err(why) => {
                return inconclusive(format!(
                    "{reason}; but the runs cannot be paired for the directional sign test: {why}"
                ));
            }
        }
    }
    (outcome, reason)
}

/// The frozen directional rule (part of the rules `RULES_VERSION` names): a
/// positive improvement with a minimum effect above 0 must also pass a
/// one-sided exact sign test at this level.
pub const SIGN_TEST_ALPHA: f64 = 0.05;
/// Its identifier, written into every reason that applies it.
pub const SIGN_TEST_RULE: &str = "paired-sign-test/one-sided/alpha-0.05";

/// The directional sign test on one criterion's paired runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignTest {
    /// Pairs whose candidate run beat its baseline run by the minimum effect.
    pub reached: u32,
    /// Every planned pair, an unmeasured one counted as not reaching it.
    pub pairs: u32,
    /// P(X >= reached) for X ~ Binomial(pairs, 1/2).
    pub p: f64,
}

impl SignTest {
    fn describe(&self, criterion: &CriterionV1) -> String {
        let (unit, _) = effect_unit(criterion.metric);
        format!(
            "{SIGN_TEST_RULE}: {} of {} same-case run pairs (native run order) reached the {} {unit} \
             effect, one-sided binomial p={:.5}; evidence of a consistent direction, not a \
             confidence interval for the mean",
            self.reached,
            self.pairs,
            number(criterion.min_effect),
            self.p
        )
    }
}

/// The one-sided exact binomial tail P(X >= k), X ~ Binomial(n, 1/2).
pub fn sign_test_p(reached: u32, pairs: u32) -> f64 {
    if reached == 0 {
        return 1.0;
    }
    if reached > pairs {
        return 0.0;
    }
    if reached <= pairs / 2 {
        return 1.0 - sign_test_p(pairs - reached + 1, pairs);
    }
    // Start at the requested upper-tail term in log space: 2^-n alone
    // underflows for large samples, even when their tail is near one.
    let n = f64::from(pairs);
    let log_term = (0..pairs - reached).fold(-n * std::f64::consts::LN_2, |sum, i| {
        sum + (n - f64::from(i)).ln() - f64::from(i + 1).ln()
    });
    let mut term = log_term.exp();
    let mut tail = term;
    for i in reached..pairs {
        term *= (n - f64::from(i)) / f64::from(i + 1);
        tail += term;
    }
    tail.min(1.0)
}

/// Pairs the i-th baseline run with the i-th candidate run, in the order the
/// E2E reported them, only when both sides ran exactly the same cases the
/// same number of times (every run names its case, the i-th runs share it,
/// no run id repeats). A pair reaches the effect when both runs are
/// measured and the candidate beats the baseline by `min_effect` in the
/// wanted direction; an unmeasured run (technical failure, an unfinished or
/// failed task for an effort metric, an unknown value) never reaches it.
/// Ambiguous pairing is an error, never a guess.
pub fn directional(evidence: &EvidenceV1, criterion: &CriterionV1) -> Result<SignTest, String> {
    let (baseline, candidate) = (&evidence.baseline.runs, &evidence.candidate.runs);
    if baseline.len() != candidate.len() {
        return Err(format!(
            "the baseline has {} run(s) and the candidate {}",
            baseline.len(),
            candidate.len()
        ));
    }
    for (name, runs) in [("baseline", baseline), ("candidate", candidate)] {
        let mut ids = std::collections::BTreeSet::new();
        if let Some(run) = runs.iter().find(|run| !ids.insert(run.run_id.as_str())) {
            return Err(format!("the {name} reports run {} twice", run.run_id));
        }
    }
    let (_, scale) = effect_unit(criterion.metric);
    let mut reached = 0;
    for (at, (left, right)) in baseline.iter().zip(candidate).enumerate() {
        match (&left.case_id, &right.case_id) {
            (Some(left_case), Some(right_case)) if left_case == right_case => {}
            (left_case, right_case) => {
                return Err(format!(
                    "run pair {at} is not of one case (baseline {left_case:?}, candidate {right_case:?})"
                ))
            }
        }
        let value = |run: &EvidenceRunV1| {
            run.measurable()
                .then(|| metric_value(run, criterion))
                .flatten()
        };
        if let (Some(before), Some(after)) = (value(left), value(right)) {
            let change = (after - before) / scale;
            let wanted = match criterion.direction {
                DirectionV1::Decrease => -change,
                DirectionV1::Increase => change,
            };
            if wanted + EFFECT_TOLERANCE >= criterion.min_effect {
                reached += 1;
            }
        }
    }
    let pairs = baseline.len() as u32;
    if pairs == 0 {
        return Err("no run pair".into());
    }
    Ok(SignTest {
        reached,
        pairs,
        p: sign_test_p(reached, pairs),
    })
}

/// The frozen sampling plan, held exactly: each side reports exactly
/// `planned` runs of the scenario, every run id once. An extra, missing or
/// duplicated sample is never silently accepted. `None`: it holds.
pub fn sampling_gap(evidence: &EvidenceV1, planned: u32) -> Option<String> {
    for (name, side) in [
        ("baseline", &evidence.baseline),
        ("candidate", &evidence.candidate),
    ] {
        if side.runs.len() != planned as usize {
            return Some(format!(
                "the {name} reports {} run(s) of {}, the frozen plan samples exactly {planned}",
                side.runs.len(),
                evidence.scenario_id
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        if let Some(run) = side
            .runs
            .iter()
            .find(|run| !ids.insert(run.run_id.as_str()))
        {
            return Some(format!(
                "the {name} reports run {} of {} more than once",
                run.run_id, evidence.scenario_id
            ));
        }
    }
    None
}

/// The criteria a plan judges: its metric, then every preserved condition.
fn criteria(plan: &CyclePlanV1) -> impl Iterator<Item = &CriterionV1> {
    std::iter::once(&plan.criterion).chain(&plan.preserved)
}

fn patterns(plan: &CyclePlanV1) -> Vec<String> {
    criteria(plan)
        .filter_map(|criterion| criterion.pattern.clone())
        .collect()
}

/// One side's runs of `criterion`'s scenario with its `n` and `mean`.
pub fn measure(
    plan: &CyclePlanV1,
    criterion: &CriterionV1,
    execution_id: &str,
    bundle: &serde_json::Value,
) -> EvidenceSideV1 {
    let mut evidence = EvidenceV1 {
        scenario_id: criterion.scenario_id.clone(),
        computed_at: 0,
        baseline: EvidenceSideV1 {
            execution_id: execution_id.into(),
            runs: crate::evidence::runs_of(bundle, &criterion.scenario_id, &patterns(plan)),
            n: 0,
            mean: None,
        },
        candidate: EvidenceSideV1 {
            execution_id: String::new(),
            runs: Vec::new(),
            n: 0,
            mean: None,
        },
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    };
    judge(&mut evidence, Some(criterion));
    evidence.baseline
}

/// The baseline measured against the frozen plan: usable only when every
/// criterion has its minimum of measured runs with a value.
pub fn baseline(plan: &CyclePlanV1, execution_id: &str, bundle: &serde_json::Value) -> BaselineV1 {
    let primary = measure(plan, &plan.criterion, execution_id, bundle);
    let preserved: Vec<EvidenceSideV1> = plan
        .preserved
        .iter()
        .map(|criterion| measure(plan, criterion, execution_id, bundle))
        .collect();
    let short: Vec<String> = criteria(plan)
        .zip(std::iter::once(&primary).chain(&preserved))
        .filter(|(criterion, side)| {
            side.mean.is_none()
                || side.n < criterion.min_runs.max(plan.sampling.runs_per_side)
                || side.runs.len() != plan.sampling.runs_per_side as usize
        })
        .map(|(criterion, side)| {
            format!(
                "{} (n={}, {} of exactly {} planned runs)",
                metric_label(criterion),
                side.n,
                side.runs.len(),
                plan.sampling.runs_per_side
            )
        })
        .collect();
    BaselineV1 {
        execution_id: execution_id.into(),
        usable: short.is_empty(),
        reason: if short.is_empty() {
            "every planned criterion is measured on the baseline".into()
        } else {
            format!("not measured on enough valid runs: {}", short.join(", "))
        },
        primary,
        preserved,
    }
}

/// The pair's evidence for every scenario the plan judges, read from the two
/// stored bundles only.
pub fn pair_evidence(
    plan: &CyclePlanV1,
    baseline: (&str, &serde_json::Value),
    candidate: (&str, &serde_json::Value),
    now: i64,
) -> Vec<EvidenceV1> {
    let mut scenarios: Vec<&str> = criteria(plan)
        .map(|criterion| criterion.scenario_id.as_str())
        .collect();
    scenarios.sort();
    scenarios.dedup();
    let patterns = patterns(plan);
    scenarios
        .into_iter()
        .map(|scenario_id| {
            let side = |(execution_id, bundle): (&str, &serde_json::Value)| EvidenceSideV1 {
                execution_id: execution_id.into(),
                runs: crate::evidence::runs_of(bundle, scenario_id, &patterns),
                n: 0,
                mean: None,
            };
            EvidenceV1 {
                scenario_id: scenario_id.into(),
                computed_at: now,
                baseline: side(baseline),
                candidate: side(candidate),
                computed_outcome: ValidationOutcomeV1::Inconclusive,
                reason: String::new(),
            }
        })
        .collect()
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
    // Every judged scenario is frozen with its executable identity, once.
    let mut frozen = std::collections::BTreeSet::new();
    for test in &plan.tests {
        if !frozen.insert(test.scenario_id.as_str()) {
            return invalid("a scenario is frozen twice in tests");
        }
    }
    if std::iter::once(&plan.criterion)
        .chain(&plan.preserved)
        .any(|criterion| !frozen.contains(criterion.scenario_id.as_str()))
    {
        return invalid(
            "every judged scenario needs its frozen test (case, seed, behavior, inputs, contract)",
        );
    }
    for criterion in std::iter::once(&plan.criterion).chain(&plan.preserved) {
        if !criterion.min_effect.is_finite() || criterion.min_effect < 0.0 {
            return invalid("a min_effect must be a finite number, 0 or more");
        }
        if criterion.metric == CriterionMetricV1::SignalPerRun && criterion.pattern.is_none() {
            return invalid("a signal_per_run criterion needs its pattern");
        }
        if criterion.metric == CriterionMetricV1::CriterionScore && criterion.criterion_id.is_none()
        {
            return invalid("a criterion_score criterion needs its criterion_id");
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

fn scenarios(
    identity: &EvidenceIdentityV1,
    field: fn(&ScenarioIdentityV1) -> Option<String>,
) -> Option<String> {
    let values: Option<Vec<String>> = identity
        .scenarios
        .iter()
        .map(|scenario| field(scenario).map(|value| format!("{}={value}", scenario.scenario_id)))
        .collect();
    values.and_then(|values| sorted(&values))
}

fn coverage_line(
    scenario_id: &str,
    case_id: Option<&str>,
    seed: Option<u64>,
    behavior: Option<&str>,
    inputs: Option<&str>,
    contract: Option<&str>,
) -> Option<String> {
    let known = |value: Option<&str>| {
        value
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    Some(format!(
        "{scenario_id}: case={} seed={} behavior={} inputs={} contract={}",
        known(case_id)?,
        seed?,
        known(behavior)?,
        known(inputs)?,
        known(contract)?
    ))
}

/// The executable identity the plan froze for every scenario it runs
/// (selected tests and controls), one line each in scenario order; `None`
/// when the plan froze none or any is incomplete.
fn frozen_coverage(plan: &CyclePlanV1) -> Option<String> {
    let mut lines: Vec<String> = plan
        .tests
        .iter()
        .map(|test| {
            coverage_line(
                &test.scenario_id,
                Some(test.case_id.as_str()),
                Some(test.seed),
                Some(test.behavior_sha256.as_str()),
                Some(test.inputs_sha256.as_str()),
                Some(test.contract_sha256.as_str()),
            )
        })
        .collect::<Option<_>>()?;
    lines.sort();
    (!lines.is_empty()).then(|| lines.join("; "))
}

/// The same lines as a side's reports state them: every reported scenario,
/// a scenario reported twice with different identities twice; `None` when a
/// field is unknown.
fn observed_coverage(identity: &EvidenceIdentityV1) -> Option<String> {
    let mut lines: Vec<String> = identity
        .scenarios
        .iter()
        .map(|scenario| {
            coverage_line(
                &scenario.scenario_id,
                scenario.case_id.as_deref(),
                scenario.seed,
                scenario.behavior_sha256.as_deref(),
                scenario.inputs_sha256.as_deref(),
                scenario.contract_sha256.as_deref(),
            )
        })
        .collect::<Option<_>>()?;
    lines.sort();
    (!lines.is_empty()).then(|| lines.join("; "))
}

/// Everything that must match between the two sides: scenario behavior and
/// inputs, contract, the whole subject artifact, every stack component but
/// the Harness, E2E revision, engine, model and provider, the cases and the
/// policy (both also the frozen plan's), and each side's scenarios exactly
/// the frozen coverage (case, seed, behavior, inputs and contract digests).
/// Only the product under test may differ. Unknown values never match.
pub fn compatibility(
    plan: &CyclePlanV1,
    baseline: &DecisionSideV1,
    candidate: &DecisionSideV1,
) -> ComparabilityV1 {
    let (left, right) = (&baseline.identity, &candidate.identity);
    let pairs = [
        (
            "behavior_sha256",
            scenarios(left, |scenario| scenario.behavior_sha256.clone()),
            scenarios(right, |scenario| scenario.behavior_sha256.clone()),
            None,
        ),
        (
            "inputs_sha256",
            scenarios(left, |scenario| scenario.inputs_sha256.clone()),
            scenarios(right, |scenario| scenario.inputs_sha256.clone()),
            None,
        ),
        (
            "contract_digest",
            left.contract_digest.clone(),
            right.contract_digest.clone(),
            None,
        ),
        // The whole subject artifact: model, provider, agent profile id and
        // configuration digest, context and output limits.
        (
            "subject_sha256",
            left.subject_sha256.clone(),
            right.subject_sha256.clone(),
            None,
        ),
        // Every stack component but the Harness, from the Registry identity
        // the E2E recorded; unknown in Source mode.
        (
            "fixed_stack_digest",
            left.fixed_stack_digest.clone(),
            right.fixed_stack_digest.clone(),
            None,
        ),
        (
            "model",
            left.model.clone(),
            right.model.clone(),
            Some(plan.parameters.model.clone()),
        ),
        (
            "provider",
            left.provider.clone(),
            right.provider.clone(),
            Some(plan.parameters.provider.clone()),
        ),
        (
            "e2e_revision",
            left.e2e_revision.clone(),
            right.e2e_revision.clone(),
            None,
        ),
        (
            "engine_version",
            left.engine_version.clone(),
            right.engine_version.clone(),
            None,
        ),
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
    let mut checks: Vec<ComparabilityCheckV1> = pairs
        .into_iter()
        .map(
            |(field, baseline, candidate, planned)| ComparabilityCheckV1 {
                field: field.into(),
                matches: baseline.is_some()
                    && baseline == candidate
                    && (planned.is_none() || baseline == planned),
                baseline,
                candidate,
            },
        )
        .collect();
    // Each side against the coverage frozen with the plan, not only against
    // the other side: two sides that both ran a moved catalog definition
    // agree with each other and still measured something else.
    let expected = frozen_coverage(plan);
    for (name, side) in [("baseline", baseline), ("candidate", candidate)] {
        let observed = observed_coverage(&side.identity);
        checks.push(ComparabilityCheckV1 {
            field: format!("{name}_frozen_coverage"),
            matches: expected.is_some() && observed == expected,
            baseline: expected.clone(),
            candidate: observed,
        });
    }
    // What each side must have run, as the E2E recorded it: the intended
    // Harness difference is checked against its expectation, never assumed.
    // `baseline` holds the expectation and `candidate` the observation.
    for (name, side) in [("baseline", baseline), ("candidate", candidate)] {
        for (what, expected, observed) in [
            (
                "revision",
                &side.expected_revision,
                &side.identity.workers_revision,
            ),
            ("stack", &side.expected_stack, &side.observed_stack),
        ] {
            if let Some(expected) = expected {
                checks.push(ComparabilityCheckV1 {
                    field: format!("{name}_{what}"),
                    matches: observed.as_deref() == Some(expected.as_str()),
                    baseline: Some(expected.clone()),
                    candidate: observed.clone(),
                });
            }
        }
    }
    ComparabilityV1 {
        comparable: checks.iter().all(|check| check.matches),
        checks,
    }
}

/// The decision on a pair by the frozen `plan`. `evidence` holds the runs of
/// each scenario (see [`pair_evidence`]); evidence of another
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
        baseline.execution_id.as_str(),
        candidate.execution_id.as_str(),
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
        // An observed loss stands whatever the sample; anything else on a
        // sample that is not the frozen one is unknown.
        if judged.computed_outcome != ValidationOutcomeV1::Regression {
            if let Some(gap) = sampling_gap(&judged, plan.sampling.runs_per_side) {
                judged.computed_outcome = ValidationOutcomeV1::Inconclusive;
                judged.reason = format!("{gap}; {}", judged.reason);
            }
        }
        judged
    };
    let primary = judged(&plan.criterion);
    let preserved: Vec<EvidenceV1> = plan.preserved.iter().map(judged).collect();
    let (outcome, reason) = verdict(&comparability, &plan.criterion, &primary, &preserved);
    CycleDecisionV1 {
        outcome,
        reason,
        policy_revision: plan.policy_revision.clone(),
        baseline_execution_id: baseline_id.into(),
        candidate_execution_id: candidate_id.into(),
        comparability,
        primary,
        preserved,
        decided_at: now,
    }
}

fn verdict(
    comparability: &ComparabilityV1,
    criterion: &CriterionV1,
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
        // Without a positive frozen minimum effect there is no improvement to
        // approve: any raw difference, however lucky, stays unknown.
        ValidationOutcomeV1::ValidatedImprovement if criterion.min_effect <= 0.0 => (
            Inconclusive,
            format!(
                "the primary criterion freezes no positive minimum effect, so no gain is approved: {}",
                primary.reason
            ),
        ),
        ValidationOutcomeV1::ValidatedImprovement => (Accepted, primary.reason.clone()),
        ValidationOutcomeV1::Regression => (Rejected, format!("regression: {}", primary.reason)),
        ValidationOutcomeV1::NoImprovement => (
            Rejected,
            format!("no improvement of the minimum effect: {}", primary.reason),
        ),
        ValidationOutcomeV1::Inconclusive => (Inconclusive, primary.reason.clone()),
    }
}

/// Decides the cycle's pair once by its frozen plan and stores the decision
/// in the record: a repeated event never starts another comparison. While
/// any work of the cycle has an unknown cost, a cost reduction is never
/// accepted (inconclusive).
pub fn decide_cycle(
    record: &mut CycleRecordV1,
    ledger: Option<&CycleLedgerV1>,
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
    let decision = measured(
        spent(
            decide(plan, baseline, candidate, evidence, now),
            plan,
            ledger,
            now,
        ),
        plan,
        record,
    );
    record.decision = Some(decision.clone());
    record.updated_at = now;
    Ok(decision)
}

/// Whether `worker` produces the token, cost and function-call totals: the
/// Harness counts them (`harness::metrics`) from the usage the router and
/// the providers report.
fn meters(worker: &str) -> bool {
    matches!(worker, "harness" | "llm-router") || worker.starts_with("provider-")
}

/// An accepted source decision stands only on what the validation ran:
/// every unit the candidate touches (shared code aside) is built from the
/// candidate commit by the source stack, some built worker runs the change,
/// and a total the changed code reports itself never qualifies it.
/// Dependents the stack does not build are named: not measured.
fn measured(
    mut decision: CycleDecisionV1,
    plan: &CyclePlanV1,
    record: &CycleRecordV1,
) -> CycleDecisionV1 {
    let Some(commit) = record
        .candidate
        .as_ref()
        .and_then(|state| state.candidate.as_ref()?.commit.as_ref())
    else {
        return decision;
    };
    if decision.outcome != DecisionOutcomeV1::Accepted {
        return decision;
    }
    let built = record
        .config
        .as_ref()
        .and_then(|stored| stored.config.validation.as_ref())
        .map(ValidationConfigV1::built_from_commit)
        .unwrap_or_default();
    let unbuilt: Vec<&str> = commit
        .units
        .iter()
        .filter(|unit| !crate::candidate::is_shared(unit) && !built.contains(*unit))
        .map(String::as_str)
        .collect();
    let self_reported = matches!(
        plan.criterion.metric,
        CriterionMetricV1::Tokens | CriterionMetricV1::CostUsd | CriterionMetricV1::FunctionCalls
    );
    let measured = commit.measured_in(&built);
    let gap = if !unbuilt.is_empty() {
        format!(
            "the candidate changes {} and the validation stack does not build it from the \
             candidate commit, so no run measured that change",
            unbuilt.join(", ")
        )
    } else if measured.is_empty() {
        "no worker the validation stack builds from the candidate commit runs this change".into()
    } else if let Some(meter) = commit
        .units
        .iter()
        .chain(&commit.dependents)
        .find(|unit| self_reported && meters(unit))
    {
        format!(
            "source qualification requires an independent meter: {meter} counts the {:?} \
             total this plan measures, and the candidate changes it",
            plan.criterion.metric
        )
    } else {
        let unmeasured: Vec<&str> = commit
            .dependents
            .iter()
            .filter(|unit| !built.contains(*unit))
            .map(String::as_str)
            .collect();
        if !unmeasured.is_empty() {
            decision.reason = format!(
                "{}; not measured: {}, which build with the \
                 changed code",
                decision.reason,
                unmeasured.join(", ")
            );
        }
        return decision;
    };
    decision.outcome = DecisionOutcomeV1::Inconclusive;
    decision.reason = format!("{gap}; {}", decision.reason);
    decision
}

fn spent(
    mut decision: CycleDecisionV1,
    plan: &CyclePlanV1,
    ledger: Option<&CycleLedgerV1>,
    now: i64,
) -> CycleDecisionV1 {
    let unknown = ledger.map_or(0, |ledger| cost::usage(ledger, now).unknown_cost);
    if decision.outcome == DecisionOutcomeV1::Accepted
        && plan.criterion.metric == CriterionMetricV1::CostUsd
        && unknown > 0
    {
        decision.outcome = DecisionOutcomeV1::Inconclusive;
        decision.reason = format!(
            "no cost reduction is claimed: {unknown} work item(s) of the cycle have an unknown cost; {}",
            decision.reason
        );
    }
    decision
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
            criterion_id: None,
            min_runs: 3,
            scenario_id: "s".into(),
        }
    }

    fn run(completed: bool, signal: Option<u32>, status: &str) -> EvidenceRunV1 {
        EvidenceRunV1 {
            run_id: "r".into(),
            case_id: Some("s:seed-1".into()),
            completed,
            completion: Some(if completed { "completed" } else { "incomplete" }.into()),
            technical: Some(if completed { "valid" } else { "infrastructure" }.into()),
            status: Some(status.into()),
            cost_usd: Some(0.5),
            wall_time_ms: None,
            total_tokens: None,
            function_calls: None,
            function_call_errors: None,
            criteria: BTreeMap::new(),
            signals: signal.map(|count| {
                BTreeMap::from([("repeated_tool_error:crm::schedule".to_string(), count)])
            }),
        }
    }

    /// Native runs carry distinct ids: number them in order.
    fn side(runs: Vec<EvidenceRunV1>) -> EvidenceSideV1 {
        EvidenceSideV1 {
            execution_id: "x".into(),
            runs: runs
                .into_iter()
                .enumerate()
                .map(|(at, run)| EvidenceRunV1 {
                    run_id: format!("r{at}"),
                    ..run
                })
                .collect(),
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
    fn the_sign_test_tail_is_exact() {
        assert_eq!(sign_test_p(1, 1), 0.5);
        assert!((sign_test_p(5, 5) - 0.03125).abs() < 1e-12);
        assert!((sign_test_p(4, 5) - 6.0 / 32.0).abs() < 1e-12);
        assert!((sign_test_p(3, 3) - 0.125).abs() < 1e-12);
        assert!((sign_test_p(9, 10) - 11.0 / 1024.0).abs() < 1e-12);
        assert_eq!(sign_test_p(0, 4), 1.0);
    }

    #[test]
    fn a_large_sample_does_not_turn_one_outlier_into_significance() {
        let criterion = CriterionV1 {
            criterion_id: Some("done".into()),
            min_effect: 0.01,
            min_runs: 1_100,
            ..criterion(CriterionMetricV1::CriterionScore, DirectionV1::Increase)
        };
        let runs = |improved: bool| {
            (0..1_100)
                .map(|at| EvidenceRunV1 {
                    criteria: BTreeMap::from([("done".into(), f64::from(improved && at == 0))]),
                    ..run(true, None, "passed")
                })
                .collect()
        };
        let (found, reason, _) = outcome(runs(false), runs(true), &criterion);
        assert_eq!(found, ValidationOutcomeV1::Inconclusive, "{reason}");
        assert!(sign_test_p(1, 1_100) > 0.99);
        assert!(sign_test_p(550, 1_100) > 0.5);
        assert!(sign_test_p(550, 1_100) < 0.52);
        assert!(sign_test_p(600, 1_100) < SIGN_TEST_ALPHA);
    }

    #[test]
    fn the_outcome_comes_from_the_means_the_minimum_effect_and_the_direction() {
        use ValidationOutcomeV1::*;
        let decrease = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Decrease);
        // 3.0 → 0.0 is 3 signals fewer per run, on every one of 5 pairs.
        let (found, reason, n) = outcome(signals(&[3; 5]), signals(&[0; 5]), &decrease);
        assert_eq!((found, n), (ValidatedImprovement, (5, 5)), "{reason}");
        assert!(
            reason.contains("baseline 3 → candidate 0 (-3 signals per run), n=5/5")
                && reason.contains("a decrease of 2 signals per run or more")
                && reason.contains("5 of 5 same-case run pairs")
                && reason.contains("p=0.03125")
                && reason.contains("not a confidence interval"),
            "{reason}"
        );
        // The same mean effect on 3 pairs cannot reach p <= 0.05.
        let (found, reason, _) = outcome(signals(&[3; 3]), signals(&[0; 3]), &decrease);
        assert_eq!(found, Inconclusive, "{reason}");
        assert!(
            reason.contains("3 of 3") && reason.contains("p=0.12500"),
            "{reason}"
        );
        // Exactly the minimum counts; one fewer than that does not.
        assert_eq!(
            outcome(signals(&[3; 5]), signals(&[1; 5]), &decrease).0,
            ValidatedImprovement
        );
        assert_eq!(
            outcome(signals(&[3; 5]), signals(&[2; 5]), &decrease).0,
            NoImprovement
        );
        assert_eq!(
            outcome(signals(&[0; 5]), signals(&[0; 5]), &decrease).0,
            NoImprovement
        );
        // The wrong way by at least the minimum is a regression; by less is not.
        assert_eq!(
            outcome(signals(&[2; 5]), signals(&[4; 5]), &decrease).0,
            Regression
        );
        assert_eq!(
            outcome(signals(&[2; 5]), signals(&[3; 5]), &decrease).0,
            NoImprovement
        );
        let increase = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Increase);
        assert_eq!(
            outcome(signals(&[2; 5]), signals(&[4; 5]), &increase).0,
            ValidatedImprovement
        );
        // A mean carried by one outlier pair is not a consistent direction.
        let (found, reason, _) = outcome(
            signals(&[3; 5]),
            signals(&[0, 0, 0, 3, 3]),
            &CriterionV1 {
                min_effect: 1.0,
                ..decrease.clone()
            },
        );
        assert_eq!(found, Inconclusive, "{reason}");
        assert!(reason.contains("3 of 5"), "{reason}");
    }

    #[test]
    fn a_zero_threshold_holds_an_unchanged_mean_and_rejects_any_loss() {
        use ValidationOutcomeV1::*;
        let held = CriterionV1 {
            min_effect: 0.0,
            ..criterion(CriterionMetricV1::PassRate, DirectionV1::Increase)
        };
        let runs = |passed: usize| -> Vec<EvidenceRunV1> {
            (0..5)
                .map(|at| run(true, None, if at < passed { "passed" } else { "failed" }))
                .collect()
        };
        assert_eq!(outcome(runs(5), runs(5), &held).0, NoImprovement);
        assert_eq!(outcome(runs(5), runs(4), &held).0, Regression);
    }

    #[test]
    fn pairing_must_be_unambiguous() {
        use ValidationOutcomeV1::*;
        let decrease = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Decrease);
        // Pairs of different cases are never paired.
        let mut other_case = signals(&[0; 5]);
        other_case[2].case_id = Some("s:seed-2".into());
        let (found, reason, _) = outcome(signals(&[3; 5]), other_case, &decrease);
        assert_eq!(found, Inconclusive, "{reason}");
        assert!(reason.contains("run pair 2 is not of one case"), "{reason}");
        // A run reported twice is never counted twice.
        let mut evidence = evidence_of(signals(&[3; 5]), signals(&[0; 5]));
        evidence.candidate.runs[4].run_id = "r0".into();
        judge(&mut evidence, Some(&decrease));
        assert_eq!(evidence.computed_outcome, Inconclusive);
        assert!(evidence.reason.contains("twice"), "{}", evidence.reason);
        assert!(sampling_gap(&evidence, 5).is_some_and(|gap| gap.contains("more than once")));
        // Sides of different sizes are not paired, and miss the frozen sample.
        let evidence = evidence_of(signals(&[3; 5]), signals(&[0; 6]));
        assert!(directional(&evidence, &decrease).is_err());
        assert!(sampling_gap(&evidence, 5).is_some_and(|gap| gap.contains("exactly 5")));
    }

    #[test]
    fn the_minimum_effect_is_in_the_metrics_own_unit() {
        use CriterionMetricV1::*;
        use ValidationOutcomeV1::*;
        let measured = |metric: CriterionMetricV1, value: f64| -> Vec<EvidenceRunV1> {
            (0..5)
                .map(|_| {
                    let mut run = run(true, None, "passed");
                    match metric {
                        CostUsd => run.cost_usd = Some(value),
                        Duration => run.wall_time_ms = Some(value),
                        Tokens => run.total_tokens = Some(value),
                        FunctionCallErrors => run.function_call_errors = Some(value),
                        _ => run.function_calls = Some(value),
                    }
                    run
                })
                .collect()
        };
        for (metric, min_effect, baseline, candidate, unit) in [
            (CostUsd, 0.01, 0.07, 0.06, "USD"),
            (Duration, 5.0, 30_000.0, 25_000.0, "s"),
            (Tokens, 1_000.0, 9_000.0, 8_000.0, "tokens"),
            (FunctionCalls, 2.0, 9.0, 7.0, "calls"),
            (FunctionCallErrors, 1.0, 3.0, 2.0, "errors"),
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
        // Pass rate in percentage points: 0.5 → 0.6 over 10 runs meets the
        // 10 pp mean effect, but only one pair moved: no consistent direction.
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
        assert_eq!(found, Inconclusive, "{reason}");
        assert!(
            reason.contains("baseline 0.5 → candidate 0.6 (+10 pp)")
                && reason.contains("1 of 10 same-case run pairs"),
            "{reason}"
        );
        // Five more passes out of ten paired runs is consistent enough.
        assert_eq!(
            outcome(passing(0), passing(9), &pass).0,
            ValidatedImprovement
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
        let gave_up = || EvidenceRunV1 {
            completed: false,
            completion: Some("task_incomplete".into()),
            ..run(true, None, "passed")
        };
        let mut candidate = runs(3, 0);
        candidate.extend([gave_up(), gave_up()]);
        let (found, reason, n) = outcome(runs(3, 2), candidate, &pass);
        assert_eq!((found, n), (NoImprovement, (5, 5)), "{reason}");
        assert!(reason.contains("baseline 0.6 → candidate 0.6"), "{reason}");
        let mut broken = runs(3, 0);
        broken.extend([run(false, None, "failed"), run(false, None, "failed")]);
        assert_eq!(outcome(runs(3, 2), broken, &pass).2, (5, 3));
        // An unfinished run's lower effort is never an effort gain.
        let tokens = CriterionV1 {
            min_effect: 100.0,
            ..criterion(CriterionMetricV1::Tokens, DirectionV1::Decrease)
        };
        let spent = |value: f64, finished: bool| -> Vec<EvidenceRunV1> {
            (0..5)
                .map(|_| EvidenceRunV1 {
                    total_tokens: Some(value),
                    ..if finished {
                        run(true, None, "passed")
                    } else {
                        gave_up()
                    }
                })
                .collect()
        };
        let (found, _, n) = outcome(spent(1_000.0, true), spent(10.0, false), &tokens);
        assert_eq!((found, n), (Inconclusive, (5, 0)));
    }

    #[test]
    fn missing_data_is_inconclusive_with_the_reason() {
        use ValidationOutcomeV1::*;
        let decrease = criterion(CriterionMetricV1::SignalPerRun, DirectionV1::Decrease);
        let (found, reason, n) = outcome(signals(&[3, 3, 3]), signals(&[0, 0]), &decrease);
        assert_eq!((found, n), (Inconclusive, (3, 2)));
        assert!(
            reason.contains("the candidate has 2 completed run(s)"),
            "{reason}"
        );
        let mut flaky = signals(&[0, 0]);
        flaky.extend([
            run(false, None, "failed"),
            run(false, None, "failed"),
            run(false, None, "failed"),
        ]);
        let (found, reason, _) = outcome(signals(&[3, 3, 3]), flaky, &decrease);
        assert_eq!(found, Inconclusive);
        assert!(
            reason.contains("3 of 5 candidate runs failed technically"),
            "{reason}"
        );
        // Exactly half broken passes the completion gate, but a broken pair
        // never reaches the effect: 3 of 6 is no consistent direction.
        let mut half = signals(&[0, 0, 0]);
        half.extend([
            run(false, None, "failed"),
            run(false, None, "failed"),
            run(false, None, "failed"),
        ]);
        let (found, reason, _) = outcome(signals(&[3; 6]), half, &decrease);
        assert_eq!(found, Inconclusive, "{reason}");
        assert!(reason.contains("3 of 6 same-case run pairs"), "{reason}");
        let blind: Vec<_> = (0..3).map(|_| run(true, None, "passed")).collect();
        assert_eq!(outcome(signals(&[3, 3, 3]), blind, &decrease).2, (3, 0));
        assert!(outcome(vec![], signals(&[0, 0, 0]), &decrease)
            .1
            .contains("no run of s"));
        let mut evidence = evidence_of(signals(&[1, 1]), signals(&[0]));
        judge(&mut evidence, None);
        assert_eq!(evidence.computed_outcome, Inconclusive);
        assert_eq!((evidence.baseline.n, evidence.baseline.mean), (2, None));
    }
}
