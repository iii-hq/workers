//! UI-001 — integration starts a streamed turn; Playwright validates Console UI.

use anyhow::ensure;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
    AuthoredScenario::new(
        "UI-001",
        "A harness-started streamed turn renders to durable completion in the Console.",
    )
    .trigger(Harness::send("Return the console fixture phrase."))
    .model((Reply::text("console fixture complete")
        .chunks(["console fixture ", "complete"])
        .usage(9, 3),))
    // Content assertions for UI scenarios live in Playwright (`ui-send` checks
    // the rendered text and both message counts in the DOM); the floor (turn
    // completion, script consumption, clean send) is runner-owned, so this
    // only checks what the DOM cannot show.
    .verify(|run| {
        ensure!(
            !run.has_duplicate_messages(),
            "transcript contains duplicate entry ids"
        );
        Ok(())
    })
}
