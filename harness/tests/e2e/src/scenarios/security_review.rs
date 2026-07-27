use serde_json::{json, Value};

use crate::context::ScenarioContext;
use crate::error::EvalError;
use crate::report::ScenarioObservationV1;

use super::{common, EvaluationFuture, ScenarioSpec};

pub const ID: &str = "security_review";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    let expected = json!({
        "findings": [
            { "id": 1, "issue": "hardcoded_credential" },
            { "id": 2, "issue": "sql_injection" },
            { "id": 3, "issue": "open_redirect" }
        ]
    });
    ScenarioSpec {
        id: ID,
        prompt: r#"Review these three independent snippets:
1. `const API_KEY = "sk-live-example";`
2. `db.execute("SELECT * FROM users WHERE id = " + request.id);`
3. `response.redirect(request.query.next);`

Return only valid JSON with this shape:
{"findings":[{"id":1,"issue":"..."},{"id":2,"issue":"..."},{"id":3,"issue":"..."}]}

Use exactly these issue names when applicable: hardcoded_credential, sql_injection, open_redirect. Do not call a function."#
            .into(),
        evaluation_context: expected,
        evaluate,
    }
}

fn evaluate<'a>(
    _context: &'a ScenarioContext,
    observation: &'a ScenarioObservationV1,
    expected: &'a Value,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let metrics = &observation.metrics;
        if metrics.totals.function_calls != 0 || metrics.totals.turns != 1 {
            return Err(EvalError::assertion(format!(
                "expected one reasoning turn and no function calls, observed {} turns and {} calls",
                metrics.totals.turns, metrics.totals.function_calls
            )));
        }
        let texts = common::assistant_texts(&observation.transcript);
        if texts.len() != 1 {
            return Err(EvalError::assertion(format!(
                "expected one assistant response, observed {texts:?}"
            )));
        }
        let actual: Value = serde_json::from_str(texts[0].trim()).map_err(|error| {
            EvalError::assertion(format!(
                "security review did not return valid JSON: {error}"
            ))
        })?;
        if actual != *expected {
            return Err(EvalError::assertion(format!(
                "security findings mismatch: expected {expected}, observed {actual}"
            )));
        }
        Ok(json!({ "expected": expected, "actual": actual }))
    })
}
