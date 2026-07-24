use std::time::Duration;

use harness::functions::metrics::SessionMetricsResponseV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{EvalError, EvalFailureV1, EvalPhaseV1};

pub const DEFAULT_INVOCATION_TIMEOUT_SECONDS: u64 = 120;
pub const DEFAULT_SCENARIO_TIMEOUT_SECONDS: u64 = 600;
pub const DEFAULT_MAX_TURNS: u32 = 1_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS_PER_CALL: u64 = 8_192;
pub const AGENT_QUALITY_DEFAULT_MAX_TOTAL_TOKENS: u64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalLimitsV1 {
    #[serde(default)]
    pub execution: ExecutionLimitsV1,
    #[serde(default)]
    pub evaluation: EvaluationLimitsV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimitsV1 {
    #[serde(default = "default_invocation_timeout_seconds")]
    pub invocation_timeout_seconds: u64,
    #[serde(default = "default_scenario_timeout_seconds")]
    pub scenario_timeout_seconds: u64,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_output_tokens_per_call")]
    pub max_output_tokens_per_call: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluationLimitsV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_function_call_errors: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_error_spans: Option<u64>,
}

impl Default for EvalLimitsV1 {
    fn default() -> Self {
        Self {
            execution: ExecutionLimitsV1::default(),
            evaluation: EvaluationLimitsV1::default(),
        }
    }
}

impl Default for ExecutionLimitsV1 {
    fn default() -> Self {
        Self {
            invocation_timeout_seconds: default_invocation_timeout_seconds(),
            scenario_timeout_seconds: default_scenario_timeout_seconds(),
            max_turns: default_max_turns(),
            max_output_tokens_per_call: default_max_output_tokens_per_call(),
            max_total_tokens: None,
            max_cost_usd: None,
        }
    }
}

impl Default for EvaluationLimitsV1 {
    fn default() -> Self {
        Self {
            max_function_call_errors: None,
            max_error_spans: None,
        }
    }
}

fn default_invocation_timeout_seconds() -> u64 {
    DEFAULT_INVOCATION_TIMEOUT_SECONDS
}

fn default_scenario_timeout_seconds() -> u64 {
    DEFAULT_SCENARIO_TIMEOUT_SECONDS
}

fn default_max_turns() -> u32 {
    DEFAULT_MAX_TURNS
}

fn default_max_output_tokens_per_call() -> u64 {
    DEFAULT_MAX_OUTPUT_TOKENS_PER_CALL
}

impl EvalLimitsV1 {
    pub fn validate(self) -> Result<(), EvalError> {
        for (name, value) in [
            (
                "limits.execution.invocation_timeout_seconds",
                self.execution.invocation_timeout_seconds,
            ),
            (
                "limits.execution.scenario_timeout_seconds",
                self.execution.scenario_timeout_seconds,
            ),
            (
                "limits.execution.max_output_tokens_per_call",
                self.execution.max_output_tokens_per_call,
            ),
        ] {
            if value == 0 {
                return Err(EvalError::InvalidRequest(format!(
                    "{name} must be greater than zero"
                )));
            }
        }
        if self.execution.max_turns == 0 {
            return Err(EvalError::InvalidRequest(
                "limits.execution.max_turns must be greater than zero".into(),
            ));
        }
        if self.execution.max_total_tokens == Some(0) {
            return Err(EvalError::InvalidRequest(
                "limits.execution.max_total_tokens must be greater than zero when supplied".into(),
            ));
        }
        if self
            .execution
            .max_cost_usd
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(EvalError::InvalidRequest(
                "limits.execution.max_cost_usd must be a finite non-negative number".into(),
            ));
        }
        Ok(())
    }

    pub fn failures(self, metrics: &SessionMetricsResponseV1) -> Vec<EvalFailureV1> {
        let mut failures = Vec::new();
        if let Some(limit) = self.execution.max_total_tokens {
            match (metrics.totals.input_tokens, metrics.totals.output_tokens) {
                (Some(input), Some(output)) => {
                    let total = input.saturating_add(output);
                    if total > limit {
                        failures.push(EvalFailureV1::new(
                            EvalPhaseV1::Limit,
                            format!("total tokens {total} exceeded limit {limit}"),
                        ));
                    }
                }
                _ => failures.push(EvalFailureV1::function(
                    EvalPhaseV1::Collect,
                    "harness::metrics",
                    "total-token validation requires input_tokens and output_tokens",
                )),
            }
        }

        if let Some(limit) = self.execution.max_cost_usd {
            match metrics.totals.cost_usd {
                Some(cost) if cost > limit => failures.push(EvalFailureV1::new(
                    EvalPhaseV1::Limit,
                    format!("cost ${cost:.6} exceeded limit ${limit:.6}"),
                )),
                Some(_) => {}
                None => failures.push(EvalFailureV1::function(
                    EvalPhaseV1::Collect,
                    "harness::metrics",
                    "cost validation requires cost_usd from every model generation",
                )),
            }
        }

        if let Some(limit) = self.evaluation.max_function_call_errors {
            if metrics.totals.function_call_errors > limit {
                failures.push(EvalFailureV1::new(
                    EvalPhaseV1::Limit,
                    format!(
                        "function call errors {} exceeded limit {limit}",
                        metrics.totals.function_call_errors
                    ),
                ));
            }
        }

        if let Some(limit) = self.evaluation.max_error_spans {
            match metrics.traces.as_ref() {
                Some(traces) if traces.error_span_count > limit => {
                    failures.push(EvalFailureV1::new(
                        EvalPhaseV1::Limit,
                        format!(
                            "error spans {} exceeded limit {limit}",
                            traces.error_span_count
                        ),
                    ));
                }
                Some(_) => {}
                None => failures.push(EvalFailureV1::function(
                    EvalPhaseV1::Collect,
                    "harness::metrics",
                    "error-span validation requires in-memory observability metrics",
                )),
            }
        }
        failures
    }
}

