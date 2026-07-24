use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Serialize;

use crate::error::EvalError;
use crate::limits::AgentQualityLimitsV1;
use crate::report::AgentQualityRunReportV1;
use crate::scenarios::ScenarioId;
use crate::subject::{ResolvedAgentQualitySubjectV1, SubjectArtifactV1};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonDimension {
    Model,
    SystemPrompt,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioAggregateV1 {
    pub runs: u32,
    pub passed: u32,
    pub pass_rate: f64,
    pub benchmarked_runs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_wall_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_function_calls: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_function_call_errors: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_trace_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_span_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_error_span_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_trace_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioComparisonV1 {
    pub scenario_id: String,
    pub control: ScenarioAggregateV1,
    pub treatment: ScenarioAggregateV1,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromptComparisonReportV1 {
    pub schema_version: String,
    pub dimension: ComparisonDimension,
    pub control: SubjectArtifactV1,
    pub treatment: SubjectArtifactV1,
    pub limits: AgentQualityLimitsV1,
    pub eligible: bool,
    pub regressed_scenarios: Vec<String>,
    pub scenarios: Vec<ScenarioComparisonV1>,
}

impl PromptComparisonReportV1 {
    pub fn write_to(&self, output: &Path) -> Result<PathBuf, EvalError> {
        fs::create_dir_all(output).map_err(|error| {
            EvalError::setup(format!(
                "create comparison directory {}: {error}",
                output.display()
            ))
        })?;
        let path = output.join("comparison.json");
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| EvalError::setup(format!("serialize {}: {error}", path.display())))?;
        bytes.push(b'\n');
        fs::write(&path, bytes)
            .map_err(|error| EvalError::setup(format!("write {}: {error}", path.display())))?;
        Ok(path)
    }
}

pub fn comparison_dimension(
    control: &ResolvedAgentQualitySubjectV1,
    treatment: &ResolvedAgentQualitySubjectV1,
) -> Result<ComparisonDimension, EvalError> {
    require_equal(
        "system_prompt_strategy",
        control.system_prompt_strategy,
        treatment.system_prompt_strategy,
    )?;
    require_equal(
        "thinking_level",
        control.thinking_level,
        treatment.thinking_level,
    )?;
    require_equal(
        "provider_options",
        &control.provider_options,
        &treatment.provider_options,
    )?;
    require_equal("provider", &control.provider, &treatment.provider)?;

    let prompt_changed = control.system_prompt_sha256 != treatment.system_prompt_sha256;
    let model_changed = control.model != treatment.model;

    if !prompt_changed && model_changed {
        return Ok(ComparisonDimension::Model);
    }
    if prompt_changed && !model_changed {
        return Ok(ComparisonDimension::SystemPrompt);
    }
    Err(EvalError::setup(
        "comparison must change either the model or the system prompt, but not both",
    ))
}

pub fn compare_runs(
    dimension: ComparisonDimension,
    control: SubjectArtifactV1,
    treatment: SubjectArtifactV1,
    scenarios: &[ScenarioId],
    control_runs: &[AgentQualityRunReportV1],
    treatment_runs: &[AgentQualityRunReportV1],
) -> Result<PromptComparisonReportV1, EvalError> {
    if control_runs.is_empty() || control_runs.len() != treatment_runs.len() {
        return Err(EvalError::setup(
            "comparison requires the same non-zero number of control and treatment runs",
        ));
    }
    let limits = control_runs[0].limits;
    if control_runs
        .iter()
        .chain(treatment_runs)
        .any(|report| report.limits != limits)
    {
        return Err(EvalError::setup(
            "every control and treatment run must use the same limits",
        ));
    }

    let comparisons: Vec<_> = scenarios
        .iter()
        .map(|scenario| {
            let scenario_id = scenario.as_str();
            Ok(ScenarioComparisonV1 {
                scenario_id: scenario_id.to_string(),
                control: aggregate(control_runs, scenario_id)?,
                treatment: aggregate(treatment_runs, scenario_id)?,
            })
        })
        .collect::<Result<_, EvalError>>()?;
    let regressed_scenarios: Vec<_> = comparisons
        .iter()
        .filter(|comparison| comparison.treatment.passed < comparison.control.passed)
        .map(|comparison| comparison.scenario_id.clone())
        .collect();
    let treatment_passed_all = comparisons
        .iter()
        .all(|comparison| comparison.treatment.passed == comparison.treatment.runs);

    Ok(PromptComparisonReportV1 {
        schema_version: "1".into(),
        dimension,
        control,
        treatment,
        limits,
        eligible: regressed_scenarios.is_empty() && treatment_passed_all,
        regressed_scenarios,
        scenarios: comparisons,
    })
}

fn aggregate(
    reports: &[AgentQualityRunReportV1],
    scenario_id: &str,
) -> Result<ScenarioAggregateV1, EvalError> {
    let scenarios: Vec<_> = reports
        .iter()
        .map(|report| {
            report
                .scenarios
                .iter()
                .find(|scenario| scenario.scenario_id == scenario_id)
                .ok_or_else(|| {
                    EvalError::setup(format!(
                        "run {} has no scenario {scenario_id}",
                        report.run_id
                    ))
                })
        })
        .collect::<Result<_, _>>()?;
    let benchmarks: Vec<_> = scenarios
        .iter()
        .filter_map(|scenario| scenario.benchmark())
        .collect();
    let runs = reports.len() as u32;
    let passed = scenarios.iter().filter(|scenario| scenario.passed).count() as u32;

    Ok(ScenarioAggregateV1 {
        runs,
        passed,
        pass_rate: passed as f64 / runs as f64,
        benchmarked_runs: benchmarks.len() as u32,
        median_wall_time_ms: median_u64(benchmarks.iter().map(|value| value.wall_time_ms)),
        median_input_tokens: median_u64(benchmarks.iter().filter_map(|value| value.input_tokens)),
        median_output_tokens: median_u64(benchmarks.iter().filter_map(|value| value.output_tokens)),
        median_total_tokens: median_u64(benchmarks.iter().filter_map(|value| value.total_tokens)),
        median_reasoning_tokens: median_u64(
            benchmarks.iter().filter_map(|value| value.reasoning_tokens),
        ),
        median_function_calls: median_u64(benchmarks.iter().map(|value| value.function_calls)),
        median_function_call_errors: median_u64(
            benchmarks.iter().map(|value| value.function_call_errors),
        ),
        median_cost_usd: median_f64(benchmarks.iter().filter_map(|value| value.cost_usd)),
        median_trace_count: median_u64(benchmarks.iter().filter_map(|value| value.trace_count)),
        median_span_count: median_u64(benchmarks.iter().filter_map(|value| value.span_count)),
        median_error_span_count: median_u64(
            benchmarks.iter().filter_map(|value| value.error_span_count),
        ),
        median_trace_duration_ms: median_u64(
            benchmarks
                .iter()
                .filter_map(|value| value.trace_duration_ms),
        ),
    })
}

fn median_u64(values: impl IntoIterator<Item = u64>) -> Option<u64> {
    let mut values: Vec<_> = values.into_iter().collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[middle])
    } else {
        Some(values[middle - 1].saturating_add(values[middle]) / 2)
    }
}

