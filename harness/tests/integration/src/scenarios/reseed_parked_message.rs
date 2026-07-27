//! INT-003 — a message that parks during a turn's final step is delivered by a
//! reseeded turn.
//!
//! Regression guard for the harness finalize-drain reseed. The *completed*
//! finalize cannot be pinned from the public boundary: a message parked while
//! the terminal generation is in flight is seen by the loop's steering check
//! and delivered by an advance — the drain's window only opens after that
//! check, and no public actor can act inside it. The *failed* finalize has no
//! steering check, so a generation that parks a steer and then fails routes
//! the parked message deterministically through the finalize drain, which must
//! reseed a turn to react to it (both finalize paths share the same drain +
//! reseed). Without the reseed the parked message is stranded, no second
//! terminal turn arrives, and the run times out.

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-003";
    const MESSAGE: &str = "Answer the first question.";
    const PARKED: &str = "Follow-up that parked during finalize.";
    const SECOND_TEXT: &str = "handled the parked follow-up";

    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "reseed-parked-message",
        "A message parked during a turn's final step is delivered by a reseeded turn.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(Send::message(MESSAGE).idempotency_key("{{run_id}}:integration-003"))
    .terminal_turn_statuses(["failed", "completed"])
    .generation(
        // Turn 1's only step: the steer parks while the harness awaits this
        // generation, then the scripted failure drives finalize_failed — whose
        // drain delivers the parked row and must reseed.
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .without_tools(),
            )
            .parked_message(PARKED)
            .fails("scripted generation failure while the steered follow-up sits parked"),
    )
    .generation(
        // The reseeded turn's own step 0 (`:0` pins a fresh turn id, not an
        // advance of turn 1). Its context carries the original user message,
        // the failed generation's empty assistant residue, and the parked
        // follow-up delivered by the finalize drain.
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([
                        Message::user(MESSAGE),
                        Message::assistant_empty(&model),
                        Message::user(PARKED),
                    ])
                    .without_tools(),
            )
            .respond(Response::text(SECOND_TEXT, 12, 4)),
    )
    .verify(|run| {
        // The empty assistant residue carries no text; only the reseeded
        // turn's answer does.
        run.expect_assistant_texts([SECOND_TEXT])?;
        run.expect_message_counts(2, 2, 0)?;
        run.expect_no_duplicate_messages()?;

        // The follow-up must have PARKED (drained queue rows keep their
        // durable `e_q_` entry id) — not landed as a direct send.
        let parked_entry = run.transcript.iter().find(|item| {
            item.get("message")
                .and_then(|message| message.get("content"))
                .and_then(|content| content.as_array())
                .and_then(|blocks| blocks.first())
                .and_then(|block| block.get("text"))
                .and_then(|text| text.as_str())
                == Some(PARKED)
        });
        let entry_id = parked_entry
            .and_then(|item| item.get("entry_id"))
            .and_then(|entry_id| entry_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("parked follow-up not found in transcript"))?;
        anyhow::ensure!(
            entry_id.starts_with("e_q_"),
            "follow-up was not delivered from the queue: entry {entry_id}"
        );
        Ok(())
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::JsonMatcherV1;

    #[test]
    fn parks_before_a_failing_terminal_generation_and_expects_a_reseeded_turn() {
        let fixture = scenario();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.expected_turn_statuses, ["failed", "completed"]);
        assert_eq!(fixture.expected_traces(), 1);

        // The parked steer rides generation 1, which then fails without
        // streaming — the deterministic route into the finalize drain.
        let first = &fixture.script.generations[0];
        let effect = first
            .on_serve
            .as_ref()
            .expect("generation 1 parks a message");
        assert_eq!(effect.steer.session_id, "{{session_id}}");
        assert!(!effect.steer.message.is_empty());
        assert!(first.failure.is_some());
        assert!(first.frames.is_empty());
        assert!(!first.response.ok);

        // Generation 2 is the reseeded turn's own step 0, with no side effect.
        let second = &fixture.script.generations[1];
        assert!(second.on_serve.is_none());
        assert!(second.failure.is_none());
        let JsonMatcherV1::Regex { pattern } = &second.match_.request_id else {
            panic!("request id must be a regex");
        };
        assert!(pattern.ends_with(":0$"), "{pattern}");
    }
}
