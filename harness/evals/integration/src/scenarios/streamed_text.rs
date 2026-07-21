//! E2E-001 — streamed text reaches durable completion.

use anyhow::ensure;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
    AuthoredScenario::new(
        "E2E-001",
        "Streamed text reaches durable completion through the real queue and turn loop.",
    )
    .trigger(Harness::send("Return the fixture phrase."))
    .model((Reply::text("fixture complete")
        .chunks(["fixture ", "complete"])
        .usage(8, 2),))
    // The one direct pin for assistant-text durability and transcript shape.
    .verify(|run| {
        let texts = run.assistant_texts();
        ensure!(
            texts == ["fixture complete"],
            "assistant texts {texts:?} != [\"fixture complete\"]"
        );
        let counts = run.message_counts();
        ensure!(
            counts == (1, 1, 0),
            "message counts (user, assistant, function_result) {counts:?} != (1, 1, 0)"
        );
        ensure!(
            !run.has_duplicate_messages(),
            "transcript contains duplicate entry ids"
        );
        Ok(())
    })
}
