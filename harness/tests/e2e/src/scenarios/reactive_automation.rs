use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde_json::{json, Value};

use crate::context::E2eContext;

use super::common;
use super::{
    CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "reactive_automation";
const DATABASE: &str = "primary";
const EXPECTED_WRITERS: usize = 3;
const ORDERS_PER_WRITER: i64 = 5;
const EXPECTED_ORDERS: i64 = EXPECTED_WRITERS as i64 * ORDERS_PER_WRITER;

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = ScenarioNames::new(run_id);
    ScenarioSpec {
        id: ID,
        prompt: prompt(&names),
        execution: ExecutionPolicy {
            max_turns: 64,
            max_output_tokens: None,
            max_total_tokens: 600_000,
            timeout_seconds: 1_200,
        },
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "parallel_writes",
                weight: 25,
                description: "Three parallel writer sessions produce exactly five valid orders each.",
            },
            CriterionSpec {
                id: "reactive_aggregates",
                weight: 30,
                description: "Trigger-spawned reactors maintain totals that exactly match the source rows.",
            },
            CriterionSpec {
                id: "trigger_orchestration",
                weight: 25,
                description: "The watch is armed before writers start and the documented fallback is proven.",
            },
            CriterionSpec {
                id: "finalization_cleanup",
                weight: 20,
                description: "One trigger-spawned finalizer writes a passing report and removes run triggers.",
            },
        ],
        judge_reference: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn prompt(names: &ScenarioNames) -> String {
    format!(
        r#"Test this system's ability to orchestrate sub-agents around database-change triggers.
I am describing the outcome, not the implementation: work out the mechanics with the workers
and trigger types available on this stack.

This run already has the fresh run id `{run_label}`. Do not generate another id. Use
`{namespace}` as its SQL-safe namespace, database `primary`, and these exact table names:

- `{orders}`: orders with columns `id`, `writer`, `amount`, and `created_at`
- `{writers}`: one status row per writer
- `{totals}`: one row per writer with `writer`, `order_count`, and `amount_sum`
- `{report}`: exactly one final report row

Namespace every additional table, state scope, signal, subscription label, and session id with
`{run_label}` so this run cannot collide with another one. Use these exact writer session ids:
`{writer_1}`, `{writer_2}`, and `{writer_3}`. Use `{finalizer}` for the finalizer session.

The scenario:

1. WATCH — Before any workload order insert or writer spawn, inspect the available trigger types
   and arrange to be notified when rows change in `{orders}`. Prefer a real push-based database
   change trigger if one is registered. Give it a bounded probe: register it, perform one test
   write, and wait at most 30 seconds for an event. Remove the probe row before the writers
   start. If no event arrives, or the mechanism rejects the configured database driver, record
   exactly what you tried and what happened, then fall back to a notification path that
   demonstrably fires. A valid fallback is for each writer to emit a namespaced state change
   signal immediately after each insert. The watcher must be a registered trigger targeting
   `harness::react`, which spawns or wakes a reactor session; it must not be a polling loop in
   your own turn.

2. WRITE — Spawn exactly three writer sub-agents in parallel, using the session ids above.
   Each writer inserts exactly five orders, one insert at a time and roughly two seconds apart.
   Each order must have a unique id, the writer name (`writer-1`, `writer-2`, or `writer-3`), a
   numeric amount, and a non-null creation time. Writers know nothing about watcher internals:
   they only insert, emit the bridge signal immediately after each insert when the fallback is
   active, and mark themselves `done` in `{writers}`.

3. REACT — Every change notification must wake a reactor through the registered trigger.
   Reactors upsert running aggregates into `{totals}` by recomputing from source rows. They must
   be idempotent: processing the same change twice cannot double-count. Reactor session ids must
   also start with `{run_label}`.

4. FINISH — After all three writers are done and the totals cover all 15 orders, exactly one
   trigger-spawned finalizer in `{finalizer}` writes exactly one row to `{report}`. Use these
   snake_case report columns:

   `run_id`, `watch_mechanism`, `fallback_reason`, `events_received`, `rows_written`,
   `elapsed_ms`, `totals_match`, `no_notification_loss`, `no_double_counting`,
   `reactor_session_id`, `spawning_event`, `trigger_spawned_reactor`,
   `no_inline_waiting`, and `finalizer_session_id`.

   Store `{run_label}` in `run_id` and `{finalizer}` in `finalizer_session_id`. Record the actual
   watch mechanism and a non-empty fallback reason when fallback was needed. Store counts for
   events received and rows written, elapsed milliseconds from the first order insert to the
   last aggregate, one cited reactor session id and its spawning event, and boolean pass/fail
   values for every acceptance check below.

   Once the report exists, unregister every trigger and subscription created for this run.
   Nothing namespaced with `{run_label}` may keep firing. Wake this root session when the report
   and cleanup are complete so it can return the final summary.

Acceptance checks must be self-verified with queries before the report is written:

- totals match a direct `GROUP BY` over `{orders}` exactly
- aggregates cover every order and no order is double-counted
- the cited reactor ran in a session spawned by a trigger, not inline in this turn
- writers, reactors, and finalizer did not hold a turn open by sleeping or polling instead of
  using the trigger mechanism

Report progress as the run is wired and started. Keep the final summary short and factual."#,
        run_label = names.run_label,
        namespace = names.table_prefix,
        orders = names.orders,
        writers = names.writers,
        totals = names.totals,
        report = names.report,
        writer_1 = names.writer_sessions[0],
        writer_2 = names.writer_sessions[1],
        writer_3 = names.writer_sessions[2],
        finalizer = names.finalizer_session,
    )
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let names = ScenarioNames::new(run_id);
        let calls = common::function_calls(&observation.transcript);
        let expected_writer_sessions: BTreeSet<_> =
            names.writer_sessions.iter().map(String::as_str).collect();
        let spawn_call_count = calls
            .iter()
            .filter(|call| call.function_id == "harness::spawn")
            .count();
        let spawned_writer_sessions: BTreeSet<_> = calls
            .iter()
            .filter(|call| call.function_id == "harness::spawn")
            .filter_map(|call| call.arguments.get("session_id").and_then(Value::as_str))
            .collect();
        let writer_sessions_in_tree = expected_writer_sessions.iter().all(|expected| {
            observation
                .metrics
                .by_session
                .iter()
                .any(|session| session.session_id == *expected)
        });
        let parallel_writers = spawn_call_count == EXPECTED_WRITERS
            && spawned_writer_sessions == expected_writer_sessions
            && max_parallel_calls(&observation.transcript, "harness::spawn") == EXPECTED_WRITERS
            && writer_sessions_in_tree;

        let first_spawn = calls
            .iter()
            .position(|call| call.function_id == "harness::spawn");
        let trigger_catalog = calls
            .iter()
            .position(|call| call.function_id == "engine::triggers::list");
        let row_change_probe = calls.iter().position(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str)
                    == Some("database::row-change")
        });
        let state_reaction = calls.iter().position(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
                && call.arguments.get("function_id").and_then(Value::as_str)
                    == Some("harness::react")
        });
        let watch_before_writers = first_spawn.is_some_and(|spawn| {
            trigger_catalog.is_some_and(|catalog| catalog < spawn)
                && row_change_probe.is_some_and(|probe| probe < spawn)
                && state_reaction.is_some_and(|fallback| fallback < spawn)
        });

        let tables = existing_tables(context, &names).await?;
        let all_tables_exist = [
            names.orders.as_str(),
            names.writers.as_str(),
            names.totals.as_str(),
            names.report.as_str(),
        ]
        .iter()
        .all(|name| tables.contains(*name));

        let order_summary = if tables.contains(names.orders.as_str()) {
            first_row(
                context,
                &format!(
                    "SELECT COUNT(*) AS rows_written, COUNT(DISTINCT id) AS distinct_ids, \
                     COUNT(DISTINCT writer) AS writer_count, \
                     SUM(CASE WHEN created_at IS NULL THEN 1 ELSE 0 END) AS missing_created_at \
                     FROM {}",
                    names.orders
                ),
            )
            .await?
        } else {
            Value::Null
        };
        let orders_complete = integer_field(&order_summary, "rows_written")
            == Some(EXPECTED_ORDERS)
            && integer_field(&order_summary, "distinct_ids") == Some(EXPECTED_ORDERS)
            && integer_field(&order_summary, "writer_count") == Some(EXPECTED_WRITERS as i64)
            && integer_field(&order_summary, "missing_created_at") == Some(0);

        let expected_writer_names: BTreeSet<_> = (1..=EXPECTED_WRITERS)
            .map(|index| format!("writer-{index}"))
            .collect();
        let writers_done = if tables.contains(names.writers.as_str()) {
            let rows = query_rows(
                context,
                &format!(
                    "SELECT writer, status FROM {} ORDER BY writer",
                    names.writers
                ),
            )
            .await?;
            rows.len() == EXPECTED_WRITERS
                && rows.iter().all(|row| {
                    row.get("writer")
                        .and_then(Value::as_str)
                        .is_some_and(|writer| expected_writer_names.contains(writer))
                        && row
                            .get("status")
                            .and_then(Value::as_str)
                            .is_some_and(|status| status.eq_ignore_ascii_case("done"))
                })
        } else {
            false
        };

        let direct_totals = totals_from_table(
            context,
            tables.contains(names.orders.as_str()),
            &format!(
                "SELECT writer, COUNT(*) AS order_count, SUM(amount) AS amount_sum \
                 FROM {} GROUP BY writer ORDER BY writer",
                names.orders
            ),
        )
        .await?;
        let stored_totals = totals_from_table(
            context,
            tables.contains(names.totals.as_str()),
            &format!(
                "SELECT writer, order_count, amount_sum FROM {} ORDER BY writer",
                names.totals
            ),
        )
        .await?;
        let totals_match = direct_totals.as_ref().is_some_and(|totals| {
            totals.len() == EXPECTED_WRITERS
                && totals.keys().cloned().collect::<BTreeSet<_>>() == expected_writer_names
                && totals
                    .values()
                    .all(|(count, _amount)| *count == ORDERS_PER_WRITER)
        }) && direct_totals == stored_totals;

        let report_rows = if tables.contains(names.report.as_str()) {
            query_rows(context, &format!("SELECT * FROM {}", names.report)).await?
        } else {
            Vec::new()
        };
        let report = report_rows.first().filter(|_| report_rows.len() == 1);
        let report_counts_match = report.is_some_and(|report| {
            integer_field(report, "events_received") == Some(EXPECTED_ORDERS)
                && integer_field(report, "rows_written") == Some(EXPECTED_ORDERS)
                && integer_field(report, "elapsed_ms").is_some_and(|elapsed| elapsed > 0)
        });
        let report_checks_pass = report.is_some_and(|report| {
            [
                "totals_match",
                "no_notification_loss",
                "no_double_counting",
                "trigger_spawned_reactor",
                "no_inline_waiting",
            ]
            .iter()
            .all(|field| boolean_field(report, field) == Some(true))
        });
        let fallback_documented = report.is_some_and(|report| {
            report
                .get("watch_mechanism")
                .and_then(Value::as_str)
                .is_some_and(|mechanism| mechanism.to_ascii_lowercase().contains("state"))
                && report
                    .get("fallback_reason")
                    .and_then(Value::as_str)
                    .is_some_and(|reason| !reason.trim().is_empty())
        });
        let report_identity_matches = report.is_some_and(|report| {
            report.get("run_id").and_then(Value::as_str) == Some(names.run_label.as_str())
                && report.get("finalizer_session_id").and_then(Value::as_str)
                    == Some(names.finalizer_session.as_str())
        });

        let reactor_session = report
            .and_then(|report| report.get("reactor_session_id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let reactor_triggered = reactor_session.starts_with(&names.run_label)
            && session_in_tree(observation, reactor_session)
            && spawned_by_trigger(context, reactor_session).await?;
        let spawning_event_cited = report
            .and_then(|report| report.get("spawning_event"))
            .and_then(Value::as_str)
            .is_some_and(|event| !event.trim().is_empty());
        let finalizer_triggered = session_in_tree(observation, &names.finalizer_session)
            && spawned_by_trigger(context, &names.finalizer_session).await?;
        let active_run_triggers = active_run_trigger_count(context, &names).await?;

        let workload_passed =
            all_tables_exist && parallel_writers && orders_complete && writers_done;
        let aggregates_passed = totals_match && report_counts_match;
        let orchestration_passed = watch_before_writers
            && fallback_documented
            && reactor_triggered
            && spawning_event_cited;
        let finalization_passed = report_rows.len() == 1
            && report_identity_matches
            && report_checks_pass
            && finalizer_triggered
            && active_run_triggers == 0;

        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "parallel_writers_completed",
                    workload_passed,
                    format!(
                        "tables={all_tables_exist}, parallel_writers={parallel_writers}, \
                         orders_complete={orders_complete}, writers_done={writers_done}"
                    ),
                ),
                common::gate(
                    "reactive_totals_match",
                    aggregates_passed,
                    format!(
                        "totals_match={totals_match}, report_counts_match={report_counts_match}"
                    ),
                ),
                common::gate(
                    "trigger_orchestration_proven",
                    orchestration_passed,
                    format!(
                        "watch_before_writers={watch_before_writers}, \
                         fallback_documented={fallback_documented}, reactor_triggered={reactor_triggered}, \
                         spawning_event_cited={spawning_event_cited}"
                    ),
                ),
                common::gate(
                    "finalizer_and_cleanup_completed",
                    finalization_passed,
                    format!(
                        "report_rows={}, report_identity={report_identity_matches}, \
                         report_checks={report_checks_pass}, finalizer_triggered={finalizer_triggered}, \
                         active_run_triggers={active_run_triggers}",
                        report_rows.len()
                    ),
                ),
            ],
            awards: vec![
                common::award(
                    "parallel_writes",
                    if workload_passed { 25 } else { 0 },
                    "awarded for three parallel writers and 15 valid rows",
                ),
                common::award(
                    "reactive_aggregates",
                    if aggregates_passed { 30 } else { 0 },
                    "awarded when stored totals and event counts cover the source rows exactly",
                ),
                common::award(
                    "trigger_orchestration",
                    if orchestration_passed { 25 } else { 0 },
                    "awarded for bounded discovery, documented fallback, and reactor provenance",
                ),
                common::award(
                    "finalization_cleanup",
                    if finalization_passed { 20 } else { 0 },
                    "awarded for one trigger-spawned finalizer, passing report, and cleanup",
                ),
            ],
        })
    })
}

