//! E2E-002 — the recorder runs exactly once and its result closes the turn.

use anyhow::ensure;
use serde_json::json;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
    AuthoredScenario::new("E2E-002", "The recorder runs exactly once.")
        .trigger(Harness::send("Call the recorder once."))
        .function("record", Function::recorder())
        .model((
            Reply::function_call("record", json!({ "value": "expected" })).usage(8, 4),
            Reply::text("recorded once").usage(18, 2),
        ))
        .verify(|run| {
            let texts = run.assistant_texts();
            ensure!(
                texts == ["recorded once"],
                "assistant texts {texts:?} != [\"recorded once\"]"
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
