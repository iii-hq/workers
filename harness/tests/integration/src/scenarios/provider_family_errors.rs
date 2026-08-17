//! UI-003..005 — representative provider-protocol failures stay actionable
//! through the Harness and Console boundary.
//!
//! Provider-specific request and error parsing lives in the hermetic provider
//! contract suite. These fixtures begin at the normalized router boundary and
//! pin the user-facing behavior shared by each protocol family: a permanent
//! generation failure finalizes the turn with one stable public summary while
//! preserving the provider-specific diagnostic detail in the durable failure
//! record. That contract must remain present after a later turn completes.

use serde_json::Value;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send, Tool};
use super::{ScenarioDriver, VerifyFn};
use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

const ANTHROPIC_REASON: &str = "anthropic messages: credit balance is too low";
const CHAT_REASON: &str = "openai chat completions: insufficient quota";
const RESPONSES_REASON: &str = "openai responses: credit balance exhausted";
const PUBLIC_SUMMARY: &str = "The provider rejected this request.";
const NEXT_ACTION: &str = "Review the selected model and provider settings, then try again.";
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
        "A permanent provider failure persists one stable public summary and its provider-specific diagnostic detail.",
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

    let durable_errors = run
        .transcript
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let custom = item.get("custom")?;
            (custom.get("custom_type").and_then(Value::as_str) == Some("error"))
                .then_some((index, item, custom))
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        durable_errors.len() == 1,
        "expected one durable error record after recovery, found {}: {:?}",
        durable_errors.len(),
        run.transcript
    );
    let (error_index, item, error) = durable_errors[0];
    anyhow::ensure!(
        item.get("entry_id")
            .and_then(Value::as_str)
            .is_some_and(|entry_id| entry_id.starts_with("e_") && entry_id.ends_with("_error")),
        "durable error record has no stable error entry id: {item}"
    );
    let data = error.get("data").cloned().unwrap_or(Value::Null);
    anyhow::ensure!(
        data.get("code").and_then(Value::as_str) == Some("invocation_failed"),
        "failure code did not preserve the router invocation error: {data}"
    );
    anyhow::ensure!(
        data.get("class").and_then(Value::as_str) == Some("llm.permanent"),
        "failure class is not permanent: {data}"
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
        data.get("summary").and_then(Value::as_str) == Some(PUBLIC_SUMMARY),
        "failure summary is not the stable public message: {data}"
    );
    anyhow::ensure!(
        !data
            .get("summary")
            .and_then(Value::as_str)
            .is_some_and(|summary| summary.contains(expected_reason)),
        "failure summary contains provider-specific diagnostic detail: {data}"
    );
    anyhow::ensure!(
        data.get("detail")
            .and_then(Value::as_str)
            .is_some_and(|detail| detail.contains(expected_reason)),
        "failure detail does not preserve the provider reason: {data}"
    );
    anyhow::ensure!(
        data.get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| reason.contains(expected_reason)),
        "compatibility reason does not preserve the provider reason: {data}"
    );
    anyhow::ensure!(
        data.get("next_actions")
            .and_then(Value::as_array)
            .is_some_and(|actions| {
                actions.len() == 1 && actions[0].as_str() == Some(NEXT_ACTION)
            }),
        "failure next actions are not actionable and stable: {data}"
    );

    let recovery_index = run
        .transcript
        .iter()
        .enumerate()
        .find_map(|(index, item)| {
            let message = item.get("message")?;
            (message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks.iter().any(|block| {
                            block.get("type").and_then(Value::as_str) == Some("text")
                                && block.get("text").and_then(Value::as_str) == Some(RECOVERY_TEXT)
                        })
                    }))
            .then_some(index)
        })
        .ok_or_else(|| anyhow::anyhow!("recovery assistant message is missing"))?;
    anyhow::ensure!(
        error_index < recovery_index,
        "durable failure record was not retained before the completed recovery turn"
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

    #[test]
    fn provider_specific_reasons_share_one_public_failure_contract() {
        assert_eq!(PUBLIC_SUMMARY, "The provider rejected this request.");
        assert_eq!(
            NEXT_ACTION,
            "Review the selected model and provider settings, then try again."
        );
        for reason in [ANTHROPIC_REASON, CHAT_REASON, RESPONSES_REASON] {
            assert_ne!(reason, PUBLIC_SUMMARY);
            assert!(!PUBLIC_SUMMARY.contains(reason));
        }
    }
}
