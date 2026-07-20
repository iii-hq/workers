//! C-E2E-507 — crash recovery closes the interrupted function call.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/507>.

use serde_json::json;

use crate::types::scenario::GenerationMatchOverridesV1;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "C-E2E-507",
        "An engine crash during a dispatched function call must not leave the call dangling or the session unusable.",
    )
    .quarantine()
    .send(Send::message("Call the recorder once."))
    .function(
        "record",
        Function::new(
            "Record one integration fixture value.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            }),
            json!({
                "content": [{ "type": "text", "text": "recorded" }],
                "is_error": false
            }),
        ),
    )
    .generation(Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4))
    // Recovery can legitimately reconstruct the second request differently;
    // this reproduction grades the durable outcome instead.
    .generation(
        Reply::text("recovered")
            .usage(20, 2)
            .match_overrides(GenerationMatchOverridesV1 {
                request_id: Some(regex("^t_[0-9a-f]{32}:[0-9]+$")),
                system_prompt: Some(present()),
                messages: Some(present()),
                tools: Some(present()),
                ..Default::default()
            }),
    )
    .fault(Fault::engine_sigkill())
    .scenario_timeout_ms(120_000)
    .expect(
        Expect::new()
            .message_counts(1, 2, 1)
            .assistant_text("recovered")
            .calls_closed()
            .function_result(FunctionResult::closing("call-1"))
            .call(TargetCall::counted("record", 1).payload(json!({ "value": "expected" }))),
    )
}