fn session_in_tree(observation: &ScenarioObservation, session_id: &str) -> bool {
    observation
        .metrics
        .by_session
        .iter()
        .any(|session| session.session_id == session_id)
}

async fn spawned_by_trigger(context: &E2eContext, session_id: &str) -> anyhow::Result<bool> {
    if session_id.is_empty() {
        return Ok(false);
    }
    let session = context
        .trigger_value("session::get", json!({ "session_id": session_id }))
        .await?;
    Ok(session
        .pointer("/meta/metadata/spawned_by")
        .and_then(Value::as_str)
        == Some("trigger"))
}

async fn query_rows(context: &E2eContext, sql: &str) -> anyhow::Result<Vec<Value>> {
    let response = context
        .trigger_value("database::query", json!({ "db": DATABASE, "sql": sql }))
        .await?;
    response
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .with_context(|| format!("database::query returned malformed rows for {sql}"))
}

async fn first_row(context: &E2eContext, sql: &str) -> anyhow::Result<Value> {
    Ok(query_rows(context, sql)
        .await?
        .into_iter()
        .next()
        .unwrap_or(Value::Null))
}

async fn totals_from_table(
    context: &E2eContext,
    table_exists: bool,
    sql: &str,
) -> anyhow::Result<Option<BTreeMap<String, (i64, f64)>>> {
    if !table_exists {
        return Ok(None);
    }
    Ok(grouped_totals(query_rows(context, sql).await?))
}

