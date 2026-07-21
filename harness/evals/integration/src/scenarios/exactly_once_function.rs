//! C-E2E-002 — an allow-listed function executes exactly once.

use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "C-E2E-002",
        "An allow-listed native function executes exactly once with a durable result.",
    )
    .send(Send::message("Call the recorder once."))
    .function("record", Function::recorder())
    .generation(Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4))
    .generation(Reply::text("recorded once").usage(18, 2))
    .expect(
        Expect::new()
            .message_counts(1, 2, 1)
            .assistant_text("recorded once")
            .function_result(
                FunctionResult::closing("call-1")
                    .function("record")
                    .content(vec![json!({ "type": "text", "text": "recorded" })])
                    .is_error(false),
            )
            .call(TargetCall::counted("record", 1).payload(json!({ "value": "expected" }))),
    )
}
