use serde_json::{json, Value};

use crate::context::ScenarioContext;
use crate::error::EvalError;
use crate::report::ScenarioObservationV1;

use super::{common, EvaluationFuture, ScenarioSpec};

pub const ID: &str = "plain_response";
const EXPECTED: &str = "HARNESS_E2E_PLAIN_OK";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        prompt: format!(
            "Reply with exactly `{EXPECTED}` and nothing else. Do not call any function."
        ),
        evaluation_context: Value::Null,
        evaluate,
    }
}

fn evaluate<'a>(
    _context: &'a ScenarioContext,
    observation: &'a ScenarioObservationV1,
    _evaluation_context: &'a Value,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let metrics = &observation.metrics;
        if metrics.totals.function_calls != 0 || metrics.totals.turns != 1 {
            return Err(EvalError::assertion(format!(
                "expected one assistant turn and no function calls, observed {} turns and {} calls",
                metrics.totals.turns, metrics.totals.function_calls
            )));
        }
        let texts = common::assistant_texts(&observation.transcript);
        if texts != [EXPECTED] {
            return Err(EvalError::assertion(format!(
                "expected one assistant entry containing {EXPECTED:?}, observed {texts:?}"
            )));
        }
        Ok(json!({ "expected": EXPECTED, "actual": texts[0] }))
    })
}
