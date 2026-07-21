//! E2E-002 — the recorder runs exactly once and its result closes the turn.

use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new("E2E-002", "The recorder runs exactly once.")
        .trigger(Harness::send("Call the recorder once."))
        .function("record", Function::recorder())
        .model((
            Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4),
            Reply::text("recorded once").usage(18, 2),
        ))
        .expect([
            turn_completes(),
            script_fully_consumed(),
            assistant_said("recorded once"),
            function_called_once("record", json!({ "value": "expected" })),
            no_duplicate_messages(),
        ])
}
