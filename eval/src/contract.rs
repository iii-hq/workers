use std::collections::BTreeMap;

use harness::functions::metrics::SessionMetricsResponseV1;
use harness::prompt::{Mode, SystemPromptStrategy};
use harness::types::model::ThinkingLevel;
use harness::types::output::OutputContract;
use harness::types::turn::FunctionPolicy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::EvalError;
use crate::limits::EvalLimitsV1;
use crate::report::EvalReportV1;

pub const DEFAULT_RUNS: u32 = 1;
pub const MAX_RUNS: u32 = 20;
pub const DEFAULT_LIST_LIMIT: u32 = 50;
pub const MAX_LIST_LIMIT: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonDimensionV1 {
    Prompt,
    SystemPrompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariantRoleV1 {
    Control,
    Treatment,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOrderV1 {
    #[default]
    BalancedControlFirst,
    BalancedTreatmentFirst,
}

impl ExecutionOrderV1 {
    pub fn roles(self, iteration: u32) -> [VariantRoleV1; 2] {
        let control_first = match self {
            Self::BalancedControlFirst => iteration % 2 == 1,
            Self::BalancedTreatmentFirst => iteration.is_multiple_of(2),
        };
        if control_first {
            [VariantRoleV1::Control, VariantRoleV1::Treatment]
        } else {
            [VariantRoleV1::Treatment, VariantRoleV1::Control]
        }
    }

    pub fn reversed(self) -> Self {
        match self {
            Self::BalancedControlFirst => Self::BalancedTreatmentFirst,
            Self::BalancedTreatmentFirst => Self::BalancedControlFirst,
        }
    }
}

impl VariantRoleV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Treatment => "treatment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalVariantV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub prompt: String,
    /// `null` disables the system prompt for this variant. An empty string
    /// keeps the harness behavior of resolving the provider or built-in prompt.
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalModelConfigV1 {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorSpecV1 {
    pub function_id: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvalStartRequestV1 {
    pub dimension: ComparisonDimensionV1,
    pub model: EvalModelConfigV1,
    pub control: EvalVariantV1,
    pub treatment: EvalVariantV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<EvaluatorSpecV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_order: Option<ExecutionOrderV1>,
    #[serde(default)]
    pub limits: EvalLimitsV1,
    /// Shared function policy for both variants. Omitted means deny all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// Shared output contract for both variants. Omitted means text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputContract>,
    /// Shared harness metadata, including an optional filesystem scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizedEvalRequestV1 {
    pub dimension: ComparisonDimensionV1,
    pub model: EvalModelConfigV1,
    pub control: EvalVariantV1,
    pub treatment: EvalVariantV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<EvaluatorSpecV1>,
    pub runs: u32,
    #[serde(default)]
    pub execution_order: ExecutionOrderV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_evaluation_id: Option<String>,
    pub limits: EvalLimitsV1,
    pub functions: FunctionPolicy,
    pub output: OutputContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl EvalStartRequestV1 {
    pub fn normalize(self) -> Result<NormalizedEvalRequestV1, EvalError> {
        validate_non_empty("model.model", &self.model.model)?;
        if self
            .model
            .provider
            .as_deref()
            .is_some_and(|provider| provider.trim().is_empty())
        {
            return Err(EvalError::InvalidRequest(
                "model.provider cannot be empty when supplied".into(),
            ));
        }
        validate_non_empty("control.prompt", &self.control.prompt)?;
        validate_non_empty("treatment.prompt", &self.treatment.prompt)?;
        if let Some(evaluator) = &self.evaluator {
            validate_non_empty("evaluator.function_id", &evaluator.function_id)?;
        }
        if self
            .control
            .label
            .as_deref()
            .is_some_and(|label| label.trim().is_empty())
            || self
                .treatment
                .label
                .as_deref()
                .is_some_and(|label| label.trim().is_empty())
        {
            return Err(EvalError::InvalidRequest(
                "variant labels cannot be empty when supplied".into(),
            ));
        }

        match self.dimension {
            ComparisonDimensionV1::Prompt => {
                if self.control.system_prompt != self.treatment.system_prompt {
                    return Err(EvalError::InvalidRequest(
                        "prompt comparison requires identical system prompts".into(),
                    ));
                }
                if self.control.prompt == self.treatment.prompt {
                    return Err(EvalError::InvalidRequest(
                        "prompt comparison requires different prompts".into(),
                    ));
                }
            }
            ComparisonDimensionV1::SystemPrompt => {
                if self.control.prompt != self.treatment.prompt {
                    return Err(EvalError::InvalidRequest(
                        "system_prompt comparison requires identical prompts".into(),
                    ));
                }
                if self.control.system_prompt == self.treatment.system_prompt {
                    return Err(EvalError::InvalidRequest(
                        "system_prompt comparison requires different system prompts".into(),
                    ));
                }
            }
        }

        let runs = self.runs.unwrap_or(DEFAULT_RUNS);
        if !(1..=MAX_RUNS).contains(&runs) {
            return Err(EvalError::InvalidRequest(format!(
                "runs must be between 1 and {MAX_RUNS}"
            )));
        }
        self.limits.validate()?;

        Ok(NormalizedEvalRequestV1 {
            dimension: self.dimension,
            model: self.model,
            control: self.control,
            treatment: self.treatment,
            evaluator: self.evaluator,
            runs,
            execution_order: self.execution_order.unwrap_or_default(),
            source_evaluation_id: None,
            limits: self.limits,
            functions: self.functions.unwrap_or_default(),
            output: self.output.unwrap_or_default(),
            metadata: self.metadata,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatusV1 {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl EvalStatusV1 {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvalRunStatusV1 {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl EvalRunStatusV1 {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalStartResponseV1 {
    pub evaluation_id: String,
    pub status: EvalStatusV1,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct EvalListRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl EvalListRequestV1 {
    pub fn normalized_limit(&self) -> Result<usize, EvalError> {
        let limit = self.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        if !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(EvalError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_LIST_LIMIT}"
            )));
        }
        Ok(limit as usize)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalSummaryV1 {
    pub evaluation_id: String,
    pub status: EvalStatusV1,
    pub dimension: ComparisonDimensionV1,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treatment_label: Option<String>,
    pub total_runs: u32,
    pub terminal_runs: u32,
    pub passed_runs: u32,
    pub failed_runs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligible: Option<bool>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalListResponseV1 {
    pub evaluations: Vec<EvalSummaryV1>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EvaluationIdRequestV1 {
    pub evaluation_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EvalRerunRequestV1 {
    pub evaluation_id: String,
    #[serde(default)]
    pub reverse_order: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActiveRunV1 {
    pub run_id: String,
    pub role: VariantRoleV1,
    pub iteration: u32,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalStatusResponseV1 {
    pub evaluation_id: String,
    pub status: EvalStatusV1,
    pub total_runs: u32,
    pub terminal_runs: u32,
    pub passed_runs: u32,
    pub failed_runs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<ActiveRunV1>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalResultResponseV1 {
    pub status: EvalStatusV1,
    pub request: NormalizedEvalRequestV1,
    pub progress: crate::report::EvalProgressV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<EvalReportV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalCancelResponseV1 {
    pub cancelled: bool,
    pub status: EvalStatusV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalDeleteResponseV1 {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvaluatorInputV1 {
    pub evaluation_id: String,
    pub run_id: String,
    pub role: VariantRoleV1,
    pub session_id: String,
    pub output: Value,
    pub metrics: SessionMetricsResponseV1,
    #[serde(default = "empty_object")]
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorResponseV1 {
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl EvaluatorResponseV1 {
    pub fn validate(self) -> Result<Self, EvalError> {
        if self
            .score
            .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
        {
            return Err(EvalError::InvalidRequest(
                "evaluator score must be a finite number between 0 and 1".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepRequestV1 {
    pub evaluation_id: String,
    pub step: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepResponseV1 {
    pub skipped: bool,
    pub status: EvalStatusV1,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct WakeEventV1 {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WakeResponseV1 {
    pub woke: bool,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct SweepEventV1 {
    #[serde(default)]
    pub scheduled_at: Option<i64>,
    #[serde(default)]
    pub scheduled_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SweepResponseV1 {
    pub swept: u64,
}

fn validate_non_empty(name: &str, value: &str) -> Result<(), EvalError> {
    if value.trim().is_empty() {
        Err(EvalError::InvalidRequest(format!("{name} cannot be empty")))
    } else {
        Ok(())
    }
}

fn empty_object() -> Value {
    json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(dimension: ComparisonDimensionV1) -> EvalStartRequestV1 {
        EvalStartRequestV1 {
            dimension,
            model: EvalModelConfigV1 {
                model: "model".into(),
                provider: Some("provider".into()),
                system_prompt_strategy: SystemPromptStrategy::Override,
                mode: Some(Mode::Agent),
                thinking_level: None,
                provider_options: None,
            },
            control: EvalVariantV1 {
                label: None,
                prompt: "control".into(),
                system_prompt: Some("system".into()),
            },
            treatment: EvalVariantV1 {
                label: None,
                prompt: "treatment".into(),
                system_prompt: Some("system".into()),
            },
            evaluator: Some(EvaluatorSpecV1 {
                function_id: "eval::assert::exact".into(),
                arguments: json!({"expected": "OK"}),
            }),
            runs: None,
            execution_order: None,
            limits: EvalLimitsV1::default(),
            functions: None,
            output: None,
            metadata: None,
        }
    }

    #[test]
    fn list_limit_defaults_and_stays_bounded() {
        assert_eq!(
            EvalListRequestV1::default().normalized_limit().unwrap(),
            DEFAULT_LIST_LIMIT as usize
        );
        assert_eq!(
            EvalListRequestV1 { limit: Some(1) }
                .normalized_limit()
                .unwrap(),
            1
        );
        assert!(EvalListRequestV1 { limit: Some(0) }
            .normalized_limit()
            .is_err());
        assert!(EvalListRequestV1 {
            limit: Some(MAX_LIST_LIMIT + 1)
        }
        .normalized_limit()
        .is_err());
    }

    #[test]
    fn list_request_accepts_engine_metadata() {
        let request: EvalListRequestV1 =
            serde_json::from_value(json!({"limit": 5, "_caller_worker_id": "console"})).unwrap();
        assert_eq!(request.limit, Some(5));
    }

    #[test]
    fn prompt_comparison_changes_only_prompt_and_defaults_to_one_run() {
        let normalized = request(ComparisonDimensionV1::Prompt).normalize().unwrap();
        assert_eq!(normalized.runs, 1);
        assert_eq!(normalized.functions, FunctionPolicy::default());
        assert_eq!(normalized.output, OutputContract::Text);
    }

    #[test]
    fn system_prompt_comparison_changes_only_system_prompt() {
        let mut request = request(ComparisonDimensionV1::SystemPrompt);
        request.treatment.prompt = request.control.prompt.clone();
        request.treatment.system_prompt = Some("new system".into());
        assert!(request.normalize().is_ok());
    }

    #[test]
    fn system_prompt_comparison_accepts_default_vs_disabled() {
        let mut request = request(ComparisonDimensionV1::SystemPrompt);
        request.treatment.prompt = request.control.prompt.clone();
        request.control.system_prompt = Some(String::new());
        request.treatment.system_prompt = None;
        assert!(request.normalize().is_ok());
    }

    #[test]
    fn rejects_identical_or_two_dimension_changes() {
        let mut identical = request(ComparisonDimensionV1::Prompt);
        identical.treatment.prompt = identical.control.prompt.clone();
        assert!(identical.normalize().is_err());

        let mut both = request(ComparisonDimensionV1::Prompt);
        both.treatment.system_prompt = Some("different".into());
        assert!(both.normalize().is_err());
    }

    #[test]
    fn runs_are_bounded() {
        let mut zero = request(ComparisonDimensionV1::Prompt);
        zero.runs = Some(0);
        assert!(zero.normalize().is_err());
        let mut too_many = request(ComparisonDimensionV1::Prompt);
        too_many.runs = Some(MAX_RUNS + 1);
        assert!(too_many.normalize().is_err());
    }

    #[test]
    fn evaluator_score_is_bounded() {
        assert!(EvaluatorResponseV1 {
            passed: true,
            score: Some(1.0),
            reason: None,
            details: None,
        }
        .validate()
        .is_ok());
        assert!(EvaluatorResponseV1 {
            passed: true,
            score: Some(1.1),
            reason: None,
            details: None,
        }
        .validate()
        .is_err());
    }
}
