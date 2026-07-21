//! E2E-507 — crash recovery closes the interrupted function call.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/507>.

use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "E2E-507",
        "An engine crash during a dispatched function call must not leave the call dangling or the session unusable.",
    )
    .quarantine()
    .trigger(Harness::send("Call the recorder once."))
    .function("record", Function::recorder())
    .model((
        Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4),
        // Recovery can legitimately reconstruct the second request
        // differently; this reproduction grades the durable outcome instead.
        Reply::text("recovered").usage(20, 2).recovery_boundary(),
    ))
    .fault(Fault::engine_sigkill())
    .scenario_timeout_ms(120_000)
    // Recovery is proven by the closed call, the message shape, and both
    // generations being consumed; text durability is `streamed-text`'s pin.
    .expect([
        turn_completes(),
        script_fully_consumed(),
        no_duplicate_messages(),
        message_counts(1, 2, 1),
        all_calls_closed(),
        function_result_closes("call-1"),
        function_called_once("record", json!({ "value": "expected" })),
    ])
}
