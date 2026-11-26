//! INT-049 — the end of a turn compacts a conversation past the global
//! autocompact setting (chat-projects phase 3, D158/D159), against the
//! context-manager the stack runs.
//!
//! A setup call turns autocompact on at its 20k floor. The scripted model's
//! window is 32,768 tokens, so context-manager's `usable` (25,396) is above
//! the setting and the setting applies. Three short turns run. The
//! snapshot's `total` stays context-manager's estimate here (about 7k: the
//! scripted router counts no tokens, so `exactify` never applies the billed
//! numbers), while the third step bills 21,000 input tokens: the step's
//! billed input decides, so before the third turn completes the harness
//! calls `context::compact`, and context-manager summarises the head (the
//! first exchange) through the router, keeping the rest verbatim.
//! The transcript gets the start notice, then the `compaction` record with
//! contract C's fields (`context_before` is the billed input plus the
//! step's output), and the turn completes after them. A context-manager
//! without the budget echo (older than 1.3.0) runs these turns with no cap
//! inside the turn; the end of the turn compacts either way.

use serde_json::{Value, json};

use super::ScenarioDriver;
use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-049";
const SETTING: u64 = 20_000;
const FIRST_MESSAGE: &str = "Plan the warehouse migration.";
const SECOND_MESSAGE: &str = "List the risks of the cutover.";
const THIRD_MESSAGE: &str = "Draft the rollback steps.";
const FIRST_TEXT: &str = "The migration runs in three waves.";
const SECOND_TEXT: &str = "The cutover risks are data drift and downtime.";
const THIRD_TEXT: &str = "Rollback restores the snapshot and replays the queue.";
const SUMMARY: &str = "The user plans a warehouse migration in three waves.";
/// The third step's billed input and output: past the setting, while the
/// estimated `total` is about 7k.
const THIRD_INPUT: u64 = 21_000;
const THIRD_OUTPUT: u64 = 40;

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("fixture-model");
    let turn = |messages: Vec<Value>| {
        Request::new()
            .turn_request_step(0)
            .system_prompt_sha256("{{system_prompt_sha256}}")
            .messages_exact(messages)
            .without_tools()
    };

    Scenario::new(
        ID,
        "autocompact-turn-end",
        "With autocompact on at 20k, a turn whose step billed 21k input tokens (the \
         estimated total is about 7k) is compacted before it completes: context-manager \
         summarises the first exchange, and the transcript gets the start notice, then a \
         compaction record with trigger auto, the threshold, the global rule and the context \
         before and after.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .setup_call(
        "harness::chat-settings::set",
        json!({ "patch": { "autocompact": { "mode": "tokens", "tokens": SETTING } } }),
        json!({ "settings": { "autocompact": { "mode": "tokens", "tokens": SETTING } } }),
    )
    .send(
        Send::message(FIRST_MESSAGE)
            .idempotency_key("{{run_id}}:integration-049")
            .without_functions(),
    )
    .terminal_turns(3)
    .expect_traces(3)
    .probe_after(
        1,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": SECOND_MESSAGE,
            "idempotency_key": "{{run_id}}:integration-049-b"
        }),
    )
    .probe_after(
        2,
        "harness::send",
        json!({
            "session_id": "{{session_id}}",
            "message": THIRD_MESSAGE,
            "idempotency_key": "{{run_id}}:integration-049-c"
        }),
    )
    .generation(
        Generation::new(1)
            .expect(turn(vec![Message::user(FIRST_MESSAGE)]))
            .respond(Response::text(FIRST_TEXT, 12, 6)),
    )
    .generation(
        Generation::new(2)
            .expect(turn(vec![
                Message::user(FIRST_MESSAGE),
                Message::assistant_text(FIRST_TEXT, &model, 12, 6),
                Message::user(SECOND_MESSAGE),
            ]))
            .respond(Response::text(SECOND_TEXT, 30, 8)),
    )
    .generation(
        Generation::new(3)
            .expect(turn(vec![
                Message::user(FIRST_MESSAGE),
                Message::assistant_text(FIRST_TEXT, &model, 12, 6),
                Message::user(SECOND_MESSAGE),
                Message::assistant_text(SECOND_TEXT, &model, 30, 8),
                Message::user(THIRD_MESSAGE),
            ]))
            .respond(Response::text(THIRD_TEXT, THIRD_INPUT, THIRD_OUTPUT)),
    )
    // context-manager's summariser, at the end of the third turn.
    .generation(
        Generation::new(4)
            .expect(Request::summary())
            .respond(Response::text(SUMMARY, 90, 14)),
    )
    .verify(verify)
    .build()
}

