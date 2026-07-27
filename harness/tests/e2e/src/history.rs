use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct HistoryComparison {
    pub compatible: bool,
    pub passed: bool,
    pub max_score_drop: f64,
    pub incompatibilities: Vec<String>,
    pub scenarios: Vec<ScenarioComparison>,
}

#[derive(Debug, Serialize)]
pub struct ScenarioComparison {
    pub scenario_id: String,
    pub baseline_median_score: Option<f64>,
    pub candidate_median_score: Option<f64>,
    pub score_change: Option<f64>,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn compare_files(
    baseline_path: &Path,
    candidate_path: &Path,
    max_score_drop: f64,
) -> Result<HistoryComparison> {
    let baseline = read_report(baseline_path)?;
    let candidate = read_report(candidate_path)?;
    compare_reports(&baseline, &candidate, max_score_drop)
}

pub fn compare_reports(
    baseline: &Value,
    candidate: &Value,
    max_score_drop: f64,
) -> Result<HistoryComparison> {
    if !max_score_drop.is_finite() || max_score_drop < 0.0 {
        bail!("max score drop must be a finite non-negative number");
    }
    let mut incompatibilities = Vec::new();
    for pointer in [
        "/subject/model",
        "/subject/provider",
        "/judge/model",
        "/judge/provider",
        "/judge_protocol",
    ] {
        require_equal(baseline, candidate, pointer, &mut incompatibilities);
    }
    require_known_equal_engine(baseline, candidate, &mut incompatibilities);

    let baseline_scenarios = scenarios_by_id(baseline, "baseline")?;
    let candidate_scenarios = scenarios_by_id(candidate, "candidate")?;
    let baseline_ids: BTreeSet<_> = baseline_scenarios.keys().copied().collect();
    let candidate_ids: BTreeSet<_> = candidate_scenarios.keys().copied().collect();
    if baseline_ids != candidate_ids {
        incompatibilities.push(format!(
            "scenario sets differ: baseline={baseline_ids:?}, candidate={candidate_ids:?}"
        ));
    }

    let mut scenarios = Vec::new();
    for scenario_id in baseline_ids.intersection(&candidate_ids) {
        let baseline_scenario = baseline_scenarios[scenario_id];
        let candidate_scenario = candidate_scenarios[scenario_id];
        for field in ["threshold", "requirements", "execution_policy"] {
            if baseline_scenario.get(field) != candidate_scenario.get(field) {
                incompatibilities.push(format!("scenario {scenario_id} differs at {field}"));
            }
        }

        let baseline_score = median_score(baseline_scenario)?;
        let candidate_score = median_score(candidate_scenario)?;
        let score_change = baseline_score
            .zip(candidate_score)
            .map(|(baseline, candidate)| candidate - baseline);
        let reason = match (baseline_score, candidate_score) {
            (Some(_), None) => Some("candidate has no comparable quality score".to_string()),
            (None, _) => Some("baseline has no comparable quality score".to_string()),
            (Some(baseline), Some(candidate)) if baseline - candidate > max_score_drop => {
                Some(format!(
                    "median score dropped by {:.2}, exceeding the allowed {:.2}",
                    baseline - candidate,
                    max_score_drop
                ))
            }
            _ => None,
        };
        scenarios.push(ScenarioComparison {
            scenario_id: (*scenario_id).to_string(),
            baseline_median_score: baseline_score,
            candidate_median_score: candidate_score,
            score_change,
            passed: reason.is_none(),
            reason,
        });
    }

    let compatible = incompatibilities.is_empty();
    let passed =
        compatible && !scenarios.is_empty() && scenarios.iter().all(|scenario| scenario.passed);
    Ok(HistoryComparison {
        compatible,
        passed,
        max_score_drop,
        incompatibilities,
        scenarios,
    })
}

fn read_report(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

fn require_equal(
    baseline: &Value,
    candidate: &Value,
    pointer: &str,
    incompatibilities: &mut Vec<String>,
) {
    if baseline.pointer(pointer) != candidate.pointer(pointer) {
        incompatibilities.push(format!("reports differ at {pointer}"));
    }
}

fn require_known_equal_engine(
    baseline: &Value,
    candidate: &Value,
    incompatibilities: &mut Vec<String>,
) {
    let baseline_revision = baseline.get("engine_revision").and_then(Value::as_str);
    let candidate_revision = candidate.get("engine_revision").and_then(Value::as_str);
    match (baseline_revision, candidate_revision) {
        (Some(baseline), Some(candidate)) if baseline == candidate => {}
        (Some(_), Some(_)) => {
            incompatibilities.push("reports use different engine revisions".into())
        }
        _ => incompatibilities
            .push("both reports need a known engine revision for historical comparison".into()),
    }
}

fn scenarios_by_id<'a>(report: &'a Value, label: &str) -> Result<BTreeMap<&'a str, &'a Value>> {
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{label} report has no scenarios array"))?;
    let mut result = BTreeMap::new();
    for scenario in scenarios {
        let id = scenario
            .get("scenario_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("{label} report has a scenario without an id"))?;
        if result.insert(id, scenario).is_some() {
            bail!("{label} report repeats scenario {id}");
        }
    }
    Ok(result)
}

fn median_score(scenario: &Value) -> Result<Option<f64>> {
    let value = scenario
        .pointer("/aggregate/median_score")
        .ok_or_else(|| anyhow!("scenario is missing aggregate.median_score"))?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_f64()
        .map(Some)
        .ok_or_else(|| anyhow!("scenario median score is not numeric or null"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(model: &str, score: Option<f64>) -> Value {
        json!({
            "subject": {"model": model, "provider": "subject-provider"},
            "judge": {"model": "judge", "provider": "judge-provider"},
            "judge_protocol": "plain-json",
            "engine_revision": "abc123",
            "scenarios": [{
                "scenario_id": "case",
                "threshold": 80,
                "requirements": {"tools": false},
                "execution_policy": {"max_turns": 2},
                "aggregate": {"median_score": score},
            }],
        })
    }

    #[test]
    fn compares_only_the_same_experiment_identity() {
        let outcome = compare_reports(
            &report("model", Some(90.0)),
            &report("model", Some(87.0)),
            3.0,
        )
        .unwrap();
        assert!(outcome.compatible);
        assert!(outcome.passed);
        assert_eq!(outcome.scenarios[0].score_change, Some(-3.0));

        let changed_model =
            compare_reports(&report("old", Some(90.0)), &report("new", Some(90.0)), 0.0).unwrap();
        assert!(!changed_model.compatible);
    }

    #[test]
    fn rejects_excessive_drop_and_missing_scores() {
        let dropped = compare_reports(
            &report("model", Some(90.0)),
            &report("model", Some(86.0)),
            3.0,
        )
        .unwrap();
        assert!(!dropped.passed);

        let missing =
            compare_reports(&report("model", Some(90.0)), &report("model", None), 3.0).unwrap();
        assert!(!missing.passed);
    }
}
