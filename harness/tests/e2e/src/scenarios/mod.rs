use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use anyhow::{bail, Result};
use clap::ValueEnum;
use harness::functions::metrics::SessionMetricsResponseV1;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::E2eContext;
use crate::report::HardGateReport;

mod assessment;
pub mod common;
pub mod custom_validator;
pub mod design_tradeoff;
pub mod direct_answer;
pub mod mechanical_reaction;
pub mod multi_subagent_validation;
pub mod persistent_state;
pub mod reactive_automation;
pub mod receiving_operation;
pub mod research_pipeline;
pub mod security_review;
pub mod security_triage;
pub mod shell_coder_sandbox;
pub mod subagent_validation;
pub mod subagent_validation_failure;
pub mod timer_wake;
pub mod validation_chain;
pub mod validation_loop;
pub mod validation_scope_enforcement;
pub mod validation_self_repair;

/// Judge-backed scenarios keep a low semantic floor while exposing the full
/// score as a quality signal. Objective contract violations remain hard gates.
pub(super) const JUDGE_BACKED_PASS_THRESHOLD: u8 = 50;

pub type EvaluationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ObjectiveEvaluation>> + Send + 'a>>;
pub type CleanupFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
pub type ScenarioEvaluator =
    for<'a> fn(&'a E2eContext, &'a ScenarioObservation, &'a str) -> EvaluationFuture<'a>;
pub type ScenarioCleanup = for<'a> fn(&'a E2eContext, &'a str) -> CleanupFuture<'a>;
/// Pre-send hook: provision what the prompt refers to (e.g. register a
/// temporary validator function on the suite's own worker connection).
pub type ScenarioSetup = for<'a> fn(&'a E2eContext, &'a str) -> CleanupFuture<'a>;

#[derive(Debug, Clone)]
pub struct CriterionSpec {
    pub id: &'static str,
    pub weight: u8,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub max_turns: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    pub max_total_tokens: u64,
    /// Stop only after this many seconds without observable useful progress.
    /// Large scenarios have no fixed wall-clock deadline.
    pub stuck_timeout_seconds: u64,
}

