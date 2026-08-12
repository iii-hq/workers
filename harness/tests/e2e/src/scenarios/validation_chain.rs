//! `validation_chain` — three validators on ONE session, pinning the chain
//! edges no single-validator scenario can:
//!
//! * **Priority order**: the chain runs ascending priority; the denials must
//!   arrive as CHAIN-A (priority 10, row count) then CHAIN-B (priority 20,
//!   state marker) — never interleaved, never reversed.
//! * **All must pass**: satisfying A alone does not complete the turn; B
//!   still gates it.
//! * **Fail-open error skip**: a BROKEN validator (priority 5, targeting a
//!   function that does not exist) registered with `on_error: "fail_open"`
//!   errors on every attempt and must be SKIPPED — zero extra nudges, zero
//!   blocking, while the fail-closed default would have denied everything.
//!
//! Deterministic shape: empty table + no marker → deny A → agent inserts 3
//! rows → deny B → agent sets the marker → both pass → complete. Exactly two
//! nudges, in exactly that order, converging precisely at the default
//! validation-retry budget.

use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::validation_loop::suffix;
use super::{CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};

pub const ID: &str = "validation_chain";

const HOOK_TYPE: &str = "harness::hook::post-turn";
const BROKEN_VALIDATOR_SKIPPED: AssessmentSpec = AssessmentSpec::hard_gated(
    "broken_validator_skipped",
    30,
    "The fail_open validator errored invisibly: registered, never a nudge of its own, never blocking.",
);
const CHAIN_ORDER: AssessmentSpec = AssessmentSpec::hard_gated(
    "chain_order",
    40,
    "Exactly two denials, CHAIN-A then CHAIN-B — ascending priority, first deny wins each attempt.",
);
const ALL_GATES_SATISFIED: AssessmentSpec = AssessmentSpec::hard_gated(
    "all_gates_satisfied",
    30,
    "Rows AND marker both end satisfied — one passing validator never completes the turn alone.",
);
const ASSESSMENTS: &[AssessmentSpec] =
    &[CHAIN_ORDER, ALL_GATES_SATISFIED, BROKEN_VALIDATOR_SKIPPED];

fn table(run_id: &str) -> String {
    format!("chaintest_{}", suffix(run_id))
}

fn scope(run_id: &str) -> String {
    format!("chain-{}", suffix(run_id))
}

