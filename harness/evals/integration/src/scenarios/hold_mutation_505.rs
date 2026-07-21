//! E2E-505 — a holding hook's mutation reaches the released call.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/505>.

use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "E2E-505",
        "A pre-trigger hook that holds and mutates must apply its mutation to the released call.",
    )
    .quarantine()
    .trigger(Harness::send("Call the recorder once."))
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
    .model((
        Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4),
        Reply::text("approved and recorded")
            .usage(20, 3)
            .recovery_boundary(),
    ))
    .expect([
        turn_completes(),
        script_fully_consumed(),
        no_duplicate_messages(),
        all_calls_closed(),
        function_called_once("record", json!({ "value": "expected+approved" })),
        function_called_matching(
            "hook-gate",
            1,
            json!({
                "point": "pre_trigger",
                "call": {
                    "id": "call-1",
                    "function_id": "{{run_id}}::record",
                    "arguments": { "value": "expected" }
                }
            }),
        ),
    ])
}