impl ExecutionPolicy {
    fn validate(self, scenario_id: &str) -> Result<()> {
        if self.max_turns == 0 {
            bail!("scenario '{scenario_id}': execution.max_turns=0; expected at least 1");
        }
        if self.max_output_tokens == Some(0) {
            bail!(
                "scenario '{scenario_id}': execution.max_output_tokens=0; expected None (provider limit) or at least 1"
            );
        }
        if self.max_total_tokens == 0 {
            bail!("scenario '{scenario_id}': execution.max_total_tokens=0; expected at least 1");
        }
        if self.stuck_timeout_seconds == 0 {
            bail!(
                "scenario '{scenario_id}': execution.stuck_timeout_seconds=0; expected at least 1"
            );
        }
        if self
            .max_output_tokens
            .is_some_and(|max_output_tokens| self.max_total_tokens < max_output_tokens)
        {
            let max_output_tokens = self.max_output_tokens.expect("checked above");
            bail!(
                "scenario '{scenario_id}': execution.max_total_tokens={} is lower than execution.max_output_tokens={max_output_tokens}; expected max_total_tokens >= max_output_tokens",
                self.max_total_tokens
            );
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ScenarioSpec {
    pub id: &'static str,
    /// Increment when the scenario's behavioral contract changes. Structural
    /// refactors that preserve prompts, gates, criteria, and policy keep it.
    pub version: u32,
    pub prompt: String,
    pub filesystem_root: Option<PathBuf>,
    pub execution: ExecutionPolicy,
    pub denied_functions: &'static [&'static str],
    pub threshold: u8,
    pub criteria: Vec<CriterionSpec>,
    pub judge_reference: Option<Value>,
    /// Runs BEFORE the prompt is sent; a failure aborts the run.
    pub setup: Option<ScenarioSetup>,
    pub evaluate: ScenarioEvaluator,
    pub cleanup: Option<ScenarioCleanup>,
}

impl ScenarioSpec {
    pub fn validate(&self) -> Result<()> {
        if self.prompt.trim().is_empty() {
            bail!(
                "scenario '{}': prompt is empty after trimming; provide a non-empty task prompt",
                self.id
            );
        }
        if self.version == 0 {
            bail!("scenario '{}': version=0; expected version >= 1", self.id);
        }
        self.execution.validate(self.id)?;
        if !(1..=100).contains(&self.threshold) {
            bail!(
                "scenario '{}': threshold={}; expected a value in 1..=100",
                self.id,
                self.threshold
            );
        }
        if self.needs_judge() && self.threshold != JUDGE_BACKED_PASS_THRESHOLD {
            bail!(
                "scenario '{}': judge-backed threshold={}; expected {JUDGE_BACKED_PASS_THRESHOLD}; set this threshold or remove judge_reference",
                self.id, self.threshold
            );
        }
        let mut ids = HashMap::new();
        for (index, criterion) in self.criteria.iter().enumerate() {
            if criterion.id.trim().is_empty() {
                bail!(
                    "scenario '{}': criteria[{index}].id is empty after trimming; use a stable non-empty identifier",
                    self.id
                );
            }
            if criterion.weight == 0 {
                bail!(
                    "scenario '{}': criterion '{}' has weight=0; expected at least 1",
                    self.id,
                    criterion.id
                );
            }
            if let Some(first_index) = ids.insert(criterion.id, index) {
                bail!(
                    "scenario '{}': criterion id '{}' is duplicated at indexes {first_index} and {index}; criterion ids must be unique",
                    self.id, criterion.id
                );
            }
        }
        let total: u16 = self
            .criteria
            .iter()
            .map(|criterion| u16::from(criterion.weight))
            .sum();
        if total != 100 {
            let declared = self
                .criteria
                .iter()
                .map(|criterion| format!("{}={}", criterion.id, criterion.weight))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "scenario '{}': criterion weights total={total}; expected exactly 100; declared weights=[{declared}]",
                self.id
            );
        }
        Ok(())
    }

    pub fn needs_judge(&self) -> bool {
        self.judge_reference.is_some()
    }
}

pub struct ScenarioObservation {
    pub metrics: SessionMetricsResponseV1,
    pub transcript: Value,
    pub response: String,
}

pub struct ObjectiveEvaluation {
    pub hard_gates: Vec<HardGateReport>,
    pub awards: Vec<CriterionAward>,
}

pub struct CriterionAward {
    pub id: String,
    pub awarded: u8,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ScenarioId {
    #[value(name = "direct_answer")]
    DirectAnswer,
    #[value(name = "persistent_state")]
    PersistentState,
    #[value(name = "security_review")]
    SecurityReview,
    #[value(name = "reactive_automation")]
    ReactiveAutomation,
    #[value(name = "shell_coder_sandbox")]
    ShellCoderSandbox,
    #[value(name = "design_tradeoff")]
    DesignTradeoff,
    #[value(name = "security_triage")]
    SecurityTriage,
    #[value(name = "research_pipeline")]
    ResearchPipeline,
    #[value(name = "mechanical_reaction")]
    MechanicalReaction,
    #[value(name = "timer_wake")]
    TimerWake,
    #[value(name = "receiving_operation")]
    ReceivingOperation,
    #[value(name = "validation_loop")]
    ValidationLoop,
    #[value(name = "subagent_validation")]
    SubagentValidation,
    #[value(name = "multi_subagent_validation")]
    MultiSubagentValidation,
    #[value(name = "subagent_validation_failure")]
    SubagentValidationFailure,
    #[value(name = "custom_validator")]
    CustomValidator,
    #[value(name = "validation_self_repair")]
    ValidationSelfRepair,
    #[value(name = "validation_scope_enforcement")]
    ValidationScopeEnforcement,
    #[value(name = "validation_chain")]
    ValidationChain,
}

impl ScenarioId {
    pub const ALL: [Self; 19] = [
        Self::DirectAnswer,
        Self::PersistentState,
        Self::SecurityReview,
        Self::ReactiveAutomation,
        Self::ShellCoderSandbox,
        Self::DesignTradeoff,
        Self::SecurityTriage,
        Self::ResearchPipeline,
        Self::MechanicalReaction,
        Self::TimerWake,
        Self::ReceivingOperation,
        Self::ValidationLoop,
        Self::SubagentValidation,
        Self::MultiSubagentValidation,
        Self::SubagentValidationFailure,
        Self::CustomValidator,
        Self::ValidationSelfRepair,
        Self::ValidationScopeEnforcement,
        Self::ValidationChain,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectAnswer => direct_answer::ID,
            Self::PersistentState => persistent_state::ID,
            Self::SecurityReview => security_review::ID,
            Self::ReactiveAutomation => reactive_automation::ID,
            Self::ShellCoderSandbox => shell_coder_sandbox::ID,
            Self::DesignTradeoff => design_tradeoff::ID,
            Self::SecurityTriage => security_triage::ID,
            Self::ResearchPipeline => research_pipeline::ID,
            Self::MechanicalReaction => mechanical_reaction::ID,
            Self::TimerWake => timer_wake::ID,
            Self::ReceivingOperation => receiving_operation::ID,
            Self::ValidationLoop => validation_loop::ID,
            Self::SubagentValidation => subagent_validation::ID,
            Self::MultiSubagentValidation => multi_subagent_validation::ID,
            Self::SubagentValidationFailure => subagent_validation_failure::ID,
            Self::CustomValidator => custom_validator::ID,
            Self::ValidationSelfRepair => validation_self_repair::ID,
            Self::ValidationScopeEnforcement => validation_scope_enforcement::ID,
            Self::ValidationChain => validation_chain::ID,
        }
    }

