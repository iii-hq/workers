//! INT-014 — a frozen `max_total_tokens` budget admits the first generation
//! and rejects the second at budget preflight.
//!
//! Determinism without calibrating the (unknowable) system-prompt size: the
//! controlled function returns a ~240k-character result, so generation 2's
//! reservation (`assembled_input_estimate + max_output`) exceeds the 30k
//! budget by tens of thousands of tokens no matter what the prompt costs,
//! while generation 1's reservation (prompt + one short message + 4_096
//! max-output) stays far below it. The fixture model's context window is
//! widened so context assembly never prunes the fat result away before the
//! budget check.
//!
//! Covered: `budget.rs` prepare_root/reserve/reconcile/release + Exceeded,
//! `turn_loop::finalize_failed` (durable `custom_type:"error"` entry, failed
//! terminal turn), and the `harness_budget` state ledger read through a
//! recorded probe response.

use serde_json::{json, Value};

use super::dsl::{ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const BUDGET_TOKENS: u64 = 30_000;
/// Scripted generation-1 usage: 8 input + 4 output.
const EXPECTED_USED_TOKENS: u64 = 12;
const FAT_RESULT_CHARS: usize = 240_000;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-014";
    const MESSAGE: &str = "Fetch the archive; the budget will not survive it.";

    let mut model = Model::scripted("fixture-model");
    // Wide enough that assembly keeps the fat function result intact and the
    // budget check — not a context overflow — is what stops the turn.
    model.context_window = 400_000;

    let archive = ControlledFunction::new("{{run_id}}::archive", "Return one huge archive blob.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {},
        }))
        .returns_text(&"x".repeat(FAT_RESULT_CHARS));

    Scenario::new(
        ID,
        "budget-preflight-exceeded",
        "A frozen token budget admits generation 1, then fails the turn at budget preflight \
         when the fat function result blows generation 2's reservation.",
        ScenarioDriver::Direct,
        model,
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-014")
            .allow_function(&archive)
            .max_total_tokens(BUDGET_TOKENS),
    )
    .function(archive.clone())
    .terminal_turn_statuses(["failed"])
    .expect_traces(1)
    // The ledger survives the failed turn: generation 1 reconciled its actual
    // usage and the rejected generation-2 reservation was never charged.
    .probe_after(
        1,
        "state::get",
        json!({ "scope": "harness_budget", "key": "{{session_id}}" }),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([], [archive.tool()]),
            )
            .respond(Response::function_call(
                "call-archive",
                &archive,
                json!({}),
                8,
                4,
            )),
    )
    .verify(|run| {
        // user + assistant(function_call) + fat function_result, then the
        // turn fails before any further assistant message.
        run.expect_message_counts(1, 1, 1)?;
        run.expect_target_calls(1)?;
        let error_entry = run
            .transcript
            .iter()
            .find_map(|item| {
                let custom_type = item
                    .pointer("/custom/custom_type")
                    .and_then(Value::as_str);
                (custom_type == Some("error"))
                    .then(|| item.pointer("/custom/data"))
                    .flatten()
            })
            .ok_or_else(|| anyhow::anyhow!("no custom error entry in the transcript"))?;
        anyhow::ensure!(
            error_entry.get("code").and_then(Value::as_str) == Some("harness.budget_exceeded"),
            "error code {:?} != harness.budget_exceeded",
            error_entry.get("code")
        );
        anyhow::ensure!(
            error_entry.get("phase").and_then(Value::as_str) == Some("budget_preflight"),
            "error phase {:?} != budget_preflight",
            error_entry.get("phase")
        );
        anyhow::ensure!(
            error_entry.get("retryable") == Some(&Value::Bool(false)),
            "budget failures must not be retryable: {error_entry}"
        );
        run.expect_probe_response(0, "state::get", |response| {
            let ledger = response.get("value").unwrap_or(response);
            anyhow::ensure!(
                ledger.get("max_total_tokens").and_then(Value::as_u64) == Some(BUDGET_TOKENS),
                "ledger max_total_tokens != {BUDGET_TOKENS}: {ledger}"
            );
            anyhow::ensure!(
                ledger.get("used_tokens").and_then(Value::as_u64) == Some(EXPECTED_USED_TOKENS),
                "ledger used_tokens != {EXPECTED_USED_TOKENS}: {ledger}"
            );
            anyhow::ensure!(
                ledger.get("reserved_tokens").and_then(Value::as_u64) == Some(0),
                "the rejected reservation must be rolled back: {ledger}"
            );
            Ok(())
        })?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_declares_a_failed_turn_after_one_generation() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses, ["failed"]);
        assert_eq!(fixture.script.generations.len(), 1);
        assert_eq!(fixture.expected_traces(), 1);
        assert_eq!(fixture.probe_actions.len(), 1);
    }
}
