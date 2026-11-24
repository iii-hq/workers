//! UI-007 — a side chat the Console opens and sends in runs on the
//! conversation's prefix.
//!
//! chat-projects §2.4 / §2.8 (1–2), driven by Playwright
//! (`ade/web/e2e/side-chat.spec.ts`). The conversation's turn is the
//! playground's usual one (the ready manifest's `send`, through the SDK or
//! the Console) and answers `REPLY`. The CONSOLE then opens a side chat at
//! that reply (`harness::side-chat::open`, from "Side chat" on the reply or
//! "+ New" in the Sidechats section) and sends `SIDE_MESSAGE` in it through
//! `harness::send`, naming no model, provider or effort: the harness runs it
//! on the conversation's lock. The fixture never sends in the side chat
//! itself; it waits until the router served that second generation
//! (`await_generations`) before it takes the shutdown.
//!
//! `verify` is INT-046's: the side chat's router request carries the
//! conversation's system prompt, system sections and tools, its history
//! starts with the conversation's up to the reply, nothing reaches the
//! conversation, and exactly two generations ran.

use super::ScenarioDriver;
use super::dsl::{Model, Scenario, Send};
use super::side_chat_same_prefix::{
    MESSAGE, SET_DESCRIPTION, conversation_generation, side_chat_generation, verify_side_chat,
};
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "UI-007";

    Scenario::new(
        ID,
        "console-side-chat",
        "After the conversation's turn, the Console opens a side chat at its reply and sends in \
         it with no model or effort: the side chat's first request carries the conversation's \
         system prompt, system sections and tools, its history starts with the conversation's, \
         nothing reaches the conversation, and the run makes two model calls.",
        ScenarioDriver::Playground,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:ui-007")
            .allow_id(SET_DESCRIPTION),
    )
    .generation(conversation_generation())
    .generation(side_chat_generation())
    // The side chat is not the tracked session: its turn completes where no
    // observer of the run listens, so the run waits on the router instead.
    .await_generations()
    .verify(verify_side_chat)
    .build()
}

#[cfg(test)]
mod tests {
    use super::super::side_chat_same_prefix::{REPLY, SIDE_REPLY};
    use super::*;

    #[test]
    fn scripts_the_conversation_and_a_side_chat_the_console_sends_in() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.driver, ScenarioDriver::Playground);
        assert_eq!(fixture.slug, "console-side-chat");
        assert_eq!(fixture.script.generations.len(), 2);
        // The Console sends in the side chat: no probe does it for it.
        assert!(fixture.probe_actions.is_empty());
        assert!(fixture.await_generations);
        // One tracked turn: the side chat's completes in its own session.
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.expected_traces(), 1);
        let side = serde_json::to_string(&fixture.script.generations[1]).unwrap();
        assert!(side.contains("{{system_prompt_sha256}}"), "{side}");
        assert!(side.contains(REPLY) && side.contains(SIDE_REPLY), "{side}");
        assert_eq!(fixture.scenario.send.message, MESSAGE);
    }
}