    pub fn spec(self, run_id: &str) -> ScenarioSpec {
        match self {
            Self::DirectAnswer => direct_answer::scenario(run_id),
            Self::PersistentState => persistent_state::scenario(run_id),
            Self::SecurityReview => security_review::scenario(run_id),
            Self::ReactiveAutomation => reactive_automation::scenario(run_id),
            Self::ShellCoderSandbox => shell_coder_sandbox::scenario(run_id),
            Self::DesignTradeoff => design_tradeoff::scenario(run_id),
            Self::SecurityTriage => security_triage::scenario(run_id),
            Self::ResearchPipeline => research_pipeline::scenario(run_id),
            Self::MechanicalReaction => mechanical_reaction::scenario(run_id),
            Self::TimerWake => timer_wake::scenario(run_id),
            Self::ReceivingOperation => receiving_operation::scenario(run_id),
            Self::ValidationLoop => validation_loop::scenario(run_id),
            Self::SubagentValidation => subagent_validation::scenario(run_id),
            Self::MultiSubagentValidation => multi_subagent_validation::scenario(run_id),
            Self::SubagentValidationFailure => subagent_validation_failure::scenario(run_id),
            Self::CustomValidator => custom_validator::scenario(run_id),
            Self::ValidationSelfRepair => validation_self_repair::scenario(run_id),
            Self::ValidationScopeEnforcement => validation_scope_enforcement::scenario(run_id),
            Self::ValidationChain => validation_chain::scenario(run_id),
        }
    }
}

pub fn selected(requested: &[ScenarioId]) -> Vec<ScenarioId> {
    if requested.is_empty() {
        return ScenarioId::ALL.to_vec();
    }
    requested.iter().copied().fold(Vec::new(), |mut ids, id| {
        if !ids.contains(&id) {
            ids.push(id);
        }
        ids
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    #[test]
    fn registry_contains_nineteen_unique_valid_scenarios() {
        let mut ids = HashSet::new();
        for scenario in ScenarioId::ALL {
            assert!(ids.insert(scenario.as_str()));
            scenario.spec("run").validate().unwrap();
        }
        assert_eq!(ids.len(), 19);
    }

    #[test]
    fn explicit_selection_preserves_order_and_deduplicates() {
        assert_eq!(
            selected(&[
                ScenarioId::ReactiveAutomation,
                ScenarioId::DirectAnswer,
                ScenarioId::ReactiveAutomation,
            ]),
            vec![ScenarioId::ReactiveAutomation, ScenarioId::DirectAnswer]
        );
    }

    #[test]
    fn reactive_automation_uses_the_provider_output_limit() {
        assert_eq!(
            ScenarioId::ReactiveAutomation
                .spec("run")
                .execution
                .max_output_tokens,
            None
        );
    }

    #[test]
    fn judge_backed_scenarios_use_the_quality_signal_floor() {
        for scenario in [
            ScenarioId::DirectAnswer,
            ScenarioId::SecurityReview,
            ScenarioId::DesignTradeoff,
            ScenarioId::SecurityTriage,
        ] {
            let spec = scenario.spec("run");
            assert!(spec.needs_judge());
            assert_eq!(spec.threshold, JUDGE_BACKED_PASS_THRESHOLD);
        }
    }

    #[test]
    fn validation_rejects_a_different_judge_backed_threshold() {
        let mut spec = ScenarioId::DirectAnswer.spec("run");
        spec.threshold = JUDGE_BACKED_PASS_THRESHOLD + 1;

        assert_eq!(
            spec.validate().unwrap_err().to_string(),
            "scenario 'direct_answer': judge-backed threshold=51; expected 50; set this threshold or remove judge_reference"
        );
    }

    #[test]
    fn validation_rejects_a_zero_contract_version() {
        let mut spec = ScenarioId::PersistentState.spec("run");
        spec.version = 0;

        assert_eq!(
            spec.validate().unwrap_err().to_string(),
            "scenario 'persistent_state': version=0; expected version >= 1"
        );
    }

    #[test]
    fn validation_identifies_the_invalid_execution_field() {
        type ValidationCase = (&'static str, fn(&mut ExecutionPolicy), &'static str);

        let cases: [ValidationCase; 5] = [
            (
                "max_turns",
                |execution| execution.max_turns = 0,
                "scenario 'persistent_state': execution.max_turns=0; expected at least 1",
            ),
            (
                "max_output_tokens",
                |execution| execution.max_output_tokens = Some(0),
                "scenario 'persistent_state': execution.max_output_tokens=0; expected None (provider limit) or at least 1",
            ),
            (
                "max_total_tokens",
                |execution| execution.max_total_tokens = 0,
                "scenario 'persistent_state': execution.max_total_tokens=0; expected at least 1",
            ),
            (
                "stuck_timeout_seconds",
                |execution| execution.stuck_timeout_seconds = 0,
                "scenario 'persistent_state': execution.stuck_timeout_seconds=0; expected at least 1",
            ),
            (
                "total_token_order",
                |execution| execution.max_total_tokens = 1,
                "scenario 'persistent_state': execution.max_total_tokens=1 is lower than execution.max_output_tokens=8192; expected max_total_tokens >= max_output_tokens",
            ),
        ];

        for (field, mutate, expected) in cases {
            let mut spec = ScenarioId::PersistentState.spec("run");
            mutate(&mut spec.execution);
            assert_eq!(
                spec.validate().unwrap_err().to_string(),
                expected,
                "{field}"
            );
        }
    }

    #[test]
    fn validation_reports_criterion_values_before_weight_total() {
        let mut spec = ScenarioId::PersistentState.spec("run");
        spec.criteria = vec![CriterionSpec {
            id: "durable_result",
            weight: 0,
            description: "invalid",
        }];

        assert_eq!(
            spec.validate().unwrap_err().to_string(),
            "scenario 'persistent_state': criterion 'durable_result' has weight=0; expected at least 1"
        );
    }

    #[test]
    fn validation_reports_duplicate_criterion_indexes() {
        let mut spec = ScenarioId::PersistentState.spec("run");
        spec.criteria = vec![
            CriterionSpec {
                id: "duplicate",
                weight: 50,
                description: "first",
            },
            CriterionSpec {
                id: "duplicate",
                weight: 50,
                description: "second",
            },
        ];

        assert_eq!(
            spec.validate().unwrap_err().to_string(),
            "scenario 'persistent_state': criterion id 'duplicate' is duplicated at indexes 0 and 1; criterion ids must be unique"
        );
    }
}
