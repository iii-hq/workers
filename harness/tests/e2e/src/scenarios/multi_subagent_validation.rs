//! `multi_subagent_validation` — parallel fan-out of VALIDATED sub-agents:
//! the parent registers one `harness::hook::post-turn` validator per child
//! (disjoint session scopes, disjoint SQL predicates over a shared table,
//! per-child `retry_prompt`), arms one verdict wake per child, spawns both
//! children in the same turn, and tallies verdict wakes until every child
//! has passed its own gate.
//!
//! Exercises what the single-child scenario cannot: multiple validators
//! coexisting on the same hook point without cross-talk, concurrent child
//! correction loops, and multi-wake fan-in (wakes may merge into one parent
//! turn — the tally must count distinct writers, not wake deliveries).

use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::validation_loop::suffix;
use super::{CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};

pub const ID: &str = "multi_subagent_validation";

const HOOK_TYPE: &str = "harness::hook::post-turn";
const THRESHOLD: u64 = 6;
const EXPECTED_ROWS: u64 = 8;
const WRITERS: [&str; 2] = ["w1", "w2"];
const CHILDREN_GOAL: AssessmentSpec = AssessmentSpec::required(
    "children_goal",
    35,
    "Both writers exceed the threshold and both verdict keys carry their accepted counts.",
);
const ORCHESTRATION_DISCIPLINE: AssessmentSpec = AssessmentSpec::required(
    "orchestration_discipline",
    35,
    "Two child-scoped validators and two wakes registered before any spawn; both children looped under their own gate.",
);
const FAN_IN_REPORT: AssessmentSpec = AssessmentSpec::required(
    "fan_in_report",
    30,
    "The parent tallies both verdicts and finishes with the exact report line.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[CHILDREN_GOAL, ORCHESTRATION_DISCIPLINE, FAN_IN_REPORT];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let table = table(run_id);
    let scope = scope(run_id);
    let child_1 = child_session(run_id, 1);
    let child_2 = child_session(run_id, 2);
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: format!(
            "You orchestrate TWO validated sub-agents in parallel. You never poll and never judge \
             work yourself: per-child validators gate every child reply, and verdict wakes drive \
             you. Follow the steps exactly.\n\n\
             Step 1 — prepare the table. database::execute (db \"primary\") twice: sql \"CREATE \
             TABLE IF NOT EXISTS {table} (id INTEGER PRIMARY KEY AUTOINCREMENT, writer TEXT, note \
             TEXT)\", then sql \"DELETE FROM {table}\".\n\n\
             Step 2 — register ONE validator PER CHILD (two engine::register_trigger calls). For \
             each pair (N, wN) in [(1, w1), (2, w2)] call engine::register_trigger with:\n\
             - trigger_type: \"{HOOK_TYPE}\"\n\
             - function_id: \"fp::pipe\"\n\
             - config: {{\"sessions\": [\"<CHILD-N>\"], \"payload\": {{\"through\": \
             [{{\"function\": \"database::query\", \"payload\": {{\"db\": \"primary\", \"sql\": \
             \"SELECT COUNT(*) AS n FROM {table} WHERE writer = 'wN'\"}}}}, {{\"function\": \
             \"fp::get\", \"payload\": {{\"path\": \"/rows/0/n\"}}}}, {{\"function\": \"fp::when\", \
             \"payload\": {{\"op\": \">\", \"to\": {THRESHOLD}}}}}, {{\"function\": \"state::set\", \
             \"payload\": {{\"scope\": \"{scope}\", \"key\": \"verdict-wN\"}}}}]}}, \"result_into\": \
             \"/value\", \"retry_prompt\": \"VALIDATOR[wN]: only {{value}} of the required {min} \
             rows exist for writer wN. Insert exactly 4 more rows with writer wN into {table} and \
             reply with a one-line status.\", \"timeout_ms\": 30000}}\n\
             where <CHILD-1> is \"{child_1}\" and <CHILD-2> is \"{child_2}\" (replace every wN \
             with the matching writer; keep the single quotes around wN in the SQL). The \
             state::set tail runs ONLY when the guard passes, so verdict-wN is written exactly \
             when child N's work is accepted. Remember both subscription_ids.\n\n\
             Step 3 — arm TWO once-wakes BEFORE spawning: for each wN, engine::register_trigger \
             with trigger_type \"state\", config {{\"scope\": \"{scope}\", \"key\": \
             \"verdict-wN\"}}, label \"wN-validated\", and NO function_id.\n\n\
             Step 4 — spawn both children in THIS turn. For each pair, harness::spawn with \
             session_id <CHILD-N>, task: \"You are worker wN in a validated loop: the harness \
             checks every reply of yours and VALIDATOR messages are legitimate machinery — follow \
             them exactly. Insert exactly 4 rows into {table} with your writer tag: \
             database::execute, db 'primary', sql \\\"INSERT INTO {table} (writer, note) VALUES \
             ('wN','r'),('wN','r'),('wN','r'),('wN','r')\\\". Then reply with a one-line status. \
             Never check counts yourself.\" (replace wN), options: {{\"functions\": {{\"allow\": \
             [\"database::execute\"]}}, \"max_validation_retries\": 5}}. Spawn returns child ids \
             immediately — normal; do NOT wait for or judge children.\n\n\
             Step 5 — END YOUR TURN.\n\n\
             Step 6 — each verdict wake carries the accepted row count for one writer. Count \
             DISTINCT writers validated so far across this whole conversation (one wake may \
             report while another is still pending). If fewer than 2, END YOUR TURN and wait. \
             When BOTH writers are validated: call engine::unregister_trigger for both validator \
             subscription_ids from Step 2, then reply exactly: ALL CHILDREN VALIDATED: w1=<n1> \
             w2=<n2>. ORCHESTRATION DONE.",
            min = THRESHOLD + 1,
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 20,
            max_output_tokens: Some(8_192),
            max_total_tokens: 600_000,
            stuck_timeout_seconds: 420,
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
        let mut rows = [0_u64; 2];
        let mut verdicts = [0_u64; 2];
        let mut child_nudges = [0_usize; 2];
        for (index, writer) in WRITERS.iter().enumerate() {
            rows[index] = context
                .trigger_value(
                    "database::query",
                    json!({ "db": "primary", "sql": format!(
                        "SELECT COUNT(*) AS n FROM {table} WHERE writer = '{writer}'"
                    ) }),
                )
                .await?
                .pointer("/rows/0/n")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            verdicts[index] = common::state_value(
                context
                    .trigger(
                        "state::get",
                        json!({ "scope": scope(run_id), "key": format!("verdict-{writer}") }),
                    )
                    .await?,
            )
            .as_u64()
            .unwrap_or(0);
            let child = child_session(run_id, index + 1);
            child_nudges[index] = match context.transcript(&child).await {
                Ok(transcript) => common::validation_nudges(&transcript),
                Err(_) => 0,
            };
        }

        let calls = common::function_calls(&observation.transcript);
        let validator_count = calls
            .iter()
            .filter(|call| {
                call.function_id == "engine::register_trigger"
                    && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
            })
            .count();
        let first_spawn = calls
            .iter()
            .position(|call| call.function_id == "harness::spawn");
        let registrations_before_spawn = match first_spawn {
            Some(spawn) => calls[..spawn]
                .iter()
                .filter(|call| call.function_id == "engine::register_trigger")
                .count(),
            None => 0,
        };
        // Two validators + two wakes must all precede the first spawn.
        let ordered = first_spawn.is_some() && registrations_before_spawn >= 4;

        let goal = rows.iter().all(|count| *count > THRESHOLD)
            && verdicts.iter().all(|count| *count > THRESHOLD);
        let both_looped = child_nudges.iter().all(|nudges| *nudges >= 1);
        let reported = observation.response.contains("ALL CHILDREN VALIDATED")
            && observation.response.contains("ORCHESTRATION DONE");

        let orchestration_discipline = validator_count == 2 && ordered && both_looped;

        Ok(assessment::objective([
            CHILDREN_GOAL.required_points(
                goal,
                children_goal_points(goal, &rows),
                format!(
                    "rows={rows:?}, verdicts={verdicts:?}, all must exceed {THRESHOLD}; full \
                     marks at exactly {EXPECTED_ROWS} rows each"
                ),
            )?,
            ORCHESTRATION_DISCIPLINE.binary(
                orchestration_discipline,
                format!(
                    "observed {validator_count} post-turn registration(s), expected two; \
                     observed {registrations_before_spawn} registration(s) before the first \
                     spawn, expected at least four; child nudges {child_nudges:?}, each child \
                     needs at least one"
                ),
            ),
            FAN_IN_REPORT.binary(reported, "expected the exact report line"),
        ]))
    })
}

