use serde_json::{json, Value};

use crate::context::ScenarioContext;
use crate::error::EvalError;
use crate::report::ScenarioObservationV1;

use super::{common, EvaluationFuture, ScenarioSpec};

pub const ID: &str = "single_function";

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = format!("e2e:{run_id}");
    let key = ID;
    let expected = json!({ "run_id": run_id, "scenario": ID, "status": "written" });
    let arguments = json!({ "scope": scope, "key": key, "value": expected });
    ScenarioSpec {
        id: ID,
        prompt: format!(
            "Call state::set exactly once with these exact arguments: {}. After the function succeeds, finish the task.",
            serde_json::to_string(&arguments).unwrap()
        ),
        evaluation_context: arguments,
        evaluate,
    }
}

fn evaluate<'a>(
    context: &'a ScenarioContext,
    observation: &'a ScenarioObservationV1,
    evaluation_context: &'a Value,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let scope = evaluation_context["scope"]
            .as_str()
            .ok_or_else(|| EvalError::assertion("missing evaluation scope"))?;
        let key = evaluation_context["key"]
            .as_str()
            .ok_or_else(|| EvalError::assertion("missing evaluation key"))?;
        let expected = &evaluation_context["value"];
        let observed = common::state_value(
            context
                .trigger("state::get", json!({ "scope": scope, "key": key }))
                .await?,
        );
        context
            .trigger::<_, Value>("state::delete", json!({ "scope": scope, "key": key }))
            .await?;

        if observed != *expected {
            return Err(EvalError::assertion(format!(
                "state value mismatch: expected {expected}, observed {observed}"
            )));
        }
        let metrics = &observation.metrics;
        if metrics.totals.function_calls == 0 || metrics.totals.function_call_errors != 0 {
            return Err(EvalError::assertion(format!(
                "expected successful function execution, observed {} calls and {} errors",
                metrics.totals.function_calls, metrics.totals.function_call_errors
            )));
        }
        Ok(json!({ "expected": expected, "actual": observed }))
    })
}