fn median_f64(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<_> = values
        .into_iter()
        .filter(|value| value.is_finite())
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[middle])
    } else {
        Some((values[middle - 1] + values[middle]) / 2.0)
    }
}

fn require_equal<T: PartialEq>(name: &str, control: T, treatment: T) -> Result<(), EvalError> {
    if control == treatment {
        Ok(())
    } else {
        Err(EvalError::setup(format!(
            "{name} must remain equal between control and treatment"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use harness::prompt::SystemPromptStrategy;
    use harness::types::model::ThinkingLevel;

    use super::*;
    use crate::report::AgentQualityScenarioReportV1;

    fn subject(model: &str, provider: &str, prompt_hash: &str) -> ResolvedAgentQualitySubjectV1 {
        ResolvedAgentQualitySubjectV1 {
            subject_id: format!("{model}-{prompt_hash}"),
            subject_sha256: "a".repeat(64),
            system_prompt_sha256: prompt_hash.into(),
            model: model.into(),
            provider: provider.into(),
            system_prompt: "prompt".into(),
            system_prompt_strategy: SystemPromptStrategy::Override,
            thinking_level: Some(ThinkingLevel::Low),
            provider_options: Some(BTreeMap::new()),
        }
    }

    #[test]
    fn model_comparison_changes_only_the_model_route() {
        let control = subject("model-a", "provider", "prompt-a");
        let treatment = subject("model-b", "provider", "prompt-a");
        assert_eq!(
            comparison_dimension(&control, &treatment).unwrap(),
            ComparisonDimension::Model
        );
    }

    #[test]
    fn system_prompt_comparison_changes_only_the_prompt() {
        let control = subject("model-a", "provider", "prompt-a");
        let treatment = subject("model-a", "provider", "prompt-b");
        assert_eq!(
            comparison_dimension(&control, &treatment).unwrap(),
            ComparisonDimension::SystemPrompt
        );
    }

    #[test]
    fn rejects_ambiguous_or_identical_comparisons() {
        let baseline = subject("model-a", "provider", "prompt-a");
        assert!(comparison_dimension(&baseline, &baseline).is_err());
        assert!(
            comparison_dimension(&baseline, &subject("model-b", "provider", "prompt-b")).is_err()
        );
        assert!(
            comparison_dimension(&baseline, &subject("model-a", "other-provider", "prompt-a"))
                .is_err()
        );
    }

    #[test]
    fn median_handles_odd_even_and_missing_values() {
        assert_eq!(median_u64([]), None);
        assert_eq!(median_u64([3]), Some(3));
        assert_eq!(median_u64([9, 1, 5]), Some(5));
        assert_eq!(median_u64([10, 2]), Some(6));
        assert_eq!(median_f64([]), None);
        assert_eq!(median_f64([1.0, 3.0]), Some(2.0));
    }

    #[test]
    fn treatment_must_pass_every_run_without_regressing() {
        let control_subject = subject("model-a", "provider", "prompt").artifact();
        let treatment_subject = subject("model-b", "provider", "prompt").artifact();
        let control = [
            report("control-1", true, control_subject.clone()),
            report("control-2", true, control_subject.clone()),
        ];
        let treatment = [
            report("treatment-1", true, treatment_subject.clone()),
            report("treatment-2", false, treatment_subject.clone()),
        ];

        let comparison = compare_runs(
            ComparisonDimension::Model,
            control_subject,
            treatment_subject,
            &[ScenarioId::PlainResponse],
            &control,
            &treatment,
        )
        .unwrap();
        assert!(!comparison.eligible);
        assert_eq!(comparison.regressed_scenarios, ["plain_response"]);
        assert_eq!(comparison.scenarios[0].control.passed, 2);
        assert_eq!(comparison.scenarios[0].treatment.passed, 1);
    }

    fn report(run_id: &str, passed: bool, subject: SubjectArtifactV1) -> AgentQualityRunReportV1 {
        let mut scenario = AgentQualityScenarioReportV1::new(
            "plain_response",
            format!("{run_id}-session"),
            "prompt".into(),
        );
        scenario.wall_time_ms = 10;
        scenario.passed = passed;
        scenario.evaluation = passed.then(|| serde_json::json!({ "ok": true }));
        AgentQualityRunReportV1::new(
            run_id.into(),
            subject,
            AgentQualityLimitsV1::default(),
            vec![scenario],
        )
    }
}
