mod cleanup;
mod evaluate;
mod evidence;
mod names;
mod prompt;
mod queries;

use crate::context::E2eContext;

use super::{CriterionSpec, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};
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

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = ScenarioNames::new(run_id);
    ScenarioSpec {
        id: ID,
        prompt: prompt::build(&names, STUCK_WATCHDOG_SECONDS),
        execution: ExecutionPolicy {
            max_turns: 64,
            max_output_tokens: None,
            max_total_tokens: SCENARIO_MAX_TOTAL_TOKENS,
            stuck_timeout_seconds: STUCK_WATCHDOG_SECONDS,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "parallel_writes",
                weight: 25,
                description: "Three parallel writer sessions produce exactly five valid orders each.",
            },
            CriterionSpec {
                id: "reactive_aggregates",
                weight: 30,
                description: "Trigger-spawned reactors maintain totals that exactly match the source rows.",
            },
            CriterionSpec {
                id: "trigger_orchestration",
                weight: 25,
                description: "The watch is armed before writers start and the documented fallback is proven.",
            },
            CriterionSpec {
                id: "finalization_cleanup",
                weight: 20,
                description: "One trigger-spawned finalizer writes a passing report and removes run triggers.",
            },
        ],
        judge_reference: None,
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