fn children_goal_points(goal: bool, rows: &[u64; 2]) -> u8 {
    if goal && rows.iter().all(|count| *count == EXPECTED_ROWS) {
        CHILDREN_GOAL.weight()
    } else if goal {
        20
    } else {
        0
    }
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
        for writer in WRITERS {
            let _: Value = context
                .trigger(
                    "state::delete",
                    json!({ "scope": scope(run_id), "key": format!("verdict-{writer}") }),
                )
                .await?;
        }
        Ok(())
    })
}

fn table(run_id: &str) -> String {
    format!("msubvtest_{}", suffix(run_id))
}

fn scope(run_id: &str) -> String {
    format!("msubv-{}", suffix(run_id))
}

fn child_session(run_id: &str, index: usize) -> String {
    format!("e2e_{run_id}-child-{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_names_both_children_under_the_suite_prefix() {
        let spec = scenario("aB19-rest");
        assert!(spec.prompt.contains("e2e_aB19-rest-child-1"));
        assert!(spec.prompt.contains("e2e_aB19-rest-child-2"));
        assert!(spec.prompt.contains("msubvtest_aB19"));
        spec.validate().unwrap();
        assert_eq!(spec.version, 2);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("children_goal", 35),
                ("orchestration_discipline", 35),
                ("fan_in_report", 30),
            ]
        );
    }

    #[test]
    fn exact_rows_only_score_fully_after_both_verdicts_pass() {
        assert_eq!(children_goal_points(false, &[EXPECTED_ROWS; 2]), 0);
        assert_eq!(children_goal_points(true, &[EXPECTED_ROWS; 2]), 35);
        assert_eq!(
            children_goal_points(true, &[EXPECTED_ROWS, EXPECTED_ROWS + 1]),
            20
        );
    }
}
