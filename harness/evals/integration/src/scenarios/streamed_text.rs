//! E2E-001 — streamed text reaches durable completion.

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "E2E-001",
        "Streamed text reaches durable completion through the real queue and turn loop.",
    )
    .trigger(Harness::send("Return the fixture phrase."))
    .model((Reply::text("fixture complete")
        .chunks(["fixture ", "complete"])
        .usage(8, 2),))
    // The one direct pin for assistant-text durability and transcript shape.
    .expect([
        turn_completes(),
        script_fully_consumed(),
        no_duplicate_messages(),
        message_counts(1, 1, 0),
        assistant_said("fixture complete"),
    ])
}
