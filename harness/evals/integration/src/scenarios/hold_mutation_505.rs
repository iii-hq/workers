//! C-E2E-505 — a holding hook's mutation reaches the released call.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/505>.

use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "C-E2E-505",
        "A pre-trigger hook that holds and mutates must apply its mutation to the released call.",
    )
    .quarantine()
    .send(Send::message("Call the recorder once."))
    .function("record", Function::recorder())
    .function(
        "hook-gate",
        Function::new(
            "Hold the call and stamp approval context onto its arguments.",
            json!({ "type": "object" }),
            json!({
                "decision": "hold",
                "mutations": { "arguments": { "value": "expected+approved" } }
            }),
        )
        .hidden(),
    )
    .binding(Binding::hook_pre_trigger("hook-gate", ["record"], 10))
    .release(Release::execute())
    .generation(Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4))
    .generation(Reply::text("approved and recorded").usage(20, 3).recovery_boundary())
    .expect(
        Expect::new()
            .calls_closed()
            .call(TargetCall::counted("record", 1).payload(json!({ "value": "expected+approved" })))
            .call(TargetCall::counted("hook-gate", 1).payload_subset(json!({
                "point": "pre_trigger",
                "call": {
                    "id": "call-1",
                    "function_id": "{{run_id}}::record",
                    "arguments": { "value": "expected" }
                }
            }))),
    )
}
