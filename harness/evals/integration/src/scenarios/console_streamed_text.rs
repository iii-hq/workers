//! UI-001 — the production Console sends through its agent-trigger policy.

use anyhow::ensure;
use serde_json::json;

use crate::types::scenario::GenerationMatchOverridesV1;

use super::builder::*;

pub(super) fn scenario() -> Scenario {
    AuthoredScenario::new(
        "UI-001",
        "A message sent from the Console streams to durable completion.",
    )
    .trigger(Harness::send("Return the console fixture phrase."))
    .model((Reply::text("console fixture complete")
        .chunks(["console fixture ", "complete"])
        .usage(9, 3)
        .match_overrides(GenerationMatchOverridesV1 {
            // The Console supplies agent mode and its production function
            // policy, so its composed prompt intentionally differs from
            // the direct integration request's native-policy golden.
            system_prompt: Some(regex("agent_trigger")),
            tools: Some(subset(json!([{ "name": "agent_trigger" }]))),
            ..Default::default()
        }),))
    // Content assertions for UI scenarios live in Playwright (`ui-send` checks
    // the rendered text and both message counts in the DOM); the floor (turn
    // completion, script consumption) is runner-owned, so this only checks what
    // the DOM cannot show.
    .verify(|run| {
        ensure!(
            !run.has_duplicate_messages(),
            "transcript contains duplicate entry ids"
        );
        Ok(())
    })
}
