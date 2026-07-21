//! E2E-506 — released held calls retain hook-mutated arguments.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/506>.

use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "E2E-506",
        "A held call released for execution must run with the arguments produced by earlier hooks.",
    )
    .quarantine()
    .trigger(Harness::send("Call the recorder once."))
    .function("record", Function::recorder())
    .function(
        "hook-mutate",
        Function::new(
            "Inject validated scope into the arguments.",
            json!({ "type": "object" }),
            json!({
                "decision": "continue",
                "mutations": { "arguments": { "value": "expected+scope" } }
            }),
        )
        .hidden(),
    )
    .function(
        "hook-hold",
        Function::new(
            "Hold every consulted call for explicit approval.",
            json!({ "type": "object" }),
            json!({ "decision": "hold" }),
        )
        .hidden(),
    )
    .binding(Binding::hook_pre_trigger("hook-mutate", ["record"], 10))
    .binding(Binding::hook_pre_trigger("hook-hold", ["record"], 20))
    .release(Release::execute())
    .generation(Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4))
    .generation(
        Reply::text("released and recorded")
            .usage(20, 3)
            .recovery_boundary(),
    )
    .expect(
        Expect::new()
            .calls_closed()
            .call(TargetCall::counted("record", 1).payload(json!({ "value": "expected+scope" })))
            .call(TargetCall::counted("hook-mutate", 1).payload_subset(json!({
                "point": "pre_trigger",
                "call": {
                    "id": "call-1",
                    "function_id": "{{run_id}}::record",
                    "arguments": { "value": "expected" }
                }
            })))
            .call(TargetCall::counted("hook-hold", 1).payload_subset(json!({
                "point": "pre_trigger",
                "call": {
                    "id": "call-1",
                    "function_id": "{{run_id}}::record",
                    "arguments": { "value": "expected+scope" }
                }
            }))),
    )
}
