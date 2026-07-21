//! E2E-505 — a holding hook's mutation reaches the released call.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/505>.

use anyhow::ensure;
use serde_json::json;

use crate::evidence_data::json_contains;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
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
    .verify(|run| {
        ensure!(
            !run.has_duplicate_messages(),
            "transcript contains duplicate entry ids"
        );
        ensure!(
            run.all_calls_closed(),
            "a dispatched function call has no durable result"
        );

        let record = run.calls("record");
        ensure!(
            record.len() == 1,
            "record ran {} times, not exactly once",
            record.len()
        );
        let payload = &record[0].payload;
        ensure!(
            payload == &json!({ "value": "expected+approved" }),
            "record payload {payload} lost the hook mutation"
        );

        let gate = run.calls("hook-gate");
        ensure!(
            gate.len() == 1,
            "hook-gate ran {} times, not exactly once",
            gate.len()
        );
        // Hook payloads carry engine-populated fields beyond this subset.
        let consulted = json!({
            "point": "pre_trigger",
            "call": {
                "id": "call-1",
                "function_id": format!("{}::record", run.run_id),
                "arguments": { "value": "expected" }
            }
        });
        ensure!(
            json_contains(&gate[0].payload, &consulted),
            "hook-gate payload {} does not contain {consulted}",
            gate[0].payload
        );
        Ok(())
    })
}
