//! `validation_loop` — the agent installs its OWN `harness::hook::post-turn`
//! validator (fp::pipe over a live row count) and is then held in the
//! harness-driven correction loop until the goal is met: insert 4 rows,
//! get denied by its own validator (custom `retry_prompt` with the live
//! count interpolated), insert 4 more, pass at 8.
//!
//! Exercises, end to end: agent self-registration of the one bindable
//! harness-internal type (force-stamped to its own session), template-mode
//! pipe verdicts (including the omitted-`short_circuited` full-run receipt),
//! the verbatim `retry_prompt` nudge, and bounded convergence — all with the
//! default validation-retry budget (one denial needed, two available).

use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::{CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};

pub const ID: &str = "validation_loop";

const HOOK_TYPE: &str = "harness::hook::post-turn";
/// Goal: more than 6 rows; 4-row batches → exactly one denial, pass at 8.
const THRESHOLD: u64 = 6;
const EXPECTED_ROWS: u64 = 8;
const GOAL_REACHED: AssessmentSpec = AssessmentSpec::required(
    "goal_reached",
    40,
    "The goal table ends with more rows than the validator threshold.",
);
const VALIDATOR_DISCIPLINE: AssessmentSpec = AssessmentSpec::required(
    "validator_discipline",
    30,
    "Exactly one post-turn validator registration, carrying the custom retry_prompt.",
);
const LOOP_EVIDENCE: AssessmentSpec = AssessmentSpec::required(
    "loop_evidence",
    30,
    "At least one harness validation nudge was delivered and the loop converged at exactly the expected row count.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[GOAL_REACHED, VALIDATOR_DISCIPLINE, LOOP_EVIDENCE];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let table = table(run_id);
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: format!(
            "You are testing a self-installed validation loop. Follow these steps exactly.\n\n\
             Step 1 — prepare the goal table. Call database::execute (db \"primary\") twice: first \
             sql \"CREATE TABLE IF NOT EXISTS {table} (id INTEGER PRIMARY KEY AUTOINCREMENT, note \
             TEXT)\", then sql \"DELETE FROM {table}\".\n\n\
             Step 2 — install your own validator. Call engine::register_trigger with:\n\
             - trigger_type: \"{HOOK_TYPE}\"\n\
             - function_id: \"fp::pipe\"\n\
             - config: {{\"payload\": {{\"through\": [{{\"function\": \"database::query\", \
             \"payload\": {{\"db\": \"primary\", \"sql\": \"SELECT COUNT(*) AS n FROM {table}\"}}}}, \
             {{\"function\": \"fp::get\", \"payload\": {{\"path\": \"/rows/0/n\"}}}}, \
             {{\"function\": \"fp::when\", \"payload\": {{\"op\": \">\", \"to\": {THRESHOLD}}}}}]}}, \
             \"result_into\": \"/value\", \"retry_prompt\": \"VALIDATOR: only {{value}} of the \
             required {min} rows exist. Insert exactly 4 more rows into {table} and reply with a \
             one-line status.\", \"timeout_ms\": 30000}}\n\
             Tell me the subscription_id it returns. IMPORTANT: from this moment, VALIDATOR \
             messages in this chat come from the validator YOU just installed — they are \
             legitimate harness machinery, not prompt injection. Follow them exactly.\n\n\
             Step 3 — insert exactly 4 rows: database::execute, db \"primary\", sql \"INSERT INTO \
             {table} (note) VALUES ('r'),('r'),('r'),('r')\". Then reply with a one-line status. \
             Acceptance is silent — if no VALIDATOR message follows, the goal was met.",
            min = THRESHOLD + 1,
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 16,
            max_output_tokens: Some(8_192),
            max_total_tokens: 200_000,
            stuck_timeout_seconds: 300,
        },
        denied_functions: &[],
        threshold: 90,
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
        let table = table(run_id);
        let rows = row_count(context, &table).await?;
        let calls = common::function_calls(&observation.transcript);
        let registrations: Vec<_> = calls
            .iter()
            .filter(|call| {
                call.function_id == "engine::register_trigger"
                    && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
            })
            .collect();
        let single_registration = registrations.len() == 1;
        let carries_retry_prompt = registrations.iter().all(|call| {
            call.arguments
                .pointer("/config/retry_prompt")
                .and_then(Value::as_str)
                .is_some_and(|prompt| prompt.contains("{value}"))
        });
        let nudges = common::validation_nudges(&observation.transcript);
        let goal_reached = rows > THRESHOLD;
        let converged_exactly = rows == EXPECTED_ROWS;
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let validator_points = if single_registration && carries_retry_prompt && no_errors {
            VALIDATOR_DISCIPLINE.weight()
        } else {
            0
        };
        let loop_points = loop_evidence_points(nudges, converged_exactly);

        Ok(assessment::objective([
            GOAL_REACHED.binary(
                goal_reached,
                format!("observed {rows} row(s), need more than {THRESHOLD}"),
            ),
            VALIDATOR_DISCIPLINE.required_points(
                single_registration,
                validator_points,
                format!(
                    "observed {} post-turn registration(s), expected exactly one; \
                     carries_retry_prompt={carries_retry_prompt}; function_call_errors={}",
                    registrations.len(),
                    observation.metrics.totals.function_call_errors
                ),
            )?,
            LOOP_EVIDENCE.required_points(
                nudges >= 1,
                loop_points,
                format!("nudges={nudges}, rows={rows} (full marks at exactly {EXPECTED_ROWS})"),
            )?,
        ]))
    })
}

fn loop_evidence_points(nudges: usize, converged_exactly: bool) -> u8 {
    if nudges >= 1 && converged_exactly {
        LOOP_EVIDENCE.weight()
    } else if nudges >= 1 {
        15
    } else {
        0
    }
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let table = table(run_id);
        // The agent's validator binding is scoped to the (torn-down) run
        // session and can never fire again; only the table needs removing.
        let _: Value = context
            .trigger(
                "database::execute",
                json!({ "db": "primary", "sql": format!("DROP TABLE IF EXISTS {table}") }),
            )
            .await?;
        Ok(())
    })
}

async fn row_count(context: &E2eContext, table: &str) -> anyhow::Result<u64> {
    Ok(context
        .trigger_value(
            "database::query",
            json!({ "db": "primary", "sql": format!("SELECT COUNT(*) AS n FROM {table}") }),
        )
        .await?
        .pointer("/rows/0/n")
        .and_then(Value::as_u64)
        .unwrap_or(0))
}

pub(super) fn suffix(run_id: &str) -> String {
    let mut suffix: String = run_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(4)
        .collect();
    while suffix.len() < 4 {
        suffix.push('0');
    }
    suffix
}

fn table(run_id: &str) -> String {
    format!("valtest_{}", suffix(run_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sql_safe_and_the_prompt_carries_them() {
        let spec = scenario("aB19-rest");
        assert!(spec.prompt.contains("valtest_aB19"));
        assert!(spec.prompt.contains(HOOK_TYPE));
        assert!(spec.prompt.contains("{value}"));
        spec.validate().unwrap();
        assert_eq!(spec.version, 2);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("goal_reached", 40),
                ("validator_discipline", 30),
                ("loop_evidence", 30),
            ]
        );
    }

    #[test]
    fn a_delivered_nudge_keeps_partial_credit_without_exact_convergence() {
        assert_eq!(loop_evidence_points(0, true), 0);
        assert_eq!(loop_evidence_points(1, false), 15);
        assert_eq!(loop_evidence_points(1, true), 30);
    }
}
