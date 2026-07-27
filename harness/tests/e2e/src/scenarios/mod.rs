use std::future::Future;
use std::pin::Pin;

use clap::ValueEnum;
use serde_json::Value;

use crate::context::ScenarioContext;
use crate::error::EvalError;
use crate::limits::E2eLimitsV1;
use crate::report::{E2eScenarioReportV1, ScenarioObservationV1};

pub mod common;
pub mod plain_response;
pub mod security_review;
pub mod single_function;
pub mod triggered_work;

pub type EvaluationFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, EvalError>> + Send + 'a>>;
pub type ScenarioEvaluator =
    for<'a> fn(&'a ScenarioContext, &'a ScenarioObservationV1, &'a Value) -> EvaluationFuture<'a>;

pub struct ScenarioSpec {
    pub id: &'static str,
    pub prompt: String,
    pub evaluation_context: Value,
    pub evaluate: ScenarioEvaluator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ScenarioId {
    #[value(name = "plain_response")]
    PlainResponse,
    #[value(name = "single_function")]
    SingleFunction,
    #[value(name = "security_review")]
    SecurityReview,
    #[value(name = "triggered_work")]
    TriggeredWork,
}

impl ScenarioId {
    pub const ALL: [Self; 4] = [
        Self::PlainResponse,
        Self::SingleFunction,
        Self::SecurityReview,
        Self::TriggeredWork,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlainResponse => plain_response::ID,
            Self::SingleFunction => single_function::ID,
            Self::SecurityReview => security_review::ID,
            Self::TriggeredWork => triggered_work::ID,
        }
    }

    pub async fn run(
        self,
        context: &ScenarioContext,
        run_id: &str,
        limits: &E2eLimitsV1,
    ) -> E2eScenarioReportV1 {
        let spec = match self {
            Self::PlainResponse => plain_response::scenario(run_id),
            Self::SingleFunction => single_function::scenario(run_id),
            Self::SecurityReview => security_review::scenario(run_id),
            Self::TriggeredWork => triggered_work::scenario(run_id),
        };
        common::run(context, run_id, limits, spec).await
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
    fn defaults_to_the_prompt_evaluation_scenarios() {
        assert_eq!(selected(&[]), ScenarioId::ALL);
        assert_eq!(ScenarioId::ALL.len(), 4);
    }

    #[test]
    fn explicit_selection_preserves_order_and_deduplicates() {
        assert_eq!(
            selected(&[
                ScenarioId::TriggeredWork,
                ScenarioId::PlainResponse,
                ScenarioId::TriggeredWork,
            ]),
            vec![ScenarioId::TriggeredWork, ScenarioId::PlainResponse]
        );
    }
}
