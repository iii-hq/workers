use std::collections::HashSet;
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
        if self.max_turns == 0
            || self.max_output_tokens == Some(0)
            || self.max_total_tokens == 0
            || self.stuck_timeout_seconds == 0
        {
            bail!("scenario {scenario_id} has an invalid execution policy");
        }
        if self
            .max_output_tokens
            .is_some_and(|max_output_tokens| self.max_total_tokens < max_output_tokens)
        {
            bail!(
                "scenario {scenario_id} total-token budget is smaller than its per-call output budget"
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
            bail!("scenario {} has an empty prompt", self.id);
        }
        if self.version == 0 {
            bail!("scenario {} version must be positive", self.id);
        }
        self.execution.validate(self.id)?;
        if !(1..=100).contains(&self.threshold) {
            bail!("scenario {} threshold must be between 1 and 100", self.id);
        }
        if self.needs_judge() && self.threshold != JUDGE_BACKED_PASS_THRESHOLD {
            bail!(
                "judge-backed scenario {} threshold must be {JUDGE_BACKED_PASS_THRESHOLD}",
                self.id
            );
        }
        let total: u16 = self
            .criteria
            .iter()
            .map(|criterion| u16::from(criterion.weight))
            .sum();
        if total != 100 {
            bail!(
                "scenario {} criterion weights total {total}, expected 100",
                self.id
            );
        }
        let mut ids = HashSet::new();
        for criterion in &self.criteria {
            if criterion.id.trim().is_empty() || criterion.weight == 0 {
                bail!("scenario {} has an invalid criterion", self.id);
            }
            if !ids.insert(criterion.id) {
                bail!("scenario {} repeats criterion {}", self.id, criterion.id);
            }
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
            "judge-backed scenario direct_answer threshold must be 50"
        );
    }

    #[test]
    fn validation_rejects_a_zero_contract_version() {
        let mut spec = ScenarioId::PersistentState.spec("run");
        spec.version = 0;

        assert_eq!(
            spec.validate().unwrap_err().to_string(),
            "scenario persistent_state version must be positive"
        );
    }
}
