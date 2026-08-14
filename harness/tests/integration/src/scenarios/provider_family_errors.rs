//! UI-003..005 — representative provider-protocol failures stay actionable
//! through the Harness and Console boundary.
//!
//! Provider-specific request and error parsing lives in the hermetic provider
//! contract suite. These fixtures begin at the normalized router boundary and
//! pin the user-facing behavior shared by each protocol family: a permanent
//! generation failure finalizes the turn, persists structured failure data,
//! and remains visible after Console transcript reconciliation.

use serde_json::Value;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send, Tool};
use super::{ScenarioDriver, VerifyFn};
use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

const ANTHROPIC_REASON: &str = "anthropic messages: credit balance is too low";
const CHAT_REASON: &str = "openai chat completions: insufficient quota";
const RESPONSES_REASON: &str = "openai responses: credit balance exhausted";
const RECOVERY_MESSAGE: &str =
    "Confirm the chat can continue after the provider issue is corrected.";
const RECOVERY_TEXT: &str = "provider family recovery complete";

struct FamilyCase {
    id: &'static str,
    slug: &'static str,
    model: &'static str,
    reason: &'static str,
    verify: VerifyFn,
}

pub(super) fn scenarios() -> Vec<ScenarioFixture> {
    [
        FamilyCase {
            id: "UI-003",
            slug: "console-anthropic-messages-error",
            model: "anthropic-messages-fixture",
            reason: ANTHROPIC_REASON,
            verify: verify_anthropic,
        },
        FamilyCase {
            id: "UI-004",
            slug: "console-openai-chat-error",
            model: "openai-chat-completions-fixture",
            reason: CHAT_REASON,
            verify: verify_chat,
        },
        FamilyCase {
            id: "UI-005",
            slug: "console-openai-responses-error",
            model: "openai-responses-fixture",
            reason: RESPONSES_REASON,
            verify: verify_responses,
        },
    ]
    .into_iter()
    .map(scenario)
    .collect()
}

fn scenario(case: FamilyCase) -> ScenarioFixture {
    let message = format!("Exercise the {} failure path.", case.model);
    let model = Model::scripted(case.model);
    Scenario::new(
        case.id,
        case.slug,
        "A permanent provider failure is persisted and shown as an actionable Console notice.",
        ScenarioDriver::Playground,
        model.clone(),
    )
    .send(
        Send::message(&message)
            .idempotency_key(&format!("{{{{run_id}}}}:{}", case.slug))
            .without_functions(),
    )
    .terminal_turn_statuses(["failed", "completed"])
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([Message::user(&message)])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .fails(case.reason),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("agent_trigger")
                    .messages_exact([
                        Message::user(&message),
                        Message::assistant_empty(&model),
                        Message::user(RECOVERY_MESSAGE),
                    ])
                    .tools_subset([Tool::named("agent_trigger")]),
            )
            .respond(Response::text(RECOVERY_TEXT, 12, 4)),
    )
    .verify(case.verify)
    .build()
}

fn verify_anthropic(run: &RunEvidence) -> anyhow::Result<()> {
    verify_permanent_failure(run, ANTHROPIC_REASON)
}

fn verify_chat(run: &RunEvidence) -> anyhow::Result<()> {
    verify_permanent_failure(run, CHAT_REASON)
}

fn verify_responses(run: &RunEvidence) -> anyhow::Result<()> {
    verify_permanent_failure(run, RESPONSES_REASON)
}

fn verify_permanent_failure(run: &RunEvidence, expected_reason: &str) -> anyhow::Result<()> {
    run.expect_assistant_texts([RECOVERY_TEXT])?;
    run.expect_message_counts(2, 2, 0)?;
    run.expect_no_duplicate_messages()?;

    let error = run
        .transcript
        .iter()
        .filter_map(|item| item.get("custom"))
        .find(|custom| custom.get("custom_type").and_then(Value::as_str) == Some("error"))
        .ok_or_else(|| anyhow::anyhow!("durable error record is missing"))?;
    let data = error.get("data").cloned().unwrap_or(Value::Null);
    anyhow::ensure!(
        data.get("code").and_then(Value::as_str) == Some("llm.permanent"),
        "failure code is not permanent: {data}"
    );
    anyhow::ensure!(
        data.get("retryable").and_then(Value::as_bool) == Some(false),
        "permanent failure is marked retryable: {data}"
    );
    anyhow::ensure!(
        data.get("phase").and_then(Value::as_str) == Some("generation"),
        "failure phase is not generation: {data}"
    );
    anyhow::ensure!(
        data.get("summary")
            .and_then(Value::as_str)
            .is_some_and(|summary| summary.contains(expected_reason)),
        "failure summary does not preserve the provider reason: {data}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_each_protocol_family_with_a_permanent_terminal_failure() {
        let fixtures = scenarios();
        assert_eq!(fixtures.len(), 3);
        for fixture in fixtures {
            fixture.validate().unwrap();
            assert_eq!(fixture.expected_turn_statuses, ["failed", "completed"]);
            assert_eq!(fixture.script.generations.len(), 2);
            let failed = &fixture.script.generations[0];
            assert!(failed.failure.is_some());
            assert!(failed.frames.is_empty());
            assert!(!failed.response.ok);
            assert!(fixture.script.generations[1].response.ok);
        }
    }
}
