//! INT-012 — edit one queued message in place and remove another before the
//! first streamed turn is released.
//!
//! The scripted router holds the first `router::chat` call before its first
//! frame. The runner then uses only the public harness surface to enqueue four
//! messages, captures their client-visible `entry_id`s from `harness::status`,
//! edits one row, removes another, and releases the generation. The second
//! scripted step proves that exactly the edited row and the two untouched rows
//! reached the model in the surviving queue order.

use serde_json::json;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-012";
const SLUG: &str = "queued-message-edit-unqueue";
const INITIAL_MESSAGE: &str = "Start the queued-message edit test.";
const BEFORE_MESSAGE: &str = "queued before edit";
const EDIT_MESSAGE: &str = "original queued text";
const EDIT_REPLACEMENT: &str = "edited queued text";
const REMOVE_MESSAGE: &str = "remove this queued message";
const AFTER_MESSAGE: &str = "queued after edit";
const FIRST_TEXT: &str = "first turn streamed";
const FINAL_TEXT: &str = "queued edit and unqueue complete";
const GATE: &str = "queued-message-edit-unqueue-in-flight";

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        SLUG,
        "Editing and unqueuing messages during a running turn preserves the edited row's position and drains only surviving messages.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(INITIAL_MESSAGE).idempotency_key("{{run_id}}:integration-012"),
    )
    .queued_message_edit_unqueue(
        GATE,
        BEFORE_MESSAGE,
        EDIT_MESSAGE,
        EDIT_REPLACEMENT,
        REMOVE_MESSAGE,
        AFTER_MESSAGE,
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(INITIAL_MESSAGE)])
                    .without_tools(),
            )
            .gate(GATE)
            .respond(Response::streamed_text(
                FIRST_TEXT,
                ["first turn ", "streamed"],
                10,
                3,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        Message::user(INITIAL_MESSAGE),
                        json!({ "role": "assistant" }),
                        Message::user(BEFORE_MESSAGE),
                        Message::user(EDIT_REPLACEMENT),
                        Message::user(AFTER_MESSAGE),
                    ])
                    .without_tools(),
            )
            .respond(Response::text(FINAL_TEXT, 24, 5)),
    )
    .verify(verify)
    .scenario_timeout_ms(60_000)
    .build()
}

fn verify(run: &crate::evidence_data::RunEvidence) -> anyhow::Result<()> {
    run.expect_assistant_texts([FIRST_TEXT, FINAL_TEXT])?;
    run.expect_message_counts(4, 2, 0)?;
    run.expect_no_duplicate_messages()?;

    anyhow::ensure!(
        run.generations_consumed == 2 && run.generations_total == 2,
        "{} of {} scripted generations consumed",
        run.generations_consumed,
        run.generations_total
    );
    anyhow::ensure!(
        run.status.get("status").and_then(serde_json::Value::as_str) == Some("completed")
            && run
                .status
                .get("queued")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty),
        "final status must be completed with an empty queue: {}",
        run.status
    );

    let user_texts = run
        .transcript
        .iter()
        .filter_map(|item| item.get("message"))
        .filter(|message| message.get("role").and_then(serde_json::Value::as_str) == Some("user"))
        .map(message_text)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        user_texts
            == [
                INITIAL_MESSAGE.to_string(),
                BEFORE_MESSAGE.to_string(),
                EDIT_REPLACEMENT.to_string(),
                AFTER_MESSAGE.to_string(),
            ],
        "durable user transcript has the wrong order/content: {user_texts:?}"
    );
    let transcript = serde_json::to_string(&run.transcript)?;
    anyhow::ensure!(
        !transcript.contains(EDIT_MESSAGE),
        "the pre-edit text reached the transcript"
    );
    anyhow::ensure!(
        !transcript.contains(REMOVE_MESSAGE),
        "the unqueued text reached the transcript"
    );

    let control = &run.control;
    anyhow::ensure!(
        control.get("kind").and_then(serde_json::Value::as_str) == Some(SLUG)
            && control
                .pointer("/gate/name")
                .and_then(serde_json::Value::as_str)
                == Some(GATE)
            && control
                .pointer("/gate/arrived")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
            && control.pointer("/gate/released") == Some(&serde_json::Value::Bool(true))
            && control.pointer("/edit_response/updated") == Some(&serde_json::Value::Bool(true))
            && control.pointer("/unqueue_response/removed") == Some(&serde_json::Value::Bool(true)),
        "queued-message intervention hard gates failed: {}",
        control
    );

    let edit_entry_id = control
        .pointer("/edit_target/entry_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("edit target entry_id is missing from control evidence"))?;
    let removed_entry_id = control
        .pointer("/unqueue_target/entry_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            anyhow::anyhow!("unqueue target entry_id is missing from control evidence")
        })?;
    anyhow::ensure!(
        edit_entry_id.starts_with("e_")
            && !edit_entry_id.starts_with("q_")
            && removed_entry_id.starts_with("e_")
            && !removed_entry_id.starts_with("q_")
            && edit_entry_id != removed_entry_id,
        "intervention did not use distinct client-visible entry ids: edit={edit_entry_id}, remove={removed_entry_id}"
    );
    anyhow::ensure!(
        control.pointer("/edit_position_preserved") == Some(&serde_json::Value::Bool(true))
            && control.pointer("/unqueue_absent_after_call")
                == Some(&serde_json::Value::Bool(true))
            && control.pointer("/post_unqueue_queue/0/text")
                == Some(&serde_json::Value::String(BEFORE_MESSAGE.to_string()))
            && control.pointer("/post_unqueue_queue/1/text")
                == Some(&serde_json::Value::String(EDIT_REPLACEMENT.to_string()))
            && control.pointer("/post_unqueue_queue/2/text")
                == Some(&serde_json::Value::String(AFTER_MESSAGE.to_string())),
        "queue edit/remove evidence does not prove surviving order/content: {}",
        control
    );
    Ok(())
}