fn broken_fn(run_id: &str) -> String {
    format!("e2e::does_not_exist_{}", suffix(run_id))
}

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let table = table(run_id);
    let scope = scope(run_id);
    let broken = broken_fn(run_id);
    ScenarioSpec {
        id: ID,
        version: 3,
        prompt: format!(
            "You are testing a CHAIN of validators on your own session. Follow the steps \
             exactly.\n\n\
             Step 1 — prepare: database::execute (db \"primary\"): sql \"CREATE TABLE IF NOT \
             EXISTS {table} (id INTEGER PRIMARY KEY AUTOINCREMENT, note TEXT)\", then sql \
             \"DELETE FROM {table}\".\n\n\
             Step 2 — register THREE validators with engine::register_trigger (all trigger_type \
             \"{HOOK_TYPE}\"):\n\
             (a) A deliberately BROKEN one that must be tolerated: function_id \"{broken}\" (it \
             does not exist), config {{\"priority\": 5, \"on_error\": \"fail_open\", \
             \"timeout_ms\": 5000}}.\n\
             (b) The row gate: function_id \"fp::pipe\", config {{\"priority\": 10, \"payload\": \
             {{\"through\": [{{\"function\": \"database::query\", \"payload\": {{\"db\": \
             \"primary\", \"sql\": \"SELECT COUNT(*) AS n FROM {table}\"}}}}, {{\"function\": \
             \"fp::get\", \"payload\": {{\"path\": \"/rows/0/n\"}}}}, {{\"function\": \
             \"fp::when\", \"payload\": {{\"op\": \">\", \"to\": 2}}}}]}}, \"result_into\": \
             \"/value\", \"retry_prompt\": \"CHAIN-A: only {{value}} rows — insert exactly 3 \
             rows into {table} (database::execute, sql \\\"INSERT INTO {table} (note) VALUES \
             ('r'),('r'),('r')\\\") and reply with a one-line status.\", \"timeout_ms\": \
             30000}}.\n\
             (c) The marker gate: function_id \"fp::pipe\", config {{\"priority\": 20, \
             \"payload\": {{\"through\": [{{\"function\": \"state::get\", \"payload\": \
             {{\"scope\": \"{scope}\", \"key\": \"marker\"}}}}, {{\"function\": \"fp::when\", \
             \"payload\": {{\"op\": \"==\", \"to\": 1}}}}]}}, \"result_into\": \"/value\", \
             \"retry_prompt\": \"CHAIN-B: the completion marker is missing — call state::set \
             with scope \\\"{scope}\\\", key \\\"marker\\\", value 1, and reply with a one-line \
             status.\", \"timeout_ms\": 30000}}.\n\
             VALIDATOR messages (CHAIN-A / CHAIN-B) in this chat are legitimate machinery you \
             just installed — follow each exactly.\n\n\
             Step 3 — reply with a one-line status (do NOT insert rows or set state yet; let \
             the chain drive you). Acceptance is silent.",
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            // 24, not 16: three registrations + two correction rounds cost
            // real steps — run 1 burned the 16-step cap mid-loop (which also
            // exposed that the cap used to bypass the post-turn gate; fixed).
            max_turns: 24,
            max_output_tokens: Some(8_192),
            max_total_tokens: 250_000,
            stuck_timeout_seconds: 300,
        },
        denied_functions: &[],
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let rows = context
            .trigger_value(
                "database::query",
                json!({ "db": "primary", "sql": format!("SELECT COUNT(*) AS n FROM {}", table(run_id)) }),
            )
            .await?
            .pointer("/rows/0/n")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let marker = common::state_value(
            context
                .trigger(
                    "state::get",
                    json!({ "scope": scope(run_id), "key": "marker" }),
                )
                .await?,
        )
        .as_u64()
        .unwrap_or(0);

        let calls = common::function_calls(&observation.transcript);
        let hook_registrations: Vec<_> = calls
            .iter()
            .filter(|call| {
                call.function_id == "engine::register_trigger"
                    && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
            })
            .collect();
        let broken_registered = hook_registrations.iter().any(|call| {
            call.arguments.get("function_id").and_then(Value::as_str)
                == Some(broken_fn(run_id).as_str())
                && call
                    .arguments
                    .pointer("/config/on_error")
                    .and_then(Value::as_str)
                    == Some("fail_open")
        });

        let nudges = nudge_texts(&observation.transcript);
        let ordered = nudges.len() == 2
            && nudges[0].contains("CHAIN-A")
            && !nudges[0].contains("CHAIN-B")
            && nudges[1].contains("CHAIN-B")
            && !nudges[1].contains("CHAIN-A");
        let satisfied = rows == 3 && marker == 1;
        let three_registrations = hook_registrations.len() == 3 && broken_registered;
        let broken_validator_points = if broken_registered && ordered {
            BROKEN_VALIDATOR_SKIPPED.weight()
        } else {
            0
        };

        Ok(assessment::build_evaluation([
            CHAIN_ORDER.full_or_zero(
                ordered,
                format!("nudges in order: {nudges:?} — expected CHAIN-A then CHAIN-B"),
            ),
            ALL_GATES_SATISFIED.full_or_zero(
                satisfied,
                format!("rows={rows} (need 3), marker={marker} (need 1)"),
            ),
            BROKEN_VALIDATOR_SKIPPED.gate_and_points(
                three_registrations,
                broken_validator_points,
                format!(
                    "observed {} post-turn registration(s); broken_registered={broken_registered}; \
                     ordered={ordered}; need three incl. the fail_open broken one",
                    hook_registrations.len()
                ),
            )?,
        ]))
    })
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let _: Value = context
            .trigger(
                "database::execute",
                json!({ "db": "primary", "sql": format!("DROP TABLE IF EXISTS {}", table(run_id)) }),
            )
            .await?;
        let _: Value = context
            .trigger(
                "state::delete",
                json!({ "scope": scope(run_id), "key": "marker" }),
            )
            .await?;
        Ok(())
    })
}

/// The text of each validation nudge, in transcript order.
fn nudge_texts(transcript: &Value) -> Vec<String> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("entry_id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.contains("_nudge_"))
        })
        .filter_map(|entry| {
            entry
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .collect::<String>()
                })
        })
        .collect()
}