impl ExecutionLimitsV1 {
    pub fn invocation_timeout(self) -> Duration {
        Duration::from_secs(self.invocation_timeout_seconds)
    }

    pub fn scenario_timeout(self) -> Duration {
        Duration::from_secs(self.scenario_timeout_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::functions::metrics::SessionUsageTotalsV1;

    #[test]
    fn defaults_leave_optional_tree_budgets_unset() {
        let limits = EvalLimitsV1::default();
        assert_eq!(limits.execution.max_turns, 1_000);
        assert_eq!(limits.execution.max_output_tokens_per_call, 8_192);
        assert_eq!(limits.execution.max_total_tokens, None);
        assert_eq!(limits.evaluation.max_function_call_errors, None);
        limits.validate().unwrap();
    }

    #[test]
    fn partial_limits_resolve_operational_defaults() {
        let limits: EvalLimitsV1 =
            serde_json::from_value(serde_json::json!({"execution": {}, "evaluation": {}})).unwrap();

        assert_eq!(limits, EvalLimitsV1::default());
    }

    #[test]
    fn function_call_errors_are_only_limited_when_explicitly_configured() {
        let metrics = SessionMetricsResponseV1 {
            root_session_id: "session".into(),
            complete: true,
            totals: SessionUsageTotalsV1 {
                function_call_errors: 3,
                ..Default::default()
            },
            by_session: Vec::new(),
            traces: None,
        };

        assert!(EvalLimitsV1::default().failures(&metrics).is_empty());

        let mut limits = EvalLimitsV1::default();
        limits.evaluation.max_function_call_errors = Some(0);
        assert_eq!(limits.failures(&metrics).len(), 1);
    }

    #[test]
    fn validation_rejects_zero_and_invalid_cost() {
        let mut limits = EvalLimitsV1::default();
        limits.execution.scenario_timeout_seconds = 0;
        assert!(limits.validate().is_err());

        let mut limits = EvalLimitsV1::default();
        limits.execution.max_total_tokens = Some(0);
        assert!(limits.validate().is_err());

        let mut limits = EvalLimitsV1::default();
        limits.execution.max_cost_usd = Some(f64::NAN);
        assert!(limits.validate().is_err());
    }
}
