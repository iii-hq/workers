//! E2E-506 — released held calls retain hook-mutated arguments.
//!
//! Reproduction of <https://github.com/iii-hq/workers/issues/506>.

use anyhow::ensure;
use serde_json::json;

use crate::evidence_data::json_contains;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
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
    .model((
        Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4),
        Reply::text("released and recorded")
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
            payload == &json!({ "value": "expected+scope" }),
            "record payload {payload} lost the hook mutation"
        );

        // Hook payloads carry engine-populated fields beyond these subsets:
        // hook-mutate is consulted with the original arguments, hook-hold with
        // the arguments hook-mutate produced.
        let hook_subset = |arguments: &str| {
            json!({
                "point": "pre_trigger",
                "call": {
                    "id": "call-1",
                    "function_id": format!("{}::record", run.run_id),
                    "arguments": { "value": arguments }
                }
            })
        };
        for (alias, arguments) in [("hook-mutate", "expected"), ("hook-hold", "expected+scope")] {
            let calls = run.calls(alias);
            ensure!(
                calls.len() == 1,
                "{alias} ran {} times, not exactly once",
                calls.len()
            );
            let consulted = hook_subset(arguments);
            ensure!(
                json_contains(&calls[0].payload, &consulted),
                "{alias} payload {} does not contain {consulted}",
                calls[0].payload
            );
        }
        Ok(())
    })
}
