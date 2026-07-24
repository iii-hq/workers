use std::time::Duration;

use serde_json::{json, Value};

use crate::context::ScenarioContext;
use crate::error::EvalError;
use crate::report::ScenarioObservationV1;

use super::{common, EvaluationFuture, ScenarioSpec};

pub const ID: &str = "triggered_work";

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = format!("agent-quality:{run_id}");
    let signal_key = format!("{ID}:signal");
    let result_key = format!("{ID}:result");
    let expected = json!({ "status": "triggered", "run_id": run_id });
    let registration = json!({
        "trigger_type": "state",
        "config": { "scope": scope, "key": signal_key },
        "once": true,
        "function_id": "harness::react",
        "metadata": {
            "call": {
                "function_id": "state::set",
                "payload": {
                    "scope": scope,
                    "key": result_key,
                    "value": expected
                }
            }
        }
    });
    let signal = json!({
        "scope": scope,
        "key": signal_key,
        "value": { "status": "fire" }
    });
    ScenarioSpec {
        id: ID,
        prompt: format!(
            "First call engine::register_trigger exactly once with these arguments: {}. After it succeeds, call state::set exactly once with these arguments: {}. Do not set the result key directly. Finish after the signal succeeds.",
            serde_json::to_string(&registration).unwrap(),
            serde_json::to_string(&signal).unwrap()
        ),
        evaluation_context: json!({
            "scope": scope,
            "signal_key": signal_key,
            "result_key": result_key,
            "expected": expected,
        }),
        evaluate,
    }
}

fn evaluate<'a>(
    context: &'a ScenarioContext,
    observation: &'a ScenarioObservationV1,
    evaluation_context: &'a Value,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let scope = string_field(evaluation_context, "scope")?;
        let signal_key = string_field(evaluation_context, "signal_key")?;
        let result_key = string_field(evaluation_context, "result_key")?;
        let expected = &evaluation_context["expected"];
        let observed = wait_for_state(context, scope, result_key).await?;

        for key in [signal_key, result_key] {
            context
                .trigger::<_, Value>("state::delete", json!({ "scope": scope, "key": key }))
                .await?;
        }
        if observed != *expected {
            return Err(EvalError::assertion(format!(
                "triggered result mismatch: expected {expected}, observed {observed}"
            )));
        }
        if observation.metrics.totals.function_call_errors != 0 {
            return Err(EvalError::assertion(format!(
                "expected no function-call errors, observed {}",
                observation.metrics.totals.function_call_errors
            )));
        }
        Ok(json!({ "expected": expected, "actual": observed }))
    })
}

async fn wait_for_state(
    context: &ScenarioContext,
    scope: &str,
    key: &str,
) -> Result<Value, EvalError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let value = common::state_value(
            context
                .trigger("state::get", json!({ "scope": scope, "key": key }))
                .await?,
        );
        if !value.is_null() {
            return Ok(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(EvalError::assertion(
                "triggered state result did not arrive",
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, EvalError> {
    value[field]
        .as_str()
        .ok_or_else(|| EvalError::assertion(format!("missing evaluation field {field}")))
}
