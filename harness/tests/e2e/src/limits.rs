use clap::Args;
use harness::functions::metrics::SessionMetricsResponseV1;

use crate::error::EvalError;

pub use eval::limits::{
    EvalLimitsV1 as E2eLimitsV1, EvaluationLimitsV1, ExecutionLimitsV1,
    DEFAULT_INVOCATION_TIMEOUT_SECONDS, DEFAULT_MAX_OUTPUT_TOKENS_PER_CALL,
};

pub const DEFAULT_SCENARIO_TIMEOUT_SECONDS: u64 = 300;
pub const DEFAULT_MAX_TURNS: u32 = 20;
pub const E2E_DEFAULT_MAX_TOTAL_TOKENS: u64 = 100_000;

#[derive(Debug, Clone, Args)]
pub struct LimitsArgs {
    /// Timeout for one iii function invocation.
    #[arg(long, default_value_t = DEFAULT_INVOCATION_TIMEOUT_SECONDS)]
    invocation_timeout_seconds: u64,

    /// Hard wall-clock limit for one scenario; expiry calls harness::stop.
    #[arg(long, default_value_t = DEFAULT_SCENARIO_TIMEOUT_SECONDS)]
    scenario_timeout_seconds: u64,

    /// Maximum model generation steps in a harness turn.
    #[arg(long, default_value_t = DEFAULT_MAX_TURNS)]
    max_turns: u32,

    /// Maximum output tokens forwarded to each model call.
    #[arg(long, default_value_t = DEFAULT_MAX_OUTPUT_TOKENS_PER_CALL)]
    max_output_tokens_per_call: u64,

    /// Hard input-plus-output budget over the complete session tree.
    #[arg(long, default_value_t = E2E_DEFAULT_MAX_TOTAL_TOKENS)]
    max_total_tokens: u64,

    /// Optional hard cost budget. Requires model catalog pricing.
    #[arg(long)]
    max_cost_usd: Option<f64>,

    /// Post-execution ceiling for failed function calls.
    #[arg(long, default_value_t = 0)]
    max_function_call_errors: u64,

    /// Optional post-execution ceiling for error spans. Requires in-memory traces.
    #[arg(long)]
    max_error_spans: Option<u64>,
}

impl LimitsArgs {
    pub fn resolve(&self) -> Result<E2eLimitsV1, EvalError> {
        let limits = E2eLimitsV1 {
            execution: ExecutionLimitsV1 {
                invocation_timeout_seconds: self.invocation_timeout_seconds,
                scenario_timeout_seconds: self.scenario_timeout_seconds,
                max_turns: self.max_turns,
                max_output_tokens_per_call: self.max_output_tokens_per_call,
                max_total_tokens: Some(self.max_total_tokens),
                max_cost_usd: self.max_cost_usd,
            },
            evaluation: EvaluationLimitsV1 {
                max_function_call_errors: Some(self.max_function_call_errors),
                max_error_spans: self.max_error_spans,
            },
        };
        limits
            .validate()
            .map_err(|error| EvalError::setup(error.to_string()))?;
        Ok(limits)
    }
}

pub fn limit_failures(limits: E2eLimitsV1, metrics: &SessionMetricsResponseV1) -> Vec<EvalError> {
    limits
        .failures(metrics)
        .into_iter()
        .map(|failure| match failure.phase {
            eval::error::EvalPhaseV1::Collect => EvalError::evidence(
                failure.function_id.as_deref().unwrap_or("harness::metrics"),
                failure.message,
            ),
            _ => EvalError::limit(failure.message),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use harness::functions::metrics::{SessionTraceMetricsV1, SessionUsageTotalsV1};

    use super::*;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        limits: LimitsArgs,
    }

    fn metrics() -> SessionMetricsResponseV1 {
        SessionMetricsResponseV1 {
            root_session_id: "root".into(),
            complete: true,
            totals: SessionUsageTotalsV1 {
                input_tokens: Some(70),
                output_tokens: Some(20),
                cost_usd: Some(0.25),
                ..SessionUsageTotalsV1::default()
            },
            by_session: Vec::new(),
            traces: Some(SessionTraceMetricsV1 {
                trace_count: 1,
                span_count: 5,
                error_span_count: 0,
                duration_ms: 10,
                by_session: Vec::new(),
            }),
        }
    }

    #[test]
    fn defaults_define_both_execution_and_evaluation_limits() {
        let limits = TestCli::parse_from(["test"]).limits.resolve().unwrap();
        assert_eq!(limits.execution.max_turns, 20);
        assert_eq!(limits.execution.scenario_timeout_seconds, 300);
        assert_eq!(limits.execution.max_output_tokens_per_call, 8_192);
        assert_eq!(limits.execution.max_total_tokens, Some(100_000));
        assert_eq!(limits.evaluation.max_function_call_errors, Some(0));
    }

    #[test]
    fn evaluation_limits_accept_metrics_within_budget() {
        assert!(E2eLimitsV1::default().failures(&metrics()).is_empty());
    }

    #[test]
    fn evaluation_limits_report_each_exceeded_budget() {
        let mut value = metrics();
        value.totals.function_call_errors = 2;
        value.traces.as_mut().unwrap().error_span_count = 3;
        let limits = E2eLimitsV1 {
            execution: ExecutionLimitsV1 {
                max_total_tokens: Some(80),
                max_cost_usd: Some(0.2),
                ..E2eLimitsV1::default().execution
            },
            evaluation: EvaluationLimitsV1 {
                max_function_call_errors: Some(1),
                max_error_spans: Some(2),
            },
        };
        let messages: Vec<_> = limit_failures(limits, &value)
            .into_iter()
            .map(|error| error.to_string())
            .collect();
        assert_eq!(messages.len(), 4);
        assert!(messages.iter().any(|message| message.contains("tokens")));
        assert!(messages.iter().any(|message| message.contains("cost")));
        assert!(messages
            .iter()
            .any(|message| message.contains("function call errors")));
        assert!(messages
            .iter()
            .any(|message| message.contains("error spans")));
    }

    #[test]
    fn optional_limits_fail_closed_when_metrics_are_missing() {
        let mut value = metrics();
        value.totals.cost_usd = None;
        value.traces = None;
        let limits = E2eLimitsV1 {
            execution: ExecutionLimitsV1 {
                max_total_tokens: Some(100),
                max_cost_usd: Some(1.0),
                ..E2eLimitsV1::default().execution
            },
            evaluation: EvaluationLimitsV1 {
                max_function_call_errors: Some(0),
                max_error_spans: Some(0),
            },
        };
        let failures = limit_failures(limits, &value);
        assert_eq!(failures.len(), 2);
        assert!(failures
            .iter()
            .all(|error| error.to_string().contains("requires")));
    }
}
