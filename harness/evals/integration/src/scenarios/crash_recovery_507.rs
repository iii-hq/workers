//! E2E-507 — crash recovery closes the interrupted function call.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/507>.

use anyhow::ensure;
use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
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
        // differently; this reproduction checks the durable outcome instead.
        Reply::text("recovered").usage(20, 2).recovery_boundary(),
    ))
    .fault(Fault::engine_sigkill())
    .scenario_timeout_ms(120_000)
    // Recovery is proven by the closed call, the message shape, and both
    // generations being consumed (floor); text durability is `streamed-text`'s
    // pin.
.verify(|run| {
    let counts = run.message_counts();
    ensure!(
        counts == (1, 2, 1),
        "message counts (user, assistant, function_result) {counts:?} != (1, 2, 1)"
    );
    ensure!(
        run.all_calls_closed(),
        "a dispatched function call has no durable result"
    );
    ensure!(
        run.function_result_closes("call-1"),
        "no single durable function result closes call-1"
    );
    let calls = run.calls("record");
    ensure!(
        calls.len() == 1,
        "record ran {} times, not exactly once",
        calls.len()
    );
    let payload = &calls[0].payload;
    ensure!(
        payload == &json!({ "value": "expected" }),
        "record payload {payload} != {{\"value\":\"expected\"}}"
    );
    ensure!(
        !run.has_duplicate_messages(),
        "transcript contains duplicate entry ids"
    );
        Ok(())
    })
}
