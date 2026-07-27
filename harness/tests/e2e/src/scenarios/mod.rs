use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use anyhow::{bail, Result};
use clap::ValueEnum;
use harness::functions::metrics::SessionMetricsResponseV1;
use serde::Serialize;
use serde_json::Value;

use crate::context::E2eContext;
use crate::report::HardGateReport;

pub mod common;
pub mod direct_answer;
pub mod persistent_state;
pub mod reactive_automation;
pub mod security_review;

pub type EvaluationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ObjectiveEvaluation>> + Send + 'a>>;
pub type CleanupFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
pub type ScenarioEvaluator =
    for<'a> fn(&'a E2eContext, &'a ScenarioObservation, &'a str) -> EvaluationFuture<'a>;
pub type ScenarioCleanup = for<'a> fn(&'a E2eContext, &'a str) -> CleanupFuture<'a>;

#[derive(Debug, Clone)]
pub struct CriterionSpec {
    pub id: &'static str,
    pub weight: u8,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExecutionPolicy {
    pub max_turns: u32,
    pub max_output_tokens: u64,
    pub max_total_tokens: u64,
    pub timeout_seconds: u64,
}

impl ExecutionPolicy {
    fn validate(self, scenario_id: &str) -> Result<()> {
        if self.max_turns == 0
            || self.max_output_tokens == 0
            || self.max_total_tokens == 0
            || self.timeout_seconds == 0
        {
            bail!("scenario {scenario_id} has an invalid execution policy");
        }
        if self.max_total_tokens < self.max_output_tokens {
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
    pub prompt: String,
    pub execution: ExecutionPolicy,
    pub threshold: u8,
    pub criteria: Vec<CriterionSpec>,
    pub judge_reference: Option<Value>,
    pub evaluate: ScenarioEvaluator,
    pub cleanup: Option<ScenarioCleanup>,
}

impl ScenarioSpec {
    pub fn validate(&self) -> Result<()> {
        if self.prompt.trim().is_empty() {
            bail!("scenario {} has an empty prompt", self.id);
        }
        self.execution.validate(self.id)?;
        if !(1..=100).contains(&self.threshold) {
            bail!("scenario {} threshold must be between 1 and 100", self.id);
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
}

impl ScenarioId {
    pub const ALL: [Self; 4] = [
        Self::DirectAnswer,
        Self::PersistentState,
        Self::SecurityReview,
        Self::ReactiveAutomation,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectAnswer => direct_answer::ID,
            Self::PersistentState => persistent_state::ID,
            Self::SecurityReview => security_review::ID,
            Self::ReactiveAutomation => reactive_automation::ID,
        }
    }

    pub fn spec(self, run_id: &str) -> ScenarioSpec {
        match self {
            Self::DirectAnswer => direct_answer::scenario(run_id),
            Self::PersistentState => persistent_state::scenario(run_id),
            Self::SecurityReview => security_review::scenario(run_id),
            Self::ReactiveAutomation => reactive_automation::scenario(run_id),
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
    fn registry_contains_four_unique_valid_scenarios() {
        let mut ids = HashSet::new();
        for scenario in ScenarioId::ALL {
            assert!(ids.insert(scenario.as_str()));
            scenario.spec("run").validate().unwrap();
        }
        assert_eq!(ids.len(), 4);
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
}
