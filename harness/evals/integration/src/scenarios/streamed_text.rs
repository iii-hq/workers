//! C-E2E-001 — streamed text reaches durable completion.

use super::builder::*;

pub(super) fn scenario() -> AuthoredScenario {
    AuthoredScenario::new(
        "C-E2E-001",
        "Streamed text reaches durable completion through the real queue and turn loop.",
    )
    .send(Send::message("Return the fixture phrase."))
    .generation(
        Reply::text("fixture complete")
            .chunks(["fixture ", "complete"])
            .usage(8, 2),
    )
    .expect(
        Expect::new()
            .message_counts(1, 1, 0)
            .assistant_text("fixture complete"),
    )
}
