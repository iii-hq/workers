mod cleanup;
mod evaluate;
mod evidence;
mod names;
mod prompt;
mod queries;

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::{EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};
use names::ScenarioNames;

pub const ID: &str = "reactive_automation";
pub(super) const EXPECTED_WRITERS: usize = 3;
pub(super) const ORDERS_PER_WRITER: i64 = 5;
pub(super) const EXPECTED_ORDERS: i64 = EXPECTED_WRITERS as i64 * ORDERS_PER_WRITER;

const STUCK_WATCHDOG_SECONDS: u64 = 600;
// Shared across the long-lived root and every writer/reactor/finalizer turn.
// Discovery alone can approach one million input tokens on large-context
// models before the three writers start, so retain enough room for the actual
// workload without relaxing any behavioral gate.
const SCENARIO_MAX_TOTAL_TOKENS: u64 = 2_000_000;

const PARALLEL_WRITES: AssessmentSpec = AssessmentSpec::hard_gated(
    "parallel_writes",
    25,
    "Three parallel writer sessions produce exactly five valid orders each.",
);
const REACTIVE_AGGREGATES: AssessmentSpec = AssessmentSpec::hard_gated(
    "reactive_aggregates",
    30,
    "A mechanical trigger call maintains totals that exactly match the source rows.",
);
const TRIGGER_ORCHESTRATION: AssessmentSpec = AssessmentSpec::hard_gated(
    "trigger_orchestration",
    25,
    "The aggregate call and barrier wake are armed before writers start and proven by delivery records.",
);
const FINALIZATION_CLEANUP: AssessmentSpec = AssessmentSpec::hard_gated(
    "finalization_cleanup",
    20,
    "The barrier-woken root directly spawns one finalizer, which writes a passing report before cleanup.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[
    PARALLEL_WRITES,
    REACTIVE_AGGREGATES,
    TRIGGER_ORCHESTRATION,
    FINALIZATION_CLEANUP,
];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = ScenarioNames::new(run_id);
    ScenarioSpec {
        id: ID,
        version: 3,
        prompt: prompt::build(&names, STUCK_WATCHDOG_SECONDS),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 64,
            max_output_tokens: None,
            max_total_tokens: SCENARIO_MAX_TOTAL_TOKENS,
            stuck_timeout_seconds: STUCK_WATCHDOG_SECONDS,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: None,
        evaluate,
        cleanup: Some(cleanup::run),
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let names = ScenarioNames::new(run_id);
        if !context.function_exists("database::query").await? {
            return Ok(evaluate::missing_database());
        }
        let evidence = queries::collect(context, observation, &names).await?;
        Ok(evaluate::score(&evidence, &names))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_uses_the_canonical_assessment_contract() {
        let spec = scenario("run");

        spec.validate().unwrap();
        assert_eq!(spec.version, 3);
        assert_eq!(spec.threshold, 90);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight, criterion.description))
                .collect::<Vec<_>>(),
            vec![
                (
                    "parallel_writes",
                    25,
                    "Three parallel writer sessions produce exactly five valid orders each.",
                ),
                (
                    "reactive_aggregates",
                    30,
                    "A mechanical trigger call maintains totals that exactly match the source rows.",
                ),
                (
                    "trigger_orchestration",
                    25,
                    "The aggregate call and barrier wake are armed before writers start and proven by delivery records.",
                ),
                (
                    "finalization_cleanup",
                    20,
                    "The barrier-woken root directly spawns one finalizer, which writes a passing report before cleanup.",
                ),
            ]
        );
    }
}
