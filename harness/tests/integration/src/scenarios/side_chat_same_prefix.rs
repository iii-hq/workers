//! INT-046 — a side chat sends the conversation's system prompt and tools.
//!
//! chat-projects §2.2: the conversation runs one turn. The probe then opens
//! a side chat of it (`harness::side-chat::open`, forked at the reply) and
//! sends a message in it. The side chat's first router request carries the
//! conversation's system prompt byte for byte — its frozen runtime context
//! included, the conversation's session id line and all, so the request
//! matches the same `{{system_prompt_sha256}}` — and the same system
//! sections and tools; its history starts with the conversation's up to the
//! reply. That is the prefix a provider's prompt cache keys on. Nothing is
//! written into the conversation.

use serde_json::{Value, json};

use super::ScenarioDriver;
use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use crate::evidence_data::RunEvidence;
use crate::fixtures::{SIDE_CHAT_ACTION, ScenarioFixture};

// Shared with UI-007 (`console_side_chat`), whose side chat the Console
// opens and sends in instead of the probe.
pub(super) const SET_DESCRIPTION: &str = "harness::set-description";
pub(super) const MESSAGE: &str = "The webhook retries five times before it gives up; explain why.";
pub(super) const REPLY: &str = "Five attempts cover the provider's usual outage window.";
pub(super) const SIDE_MESSAGE: &str = "Why five attempts and not eight?";
pub(super) const SIDE_REPLY: &str = "Eight would outlast the delivery receipt's expiry.";

/// The conversation's generation: its only turn answers [`REPLY`].
pub(super) fn conversation_generation() -> Generation {
    Generation::new(1)
        .expect(
            Request::new()
                .turn_request_step(0)
                .system_prompt_sha256("{{system_prompt_sha256}}")
                .messages_exact([Message::user(MESSAGE)])
                .tools_subset([]),
        )
        .respond(Response::text(REPLY, 10, 6))
}

/// The side chat's generation: the opening request of a turn in another
/// session, with the CONVERSATION's system prompt (its session id, not the
/// side chat's) and history up to the reply; it answers [`SIDE_REPLY`].
pub(super) fn side_chat_generation() -> Generation {
    Generation::new(2)
        .expect(
            Request::new()
                .turn_request_step(0)
                .system_prompt_sha256("{{system_prompt_sha256}}")
                .messages_subset([
                    Message::user(MESSAGE),
                    json!({ "role": "assistant",
                            "content": [{ "type": "text", "text": REPLY }] }),
                ])
                .tools_subset([]),
        )
        .respond(Response::text(SIDE_REPLY, 12, 6))
}

/// The run's two router requests — the conversation's and the side chat's —
/// carry the same system prompt, system sections, tools, model and
/// provider; the side chat's history is the conversation's up to the reply,
/// then [`SIDE_MESSAGE`]; nothing of the side chat reached the conversation;
/// exactly two generations ran.
pub(super) fn verify_side_chat(run: &RunEvidence) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.generations_consumed == 2 && run.generations_total == 2,
        "expected the conversation's and the side chat's router calls, got {} of {}",
        run.generations_consumed,
        run.generations_total
    );
    let requests: Vec<&Value> = run
        .router_evidence
        .get("calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|call| call["outcome"] == "matched")
        .filter_map(|call| call.get("request"))
        .collect();
    anyhow::ensure!(
        requests.len() == 2,
        "expected two matched router requests, got {}",
        requests.len()
    );
    let main = requests
        .iter()
        .find(|request| request["session_id"] == run.session_id.as_str())
        .ok_or_else(|| anyhow::anyhow!("no router request of the conversation"))?;
    let side = requests
        .iter()
        .find(|request| request["session_id"] != run.session_id.as_str())
        .ok_or_else(|| anyhow::anyhow!("no router request of the side chat"))?;
    for key in ["system_prompt", "system_sections", "tools"] {
        anyhow::ensure!(
            main[key] == side[key],
            "the side chat's {key} differs from the conversation's:\n{}\n---\n{}",
            main[key],
            side[key]
        );
    }
    anyhow::ensure!(
        main["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "the conversation offered no tools to compare: {}",
        main["tools"]
    );
    anyhow::ensure!(
        main["model"] == side["model"] && main["provider"] == side["provider"],
        "the side chat ran on another model: {} / {}",
        side["model"],
        side["provider"]
    );
    let texts = |request: &Value| -> Vec<String> {
        request["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(crate::evidence_data::message_text)
            .collect()
    };
    let side_texts = texts(side);
    anyhow::ensure!(
        side_texts.first().map(String::as_str) == Some(MESSAGE)
            && side_texts.get(1).map(String::as_str) == Some(REPLY)
            && side_texts[2..]
                .iter()
                .any(|text| text.contains(SIDE_MESSAGE)),
        "the side chat's history is not the conversation's up to the reply, then its \
         message: {side_texts:?}"
    );
    // Nothing of the side chat reached the conversation.
    let leaked = run.transcript.iter().any(|item| {
        let text = crate::evidence_data::message_text(&item["message"]);
        text.contains(SIDE_MESSAGE) || text.contains(SIDE_REPLY)
    });
    anyhow::ensure!(!leaked, "the side chat wrote into the conversation");
    run.expect_assistant_texts([REPLY])?;
    run.expect_no_duplicate_messages()
}

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-046";

    Scenario::new(
        ID,
        "side-chat-same-prefix",
        "A side chat opened at the conversation's reply sends its first request with the \
         conversation's system prompt (frozen runtime context included), system sections and \
         tools, its history starting with the conversation's, and writes nothing into the \
         conversation.",
        ScenarioDriver::Direct,
        Model::scripted("fixture-model"),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-046")
            .allow_id(SET_DESCRIPTION),
    )
    .generation(conversation_generation())
    .probe_after(1, SIDE_CHAT_ACTION, json!({ "message": SIDE_MESSAGE }))
    .generation(side_chat_generation())
    // The side chat's turn traces under its own session, not the run's.
    .expect_traces(1)
    .verify(verify_side_chat)
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_the_conversation_then_the_side_chat_behind_a_probe_action() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.script.generations.len(), 2);
        assert_eq!(fixture.probe_actions.len(), 1);
        assert_eq!(fixture.probe_actions[0].function_id, SIDE_CHAT_ACTION);
        assert_eq!(fixture.probe_actions[0].after_turns, 1);
        assert_eq!(fixture.expected_traces(), 1);
        let side = serde_json::to_string(&fixture.script.generations[1]).unwrap();
        assert!(side.contains("{{system_prompt_sha256}}"), "{side}");
        assert!(side.contains(REPLY), "{side}");
    }
}