fn custom<'a>(run: &'a RunEvidence, kind: &str) -> Vec<&'a Value> {
    run.transcript
        .iter()
        .filter(|item| item["custom"]["custom_type"] == kind)
        .map(|item| &item["custom"]["data"])
        .collect()
}

fn verify(run: &RunEvidence) -> anyhow::Result<()> {
    run.expect_assistant_texts([FIRST_TEXT, SECOND_TEXT, THIRD_TEXT])?;
    run.expect_message_counts(3, 3, 0)?;
    run.expect_no_duplicate_messages()?;
    anyhow::ensure!(
        run.generations_consumed == 4 && run.generations_total == 4,
        "{} of {} scripted generations consumed",
        run.generations_consumed,
        run.generations_total
    );

    // The summariser got the head (the first exchange; context-manager may
    // cut inside the second turn), never the third turn.
    let summary_requests: Vec<&Value> = run
        .router_evidence
        .get("calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| call.get("request"))
        .filter(|request| request.get("request_id").is_none())
        .collect();
    let [summary_request] = summary_requests.as_slice() else {
        anyhow::bail!("expected one summariser request, found {summary_requests:?}")
    };
    let head = summary_request["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    anyhow::ensure!(
        head.contains(FIRST_MESSAGE) && head.contains(FIRST_TEXT) && !head.contains(THIRD_MESSAGE),
        "the summariser's transcript is not the head: {}",
        head.chars().take(400).collect::<String>()
    );

    // One record, with contract C's fields.
    let records = custom(run, "compaction");
    let [record] = records.as_slice() else {
        anyhow::bail!("expected one compaction record, found {records:?}")
    };
    let context_before = record["context_before"].as_u64().unwrap_or_default();
    let context_after = record["context_after"].as_u64().unwrap_or_default();
    anyhow::ensure!(
        record["summary"] == SUMMARY
            && record["trigger"] == "auto"
            && record["source"] == "auto"
            && record["threshold_tokens"] == SETTING
            && record["rule"] == json!({ "scope": "global" })
            && context_before == THIRD_INPUT + THIRD_OUTPUT
            && context_after > 0
            && context_after < context_before,
        "unexpected compaction record: {record}"
    );

    // The start notice, then the record: a compaction that succeeds closes
    // with its record (the marker, "Auto-compacted at 20k · 21k → … tokens
    // · global rule", rides `harness::on-compaction`), not a second notice.
    let notices = custom(run, "notice");
    let [started] = notices.as_slice() else {
        anyhow::bail!("expected the start notice alone, found {notices:?}")
    };
    anyhow::ensure!(
        started["message"] == "Auto-compacting at 20k…"
            && started["reason"] == "compacting"
            && started["trigger"] == "auto"
            && started["source"] == "auto"
            && started["threshold_tokens"] == SETTING
            && started["rule"] == json!({ "scope": "global" })
            && started["compaction_id"] == record["compaction_id"],
        "unexpected start notice: {started}"
    );
    let position = |kind: &str| {
        run.transcript
            .iter()
            .position(|item| item["custom"]["custom_type"] == kind)
    };
    anyhow::ensure!(
        position("notice") < position("compaction"),
        "the record precedes the start notice"
    );
    anyhow::ensure!(
        context_before >= SETTING && context_after < context_before,
        "the context did not shrink: {context_before} → {context_after}"
    );

    // The status reads the refreshed snapshot, which keeps the numbers the
    // settings resolved to.
    let context = &run.status["context"];
    anyhow::ensure!(
        context["compacted"] == true
            && context["autocompact"]["setting_tokens"] == SETTING
            && context["autocompact"]["effective_tokens"] == SETTING
            && context["autocompact"]["rule"] == json!({ "scope": "global" }),
        "unexpected context snapshot: {context}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_autocompact_on_and_scripts_the_summariser_last() {
        let fixture = scenario();
        assert_eq!(fixture.expected_terminal_turns, 3);
        assert_eq!(
            fixture
                .setup_calls
                .iter()
                .map(|call| call.function_id.as_str())
                .collect::<Vec<_>>(),
            ["harness::chat-settings::set"]
        );
        let summariser = &fixture.script.generations[3].match_;
        assert_eq!(
            summariser.request_id,
            crate::types::script::JsonMatcherV1::Absent
        );
    }
}