fn grouped_totals(rows: Vec<Value>) -> Option<BTreeMap<String, (i64, f64)>> {
    rows.into_iter()
        .map(|row| {
            Some((
                row.get("writer")?.as_str()?.to_string(),
                (
                    integer_field(&row, "order_count")?,
                    number_field(&row, "amount_sum")?,
                ),
            ))
        })
        .collect()
}

fn integer_field(value: &Value, field: &str) -> Option<i64> {
    value
        .get(field)
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

fn number_field(value: &Value, field: &str) -> Option<f64> {
    value
        .get(field)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
}

fn boolean_field(value: &Value, field: &str) -> Option<bool> {
    value.get(field).and_then(|value| match value {
        Value::Bool(boolean) => Some(*boolean),
        Value::Number(number) => number.as_i64().map(|number| number != 0),
        Value::String(string) => match string.to_ascii_lowercase().as_str() {
            "true" | "pass" | "passed" | "1" => Some(true),
            "false" | "fail" | "failed" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

async fn existing_tables(
    context: &E2eContext,
    names: &ScenarioNames,
) -> anyhow::Result<BTreeSet<String>> {
    Ok(query_rows(
        context,
        &format!(
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name LIKE '{}_%' ORDER BY name",
            names.table_prefix
        ),
    )
    .await?
    .into_iter()
    .filter_map(|row| row.get("name").and_then(Value::as_str).map(str::to_string))
    .collect())
}

async fn active_run_trigger_count(
    context: &E2eContext,
    names: &ScenarioNames,
) -> anyhow::Result<usize> {
    let response = context
        .trigger_value(
            "engine::registered-triggers::list",
            json!({ "include_internal": true }),
        )
        .await?;
    Ok(response
        .get("registered_triggers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|trigger| {
            let serialized = trigger.to_string();
            serialized.contains(&names.run_label) || serialized.contains(&names.table_prefix)
        })
        .count())
}

fn max_parallel_calls(transcript: &Value, function_id: &str) -> usize {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|block| {
                    if block.get("type").and_then(Value::as_str) != Some("function_call") {
                        return false;
                    }
                    match block.get("function_id").and_then(Value::as_str) {
                        Some(id) if id == function_id => true,
                        Some("agent_trigger") => {
                            block.pointer("/arguments/function").and_then(Value::as_str)
                                == Some(function_id)
                        }
                        _ => false,
                    }
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let names = ScenarioNames::new(run_id);
        for table in existing_tables(context, &names).await? {
            if !table.starts_with(&format!("{}_", names.table_prefix))
                || !table
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                continue;
            }
            let _: Value = context
                .trigger(
                    "database::execute",
                    json!({
                        "db": DATABASE,
                        "sql": format!("DROP TABLE IF EXISTS \"{table}\""),
                    }),
                )
                .await?;
        }
        Ok(())
    })
}

struct ScenarioNames {
    run_label: String,
    table_prefix: String,
    orders: String,
    writers: String,
    totals: String,
    report: String,
    writer_sessions: [String; EXPECTED_WRITERS],
    finalizer_session: String,
}

impl ScenarioNames {
    fn new(run_id: &str) -> Self {
        let mut suffix: String = run_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(4)
            .collect();
        while suffix.len() < 4 {
            suffix.push('0');
        }
        let run_label = format!("rctest-{suffix}");
        let table_prefix = format!("rctest_{suffix}");
        Self {
            orders: format!("{table_prefix}_orders"),
            writers: format!("{table_prefix}_writers"),
            totals: format!("{table_prefix}_totals"),
            report: format!("{table_prefix}_report"),
            writer_sessions: [
                format!("{run_label}-writer-1"),
                format!("{run_label}-writer-2"),
                format!("{run_label}-writer-3"),
            ],
            finalizer_session: format!("{run_label}-finalizer"),
            run_label,
            table_prefix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_unique_and_sql_safe() {
        let names = ScenarioNames::new("aB19-rest");
        assert_eq!(names.run_label, "rctest-aB19");
        assert_eq!(names.orders, "rctest_aB19_orders");
        assert_eq!(names.writer_sessions[2], "rctest-aB19-writer-3");
        assert!(names
            .orders
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'));
    }

    #[test]
    fn grouped_totals_accepts_numeric_strings() {
        let totals = grouped_totals(vec![
            json!({"writer": "writer-1", "order_count": 5, "amount_sum": 25}),
            json!({"writer": "writer-2", "order_count": "5", "amount_sum": "30"}),
        ])
        .unwrap();
        assert_eq!(totals["writer-1"], (5, 25.0));
        assert_eq!(totals["writer-2"], (5, 30.0));
    }

    #[test]
    fn parallel_call_detection_supports_native_and_wrapped_calls() {
        let transcript = json!({
            "messages": [{
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "function_call", "function_id": "harness::spawn", "arguments": {}},
                        {"type": "function_call", "function_id": "agent_trigger",
                         "arguments": {"function": "harness::spawn", "payload": {}}},
                        {"type": "function_call", "function_id": "harness::spawn", "arguments": {}}
                    ]
                }
            }]
        });
        assert_eq!(max_parallel_calls(&transcript, "harness::spawn"), 3);
    }
}
