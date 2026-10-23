//! The comparison of a baseline and a candidate E2E execution: per-run
//! evidence read from the reports and the outcome computed in code from a
//! criterion, never by a model. Kept for the loop's decision.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::contract::*;
use crate::diagnostics;

/// Every run of `scenario_id` in an `execution-get` bundle. Detectors run on
/// each run's own transcript; a run that did not complete is kept and
/// identified, never dropped.
pub fn runs_of(bundle: &Value, scenario_id: &str, patterns: &[String]) -> Vec<EvidenceRunV1> {
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
/// side, with `patterns` counted per run, judged by `criterion`.
pub fn evidence(
    patterns: &[String],
    criterion: Option<&CriterionV1>,
    scenario_id: &str,
    baseline: (&str, &Value),
    candidate: (&str, &Value),
    now: i64,
) -> EvidenceV1 {
    let side = |(execution_id, bundle): (&str, &Value)| EvidenceSideV1 {
        execution_id: execution_id.into(),
        runs: runs_of(bundle, scenario_id, patterns),
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
    judge(&mut evidence, criterion);
    evidence
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

#[cfg(test)]
mod tests {
    use serde_json::json;

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
}
