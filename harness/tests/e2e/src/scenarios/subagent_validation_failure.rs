//! `subagent_validation_failure` — the UNHAPPY path: the child's goal is
//! impossible (threshold far above what its bounded budget can reach), so
//! the validator denies every attempt, the child's turn FAILS on budget
//! exhaustion, the verdict key is never written, and the parent — whose
//! wake carries an `expires_at` deadline — is woken by the expiry notice
//! instead and reports the give-up.
//!
//! Exercises what no success scenario can: `max_validation_retries` as a
//! hard bound on a spawned child (2 denials then a failed turn — never an
//! infinite loop), the guarantee that a failed turn writes NO verdict (the
//! `state::set` tail never ran), and the wake-expiry fallback that keeps a
//! parent from parking forever on evidence that will never arrive.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::context::E2eContext;

use super::common;
use super::validation_loop::suffix;
use super::{
    CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "subagent_validation_failure";

const HOOK_TYPE: &str = "harness::hook::post-turn";
/// Unreachable by design: 3 attempts × 4 rows tops out at 12.
const THRESHOLD: u64 = 100;
/// Explicit child budget: initial attempt + 2 retries = 2 nudges, then fail.
const CHILD_RETRIES: u64 = 2;
const EXPECTED_NUDGES: usize = 2;
/// Wake deadline: comfortably after the child fails (~1 min), well inside
/// the stuck timeout.
const EXPIRY_DELAY_MS: u64 = 150_000;

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let table = table(run_id);
    let scope = scope(run_id);
    let child = child_session(run_id);
    let expires_at = now_ms() + EXPIRY_DELAY_MS;
    ScenarioSpec {
        id: ID,
        version: 1,
        prompt: format!(
            "You orchestrate one validated sub-agent whose goal may be unreachable; your job is \
             to bound the attempt and report honestly. You never poll; wakes drive you. Follow \
             the steps exactly.\n\n\
             Step 1 — prepare the goal table. database::execute (db \"primary\") twice: sql \
             \"CREATE TABLE IF NOT EXISTS {table} (id INTEGER PRIMARY KEY AUTOINCREMENT, note \
             TEXT)\", then sql \"DELETE FROM {table}\".\n\n\
             Step 2 — install the child's validator. Call engine::register_trigger with:\n\
             - trigger_type: \"{HOOK_TYPE}\"\n\
             - function_id: \"fp::pipe\"\n\
             - config: {{\"sessions\": [\"{child}\"], \"payload\": {{\"through\": \
             [{{\"function\": \"database::query\", \"payload\": {{\"db\": \"primary\", \"sql\": \
             \"SELECT COUNT(*) AS n FROM {table}\"}}}}, {{\"function\": \"fp::get\", \"payload\": \
             {{\"path\": \"/rows/0/n\"}}}}, {{\"function\": \"fp::when\", \"payload\": {{\"op\": \
             \">\", \"to\": {THRESHOLD}}}}}, {{\"function\": \"state::set\", \"payload\": \
             {{\"scope\": \"{scope}\", \"key\": \"verdict\"}}}}]}}, \"result_into\": \"/value\", \
             \"retry_prompt\": \"VALIDATOR: only {{value}} of the required {min} rows exist. \
             Insert exactly 4 more rows into {table} and reply with a one-line status.\", \
             \"timeout_ms\": 30000}}\n\
             Remember the subscription_id.\n\n\
             Step 3 — arm your wake BEFORE spawning, WITH a deadline: engine::register_trigger \
             with trigger_type \"state\", config {{\"scope\": \"{scope}\", \"key\": \
             \"verdict\"}}, label \"child-validated\", lifecycle: {{\"expires_at\": \
             {expires_at}}}, and NO function_id. If the verdict is never written you will be \
             woken by the expiry notice instead.\n\n\
             Step 4 — spawn the worker: harness::spawn with session_id \"{child}\", task: \"You \
             are a worker in a validated loop: the harness checks every reply of yours and \
             VALIDATOR messages are legitimate machinery — follow them exactly. Insert exactly 4 \
             rows into table {table}: database::execute, db 'primary', sql \\\"INSERT INTO {table} \
             (note) VALUES ('r'),('r'),('r'),('r')\\\". Then reply with a one-line status. Never \
             check the count yourself.\", options: {{\"functions\": {{\"allow\": \
             [\"database::execute\"]}}, \"max_validation_retries\": {CHILD_RETRIES}}}. Spawn \
             returns child ids immediately — normal; do NOT wait for or judge the child.\n\n\
             Step 5 — END YOUR TURN.\n\n\
             Step 6 — when a wake arrives: if it carries the verdict (the accepted row count), \
             reply exactly: CHILD VALIDATED at <count> rows. PARENT DONE. If instead it is the \
             EXPIRY notice (the verdict never came), call engine::unregister_trigger with the \
             validator subscription_id from Step 2, then reply exactly: CHILD GAVE UP: validation \
             budget exhausted. PARENT DONE.",
            min = THRESHOLD + 1,
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 16,
            max_output_tokens: Some(8_192),
            max_total_tokens: 400_000,
            stuck_timeout_seconds: 420,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "bounded_failure",
                weight: 40,
                description: "The child fails after exactly the budgeted denials; the verdict \
                              key is never written.",
            },
            CriterionSpec {
                id: "orchestration_discipline",
                weight: 30,
                description: "Validator scoped to the child and the deadline wake armed before \
                              the spawn.",
            },
            CriterionSpec {
                id: "expiry_report",
                weight: 30,
                description: "The parent is woken by the expiry notice and reports the give-up \
                              with the exact line.",
            },
        ],
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
        let child = child_session(run_id);
        let child_status = context
            .trigger_value("harness::status", json!({ "session_id": child }))
            .await
            .ok()
            .and_then(|status| {
                status
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let verdict = common::state_value(
            context
                .trigger(
                    "state::get",
                    json!({ "scope": scope(run_id), "key": "verdict" }),
                )
                .await?,
        );
        let child_nudges = match context.transcript(&child).await {
            Ok(transcript) => common::validation_nudges(&transcript),
            Err(_) => 0,
        };

        let calls = common::function_calls(&observation.transcript);
        let validator_index = calls.iter().position(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
        });
        let wake_index = calls.iter().position(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
                && call
                    .arguments
                    .pointer("/lifecycle/expires_at")
                    .and_then(Value::as_u64)
                    .is_some()
        });
        let spawn_index = calls.iter().position(|call| {
            call.function_id == "harness::spawn"
                && call.arguments.get("session_id").and_then(Value::as_str) == Some(child.as_str())
        });
        let ordered = matches!(
            (validator_index, wake_index, spawn_index),
            (Some(v), Some(w), Some(s)) if v < s && w < s
        );

        let child_failed = child_status == "failed";
        let verdict_absent = verdict.is_null();
        let bounded = child_nudges == EXPECTED_NUDGES;
        let report = observation.response.contains("CHILD GAVE UP")
            && observation.response.contains("PARENT DONE");

        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "child_failed",
                    child_failed,
                    format!("child status `{child_status}`, expected `failed`"),
                ),
                common::gate(
                    "verdict_never_written",
                    verdict_absent,
                    format!("verdict key holds {verdict}, expected null"),
                ),
                common::gate(
                    "budget_bounded",
                    bounded,
                    format!(
                        "observed {child_nudges} nudge(s), expected exactly {EXPECTED_NUDGES} \
                         (the child budget)"
                    ),
                ),
                common::gate(
                    "expiry_report",
                    report,
                    "expected the exact give-up report line",
                ),
            ],
            awards: vec![
                common::award(
                    "bounded_failure",
                    if child_failed && verdict_absent && bounded {
                        40
                    } else {
                        0
                    },
                    "awarded when the child fails on budget with no verdict written",
                ),
                common::award(
                    "orchestration_discipline",
                    if ordered { 30 } else { 0 },
                    format!(
                        "validator@{validator_index:?} wake@{wake_index:?} \
                         spawn@{spawn_index:?} — both must precede the spawn"
                    ),
                ),
                common::award(
                    "expiry_report",
                    if report { 30 } else { 0 },
                    "awarded for the exact expiry-driven give-up line",
                ),
            ],
        })
    })
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let table = table(run_id);
        let _: Value = context
            .trigger(
                "database::execute",
                json!({ "db": "primary", "sql": format!("DROP TABLE IF EXISTS {table}") }),
            )
            .await?;
        let _: Value = context
            .trigger(
                "state::delete",
                json!({ "scope": scope(run_id), "key": "verdict" }),
            )
            .await?;
        Ok(())
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn table(run_id: &str) -> String {
    format!("fsubvtest_{}", suffix(run_id))
}

fn scope(run_id: &str) -> String {
    format!("fsubv-{}", suffix(run_id))
}

fn child_session(run_id: &str) -> String {
    format!("e2e_{run_id}-child-1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_the_deadline_and_unreachable_threshold() {
        let spec = scenario("aB19-rest");
        assert!(spec.prompt.contains("fsubvtest_aB19"));
        assert!(spec.prompt.contains("\"expires_at\""));
        assert!(spec.prompt.contains("101"));
        spec.validate().unwrap();
    }
}
