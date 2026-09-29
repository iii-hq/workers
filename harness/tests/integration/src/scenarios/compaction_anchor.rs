//! INT-035 — the turn loop anchors on the latest `compaction` session entry
//! whatever its boundary says, instead of assembling an empty context.
//!
//! The bug class this pins: the anchor's `tail_start_entry_id` names an entry
//! that is not on the active path (a session forked before `session::fork`
//! rewrote the anchor, a hand-written record) and the candidate window never
//! opened, so `context::assemble` got no messages — not even the user's new
//! one — and the turn died with a misleading `context/overflow`.
//!
//! Two probe-written records drive the two non-trivial branches:
//! - a boundary that is NOT on the path → the whole path is sent, under the
//!   record's summary;
//! - a `null` boundary (everything before the record was summarised) → only
//!   the entries after the record are sent, under the newer summary.
//!
//! The context snapshot must report the summary either way.

use serde_json::json;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-035";
const FIRST_MESSAGE: &str = "Start the anchor drill.";
const SECOND_MESSAGE: &str = "Continue the anchor drill after a foreign boundary.";
const THIRD_MESSAGE: &str = "Continue the anchor drill after a null boundary.";
const FIRST_TEXT: &str = "anchor drill started";
const SECOND_TEXT: &str = "anchor drill continued on the whole path";
const THIRD_TEXT: &str = "anchor drill continued on the tail";
const FOREIGN_SUMMARY: &str = "Summary one: the drill began and the operator wants brevity.";
const NULL_SUMMARY: &str = "Summary two: everything before this record was summarised.";
const NULL_HEAD_TOKENS: u64 = 77;

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "compaction-anchor",
        "A compaction record whose boundary is off the path anchors the next turn on the whole path; \
         a null boundary opens the window after the record; the snapshot reports the summary.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(FIRST_MESSAGE)
            .idempotency_key("{{run_id}}:integration-035")
            .without_functions(),
    )
    .terminal_turns(3)
    // Only the two follow-up sends make turns; the appends leave no trace.
    .expect_traces(3)
    .probe_after(
        1,
        "session::append",
        json!({
            "session_id": "{{session_id}}",
            "custom": { "custom_type": "compaction", "data": {
                "summary": FOREIGN_SUMMARY,
                "tail_start_entry_id": "e_not_on_this_path",
                "tokens_before": 50
            } }
        }),
    )
    .probe_after(
        1,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": SECOND_MESSAGE,
            "idempotency_key": "{{run_id}}:integration-035-b"
        }),
    )
    .probe_after(
        2,
        "session::append",
        json!({
            "session_id": "{{session_id}}",
            "custom": { "custom_type": "compaction", "data": {
                "summary": NULL_SUMMARY,
                "tail_start_entry_id": null,
                "tokens_before": NULL_HEAD_TOKENS
            } }
        }),
    )
    .probe_after(
        2,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": THIRD_MESSAGE,
            "idempotency_key": "{{run_id}}:integration-035-c"
        }),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(FIRST_MESSAGE)])
                    .without_tools(),
            )
            .respond(Response::text(FIRST_TEXT, 12, 4)),
    )
    // Foreign boundary: the summary rides the system prompt and the WHOLE
    // path is the window — the pre-fix loop sent nothing here.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(&summary_pattern(FOREIGN_SUMMARY))
                    .messages_exact([
                        Message::user(FIRST_MESSAGE),
                        Message::assistant_text(FIRST_TEXT, &model, 12, 4),
                        Message::user(SECOND_MESSAGE),
                    ])
                    .without_tools(),
            )
            .respond(Response::text(SECOND_TEXT, 18, 5)),
    )
    // Null boundary: the LATEST record wins, and only what follows it is sent.
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(&summary_pattern(NULL_SUMMARY))
                    .messages_exact([Message::user(THIRD_MESSAGE)])
                    .without_tools(),
            )
            .respond(Response::text(THIRD_TEXT, 9, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts([FIRST_TEXT, SECOND_TEXT, THIRD_TEXT])?;
        run.expect_message_counts(3, 3, 0)?;
        anyhow::ensure!(
            run.generations_consumed == 3 && run.generations_total == 3,
            "{} of {} scripted generations consumed",
            run.generations_consumed,
            run.generations_total
        );
        let records = run
            .transcript
            .iter()
            .filter(|item| item["custom"]["custom_type"] == "compaction")
            .count();
        anyhow::ensure!(records == 2, "transcript holds {records} compaction records, expected 2");
        // The snapshot describes the anchored context: "compacted" even though
        // this step compacted nothing itself, with the record's head size.
        let context = &run.status["context"];
        anyhow::ensure!(
            context["compacted"] == true,
            "context snapshot is not marked compacted: {context}"
        );
        anyhow::ensure!(
            context["summarized_head_tokens"] == NULL_HEAD_TOKENS,
            "summarized_head_tokens is {}, expected {NULL_HEAD_TOKENS}",
            context["summarized_head_tokens"]
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

/// The context-manager renders `previous_summary` under its own heading at
/// the end of the system prompt; the summaries carry no regex metacharacters.
fn summary_pattern(summary: &str) -> String {
    format!("(?s)# Conversation summary\\n\\n{summary}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drives_two_anchored_turns_from_probe_written_records() {
        let fixture = scenario();
        assert_eq!(fixture.expected_terminal_turns, 3);
        let actions: Vec<(usize, &str)> = fixture
            .probe_actions
            .iter()
            .map(|action| (action.after_turns, action.function_id.as_str()))
            .collect();
        assert_eq!(
            actions,
            [
                (1, "session::append"),
                (1, "harness::send"),
                (2, "session::append"),
                (2, "harness::send"),
            ]
        );
        assert_eq!(
            fixture.probe_actions[0].payload["custom"]["data"]["tail_start_entry_id"],
            "e_not_on_this_path"
        );
        assert!(
            fixture.probe_actions[2].payload["custom"]["data"]["tail_start_entry_id"].is_null()
        );
        assert!(regex::Regex::new(&summary_pattern(FOREIGN_SUMMARY))
            .unwrap()
            .is_match(&format!(
                "base prompt\n\n# Conversation summary\n\n{FOREIGN_SUMMARY}"
            )));
    }
}
