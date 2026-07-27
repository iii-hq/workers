use harness::functions::metrics::SessionMetricsResponseV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contract::{
    ComparisonDimensionV1, EvalModelConfigV1, EvalRunStatusV1, ExecutionOrderV1,
    NormalizedEvalRequestV1, VariantRoleV1,
};
use crate::error::EvalFailureV1;
use crate::ids;
use crate::limits::EvalLimitsV1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalBenchmarkV1 {
    pub wall_time_ms: u64,
    pub sessions: u64,
    pub turns: u64,
    pub function_calls: u64,
    pub function_call_errors: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_span_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_duration_ms: Option<u64>,
}

impl EvalBenchmarkV1 {
    pub fn from_metrics(metrics: &SessionMetricsResponseV1, wall_time_ms: u64) -> Option<Self> {
        metrics.complete.then(|| Self {
            wall_time_ms,
            sessions: metrics.by_session.len() as u64,
            turns: metrics.totals.turns,
            function_calls: metrics.totals.function_calls,
            function_call_errors: metrics.totals.function_call_errors,
            input_tokens: metrics.totals.input_tokens,
            output_tokens: metrics.totals.output_tokens,
            total_tokens: metrics
                .totals
                .input_tokens
                .zip(metrics.totals.output_tokens)
                .map(|(input, output)| input.saturating_add(output)),
            cache_read_tokens: metrics.totals.cache_read_tokens,
            cache_write_tokens: metrics.totals.cache_write_tokens,
            reasoning_tokens: metrics.totals.reasoning_tokens,
            cost_usd: metrics.totals.cost_usd,
            trace_count: metrics.traces.as_ref().map(|traces| traces.trace_count),
            span_count: metrics.traces.as_ref().map(|traces| traces.span_count),
            error_span_count: metrics
                .traces
                .as_ref()
                .map(|traces| traces.error_span_count),
            trace_duration_ms: metrics.traces.as_ref().map(|traces| traces.duration_ms),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalRunReportV1 {
    pub run_id: String,
    pub role: VariantRoleV1,
    pub iteration: u32,
    #[serde(default)]
    pub execution_position: u32,
    #[serde(default)]
    pub pair_position: u8,
    pub status: EvalRunStatusV1,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<SessionMetricsResponseV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<crate::contract::EvaluatorResponseV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benchmark: Option<EvalBenchmarkV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<EvalFailureV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantArtifactV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub prompt_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorArtifactV1 {
    pub function_id: String,
    pub arguments_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharedArtifactsV1 {
    pub model_sha256: String,
    pub function_policy_sha256: String,
    pub limits_sha256: String,
    pub output_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleOrderSensitivityV1 {
    pub first_runs: u32,
    pub first_passed: u32,
    pub second_runs: u32,
    pub second_passed: u32,
    pub differs: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrderSensitivityV1 {
    pub detected: bool,
    pub control: RoleOrderSensitivityV1,
    pub treatment: RoleOrderSensitivityV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantAggregateV1 {
    pub runs: u32,
    pub evaluated_runs: u32,
    pub passed: u32,
    pub pass_rate: f64,
    pub benchmarked_runs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_score: Option<f64>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AggregateDeltaV1 {
    pub pass_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_wall_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_total_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_reasoning_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_function_calls: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_function_call_errors: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_trace_count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_span_count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_error_span_count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub median_trace_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalReportV1 {
    pub schema_version: String,
    pub evaluation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_evaluation_id: Option<String>,
    pub execution_order_policy: ExecutionOrderV1,
    pub effective_execution_order: Vec<String>,
    pub dimension: ComparisonDimensionV1,
    pub model: EvalModelConfigV1,
    pub limits: EvalLimitsV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<EvaluatorArtifactV1>,
    pub control: VariantArtifactV1,
    pub treatment: VariantArtifactV1,
    pub shared_artifacts: SharedArtifactsV1,
    pub control_aggregate: VariantAggregateV1,
    pub treatment_aggregate: VariantAggregateV1,
    /// Treatment minus control. Negative efficiency deltas are improvements.
    pub delta: AggregateDeltaV1,
    pub order_sensitivity: OrderSensitivityV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligible: Option<bool>,
    pub runs: Vec<EvalRunReportV1>,
    pub created_at: i64,
    pub completed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalProgressV1 {
    pub effective_execution_order: Vec<String>,
    pub control_aggregate: VariantAggregateV1,
    pub treatment_aggregate: VariantAggregateV1,
    pub delta: AggregateDeltaV1,
    pub order_sensitivity: OrderSensitivityV1,
    pub runs: Vec<EvalRunReportV1>,
}

pub fn build_progress(runs: &[EvalRunReportV1]) -> EvalProgressV1 {
    let control_aggregate = aggregate(runs, VariantRoleV1::Control);
    let treatment_aggregate = aggregate(runs, VariantRoleV1::Treatment);
    EvalProgressV1 {
        effective_execution_order: effective_order(runs),
        delta: delta(&control_aggregate, &treatment_aggregate),
        order_sensitivity: order_sensitivity(runs),
        control_aggregate,
        treatment_aggregate,
        runs: runs.to_vec(),
    }
}

pub fn build_report(
    evaluation_id: &str,
    request: &NormalizedEvalRequestV1,
    runs: Vec<EvalRunReportV1>,
    created_at: i64,
    completed_at: i64,
) -> EvalReportV1 {
    let control_aggregate = aggregate(&runs, VariantRoleV1::Control);
    let treatment_aggregate = aggregate(&runs, VariantRoleV1::Treatment);
    let eligible = request.evaluator.as_ref().map(|_| {
        treatment_aggregate.runs > 0
            && treatment_aggregate.evaluated_runs == treatment_aggregate.runs
            && treatment_aggregate.passed == treatment_aggregate.evaluated_runs
            && treatment_aggregate.passed >= control_aggregate.passed
    });
    let delta = delta(&control_aggregate, &treatment_aggregate);

    EvalReportV1 {
        schema_version: "1".into(),
        evaluation_id: evaluation_id.into(),
        source_evaluation_id: request.source_evaluation_id.clone(),
        execution_order_policy: request.execution_order,
        effective_execution_order: effective_order(&runs),
        dimension: request.dimension,
        model: request.model.clone(),
        limits: request.limits,
        evaluator: request
            .evaluator
            .as_ref()
            .map(|evaluator| EvaluatorArtifactV1 {
                function_id: evaluator.function_id.clone(),
                arguments_sha256: ids::sha256_json(&evaluator.arguments),
            }),
        control: artifact(&request.control),
        treatment: artifact(&request.treatment),
        shared_artifacts: SharedArtifactsV1 {
            model_sha256: ids::sha256_json(
                &serde_json::to_value(&request.model).unwrap_or_default(),
            ),
            function_policy_sha256: ids::sha256_json(
                &serde_json::to_value(&request.functions).unwrap_or_default(),
            ),
            limits_sha256: ids::sha256_json(
                &serde_json::to_value(request.limits).unwrap_or_default(),
            ),
            output_sha256: ids::sha256_json(
                &serde_json::to_value(&request.output).unwrap_or_default(),
            ),
        },
        control_aggregate,
        treatment_aggregate,
        delta,
        order_sensitivity: order_sensitivity(&runs),
        eligible,
        runs,
        created_at,
        completed_at,
    }
}

pub fn aggregate(runs: &[EvalRunReportV1], role: VariantRoleV1) -> VariantAggregateV1 {
    let selected: Vec<_> = runs
        .iter()
        .filter(|run| run.role == role && run.status.is_terminal())
        .collect();
    let benchmarks: Vec<_> = selected
        .iter()
        .filter_map(|run| run.benchmark.as_ref())
        .collect();
    let run_count = selected.len() as u32;
    let evaluated_runs = selected.iter().filter(|run| run.passed.is_some()).count() as u32;
    let passed = selected
        .iter()
        .filter(|run| run.passed == Some(true))
        .count() as u32;
    VariantAggregateV1 {
        runs: run_count,
        evaluated_runs,
        passed,
        pass_rate: if evaluated_runs == 0 {
            0.0
        } else {
            passed as f64 / evaluated_runs as f64
        },
        benchmarked_runs: benchmarks.len() as u32,
        median_score: median_f64(
            selected
                .iter()
                .filter_map(|run| run.evaluation.as_ref()?.score),
        ),
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
    }
}

fn effective_order(runs: &[EvalRunReportV1]) -> Vec<String> {
    runs.iter()
        .map(|run| {
            format!(
                "{}:{}:{}",
                run.execution_position,
                run.role.as_str(),
                run.iteration
            )
        })
        .collect()
}

fn order_sensitivity(runs: &[EvalRunReportV1]) -> OrderSensitivityV1 {
    fn role(runs: &[EvalRunReportV1], role: VariantRoleV1) -> RoleOrderSensitivityV1 {
        let evaluated: Vec<_> = runs
            .iter()
            .filter(|run| run.role == role && run.passed.is_some())
            .collect();
        let first: Vec<_> = evaluated
            .iter()
            .filter(|run| run.pair_position == 1)
            .collect();
        let second: Vec<_> = evaluated
            .iter()
            .filter(|run| run.pair_position == 2)
            .collect();
        let first_passed = first.iter().filter(|run| run.passed == Some(true)).count() as u32;
        let second_passed = second.iter().filter(|run| run.passed == Some(true)).count() as u32;
        let differs = !first.is_empty()
            && !second.is_empty()
            && first_passed as u64 * second.len() as u64
                != second_passed as u64 * first.len() as u64;
        RoleOrderSensitivityV1 {
            first_runs: first.len() as u32,
            first_passed,
            second_runs: second.len() as u32,
            second_passed,
            differs,
        }
    }
    let control = role(runs, VariantRoleV1::Control);
    let treatment = role(runs, VariantRoleV1::Treatment);
    OrderSensitivityV1 {
        detected: control.differs || treatment.differs,
        control,
        treatment,
    }
}

fn artifact(variant: &crate::contract::EvalVariantV1) -> VariantArtifactV1 {
    VariantArtifactV1 {
        label: variant.label.clone(),
        prompt_sha256: ids::sha256_text(&variant.prompt),
        system_prompt_sha256: variant.system_prompt.as_deref().map(ids::sha256_text),
    }
}

fn delta(control: &VariantAggregateV1, treatment: &VariantAggregateV1) -> AggregateDeltaV1 {
    AggregateDeltaV1 {
        pass_rate: treatment.pass_rate - control.pass_rate,
        median_score: difference_f64(control.median_score, treatment.median_score),
        median_wall_time_ms: difference_u64(
            control.median_wall_time_ms,
            treatment.median_wall_time_ms,
        ),
        median_total_tokens: difference_u64(
            control.median_total_tokens,
            treatment.median_total_tokens,
        ),
        median_reasoning_tokens: difference_u64(
            control.median_reasoning_tokens,
            treatment.median_reasoning_tokens,
        ),
        median_function_calls: difference_u64(
            control.median_function_calls,
            treatment.median_function_calls,
        ),
        median_function_call_errors: difference_u64(
            control.median_function_call_errors,
            treatment.median_function_call_errors,
        ),
        median_cost_usd: difference_f64(control.median_cost_usd, treatment.median_cost_usd),
        median_trace_count: difference_u64(
            control.median_trace_count,
            treatment.median_trace_count,
        ),
        median_span_count: difference_u64(control.median_span_count, treatment.median_span_count),
        median_error_span_count: difference_u64(
            control.median_error_span_count,
            treatment.median_error_span_count,
        ),
        median_trace_duration_ms: difference_u64(
            control.median_trace_duration_ms,
            treatment.median_trace_duration_ms,
        ),
    }
}

fn difference_u64(control: Option<u64>, treatment: Option<u64>) -> Option<f64> {
    control
        .zip(treatment)
        .map(|(control, treatment)| treatment as f64 - control as f64)
}

fn difference_f64(control: Option<f64>, treatment: Option<f64>) -> Option<f64> {
    control.zip(treatment).and_then(|(control, treatment)| {
        let delta = treatment - control;
        delta.is_finite().then_some(delta)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ComparisonDimensionV1, EvalModelConfigV1, EvalVariantV1, EvaluatorResponseV1,
        EvaluatorSpecV1,
    };
    use harness::prompt::SystemPromptStrategy;
    use harness::types::output::OutputContract;
    use harness::types::turn::FunctionPolicy;
    use serde_json::json;

    fn request(runs: u32) -> NormalizedEvalRequestV1 {
        NormalizedEvalRequestV1 {
            dimension: ComparisonDimensionV1::Prompt,
            model: EvalModelConfigV1 {
                model: "model".into(),
                provider: Some("provider".into()),
                system_prompt_strategy: SystemPromptStrategy::Override,
                mode: None,
                thinking_level: None,
                provider_options: None,
            },
            control: EvalVariantV1 {
                label: None,
                prompt: "old".into(),
                system_prompt: Some("system".into()),
            },
            treatment: EvalVariantV1 {
                label: None,
                prompt: "new".into(),
                system_prompt: Some("system".into()),
            },
            evaluator: Some(EvaluatorSpecV1 {
                function_id: "judge".into(),
                arguments: json!({}),
            }),
            runs,
            execution_order: ExecutionOrderV1::default(),
            source_evaluation_id: None,
            limits: EvalLimitsV1::default(),
            functions: FunctionPolicy::default(),
            output: OutputContract::Text,
            metadata: None,
        }
    }

    fn run(role: VariantRoleV1, iteration: u32, passed: bool, tokens: u64) -> EvalRunReportV1 {
        EvalRunReportV1 {
            run_id: format!("{}-{iteration}", role.as_str()),
            role,
            iteration,
            execution_position: 1,
            pair_position: 1,
            status: EvalRunStatusV1::Completed,
            session_id: "session".into(),
            turn_id: Some("turn".into()),
            passed: Some(passed),
            started_at: 1,
            completed_at: Some(2),
            output: Some(json!("OK")),
            metrics: None,
            evaluation: Some(EvaluatorResponseV1 {
                passed,
                score: Some(if passed { 1.0 } else { 0.0 }),
                reason: None,
                details: None,
            }),
            benchmark: Some(EvalBenchmarkV1 {
                wall_time_ms: tokens,
                sessions: 1,
                turns: 1,
                function_calls: 0,
                function_call_errors: 0,
                input_tokens: Some(tokens),
                output_tokens: Some(1),
                total_tokens: Some(tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
                cost_usd: None,
                trace_count: None,
                span_count: None,
                error_span_count: None,
                trace_duration_ms: None,
            }),
            failures: Vec::new(),
        }
    }

    #[test]
    fn medians_and_treatment_minus_control_deltas_are_reported() {
        let runs = vec![
            run(VariantRoleV1::Control, 1, true, 10),
            run(VariantRoleV1::Treatment, 1, true, 8),
            run(VariantRoleV1::Control, 2, true, 20),
            run(VariantRoleV1::Treatment, 2, true, 12),
            run(VariantRoleV1::Control, 3, true, 30),
            run(VariantRoleV1::Treatment, 3, true, 16),
        ];
        let report = build_report("eval-1", &request(3), runs, 1, 2);
        assert_eq!(report.eligible, Some(true));
        assert_eq!(report.control_aggregate.median_total_tokens, Some(20));
        assert_eq!(report.treatment_aggregate.median_total_tokens, Some(12));
        assert_eq!(report.delta.median_total_tokens, Some(-8.0));
    }

    #[test]
    fn treatment_must_pass_every_run() {
        let runs = vec![
            run(VariantRoleV1::Control, 1, true, 10),
            run(VariantRoleV1::Treatment, 1, true, 10),
            run(VariantRoleV1::Control, 2, false, 10),
            run(VariantRoleV1::Treatment, 2, false, 10),
        ];
        assert_eq!(
            build_report("eval-1", &request(2), runs, 1, 2).eligible,
            Some(false)
        );
    }

    #[test]
    fn null_outputs_are_not_included_in_pass_rate() {
        let mut control = run(VariantRoleV1::Control, 1, false, 10);
        control.output = Some(Value::Null);
        control.passed = None;
        control.evaluation = None;
        let mut treatment = run(VariantRoleV1::Treatment, 1, false, 10);
        treatment.output = Some(Value::Null);
        treatment.passed = None;
        treatment.evaluation = None;

        let report = build_report("eval-1", &request(1), vec![control, treatment], 1, 2);
        assert_eq!(report.control_aggregate.evaluated_runs, 0);
        assert_eq!(report.treatment_aggregate.evaluated_runs, 0);
        assert_eq!(report.control_aggregate.passed, 0);
        assert_eq!(report.eligible, Some(false));
    }

    #[test]
    fn report_without_evaluator_has_no_eligibility_decision() {
        let mut request = request(1);
        request.evaluator = None;
        let report = build_report(
            "eval-1",
            &request,
            vec![
                run(VariantRoleV1::Control, 1, false, 10),
                run(VariantRoleV1::Treatment, 1, false, 10),
            ],
            1,
            2,
        );
        assert_eq!(report.evaluator, None);
        assert_eq!(report.eligible, None);
    }

    #[test]
    fn progress_uses_completed_runs_and_detects_order_sensitivity() {
        let mut control_first = run(VariantRoleV1::Control, 1, true, 10);
        control_first.pair_position = 1;
        let mut control_second = run(VariantRoleV1::Control, 2, false, 20);
        control_second.pair_position = 2;
        let mut pending = run(VariantRoleV1::Treatment, 2, true, 30);
        pending.status = EvalRunStatusV1::Running;
        pending.passed = None;

        let progress = build_progress(&[control_first, control_second, pending]);
        assert_eq!(progress.control_aggregate.runs, 2);
        assert_eq!(progress.treatment_aggregate.runs, 0);
        assert!(progress.order_sensitivity.detected);
        assert!(progress.order_sensitivity.control.differs);
    }
}
