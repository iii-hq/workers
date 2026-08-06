use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde_json::{json, Value};

use crate::context::E2eContext;

use super::evidence::{
    integer_field, Evidence, FinalReport, OrderSummary, Totals, WatchEvidence, WriterSpawnEvidence,
    WriterStatus,
};
use super::names::ScenarioNames;
use crate::scenarios::{common, ScenarioObservation};

const DATABASE: &str = "primary";

pub(super) async fn collect(
    context: &E2eContext,
    observation: &ScenarioObservation,
    names: &ScenarioNames,
) -> anyhow::Result<Evidence> {
    let calls = common::function_calls(&observation.transcript);
    let sessions_in_tree: BTreeSet<_> = observation
        .metrics
        .by_session
        .iter()
        .map(|session| session.session_id.clone())
        .collect();
    let expected_writer_sessions: BTreeSet<_> = names.writer_sessions.iter().cloned().collect();
    let writer_calls: Vec<_> = calls
        .iter()
        .filter(|call| {
            call.function_id == "harness::spawn"
                && call
                    .arguments
                    .get("session_id")
                    .and_then(Value::as_str)
                    .is_some_and(|session| expected_writer_sessions.contains(session))
        })
        .collect();

    let writer_spawns = WriterSpawnEvidence {
        call_count: writer_calls.len(),
        session_ids: writer_calls
            .iter()
            .filter_map(|call| {
                call.arguments
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect(),
        max_parallel_calls: max_parallel_writer_calls(
            &observation.transcript,
            &expected_writer_sessions,
        ),
        max_concurrent_sessions: max_concurrent_sessions(
            &writer_activity_windows(context, &names.writer_sessions).await?,
        ),
        sessions_in_tree,
    };

    let watch = WatchEvidence {
        first_writer_spawn: calls.iter().position(|call| {
            call.function_id == "harness::spawn"
                && call
                    .arguments
                    .get("session_id")
                    .and_then(Value::as_str)
                    .is_some_and(|session| expected_writer_sessions.contains(session))
        }),
        trigger_catalog: calls
            .iter()
            .position(|call| call.function_id == "engine::triggers::list"),
        row_change_probe: calls.iter().position(|call| is_probe_wake(call, names)),
        aggregate_reaction: calls
            .iter()
            .position(|call| is_aggregate_reaction(call, names)),
        completion_wake: calls
            .iter()
            .position(|call| is_completion_wake(call, names)),
    };

    let existing_tables = existing_tables(context, names).await?;

    let order_summary = if existing_tables.contains(&names.orders) {
        Some(OrderSummary::from_value(
            &first_row(
                context,
                &format!(
                    "SELECT COUNT(*) AS rows_written, COUNT(DISTINCT id) AS distinct_ids, \
                         COUNT(DISTINCT writer) AS writer_count, \
                         SUM(CASE WHEN typeof(amount) IN ('integer', 'real') \
                             THEN 0 ELSE 1 END) AS invalid_amounts, \
                         SUM(CASE WHEN created_at IS NULL THEN 1 ELSE 0 END) AS missing_created_at \
                         FROM {}",
                    names.orders
                ),
            )
            .await?,
        ))
    } else {
        None
    };

    let writers = if existing_tables.contains(&names.writers) {
        query_rows(
            context,
            &format!(
                "SELECT writer, status FROM {} ORDER BY writer",
                names.writers
            ),
        )
        .await?
        .iter()
        .map(WriterStatus::from_value)
        .collect()
    } else {
        Vec::new()
    };

    let direct_totals = totals_from_table(
        context,
        existing_tables.contains(&names.orders),
        &format!(
            "SELECT writer, COUNT(*) AS order_count, SUM(amount) AS amount_sum \
             FROM {} GROUP BY writer ORDER BY writer",
            names.orders
        ),
    )
    .await?;
    let stored_totals = totals_from_table(
        context,
        existing_tables.contains(&names.totals),
        &format!(
            "SELECT writer, order_count, amount_sum FROM {} ORDER BY writer",
            names.totals
        ),
    )
    .await?;

    let reports: Vec<_> = if existing_tables.contains(&names.report) {
        query_rows(context, &format!("SELECT * FROM {}", names.report))
            .await?
            .iter()
            .map(FinalReport::from_value)
            .collect()
    } else {
        Vec::new()
    };
    let records = common::trigger_fired_records(&observation.transcript);
    let aggregate_label = format!("{}-aggregate", names.run_label);
    let completion_label = format!("{}-writers-complete", names.run_label);
    let report_label = format!("{}-report-ready", names.run_label);
    let aggregate_deliveries = records
        .iter()
        .filter(|record| {
            record.get("label").and_then(Value::as_str) == Some(aggregate_label.as_str())
                && record.get("target").and_then(Value::as_str) == Some("database::execute")
        })
        .count();
    let completion_wake_delivered = records.iter().any(|record| {
        record.get("label").and_then(Value::as_str) == Some(completion_label.as_str())
            && record.get("target").and_then(Value::as_str) == Some("harness::send")
            && record.get("retired").and_then(Value::as_bool) == Some(true)
    });
    let report_wake_delivered = records.iter().any(|record| {
        record.get("label").and_then(Value::as_str) == Some(report_label.as_str())
            && record.get("target").and_then(Value::as_str) == Some("harness::send")
            && record.get("retired").and_then(Value::as_bool) == Some(true)
    });

    let finalizer_spawns: Vec<_> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| {
            call.function_id == "harness::spawn"
                && call.arguments.get("session_id").and_then(Value::as_str)
                    == Some(names.finalizer_session.as_str())
        })
        .collect();
    let report_wake = calls.iter().position(|call| is_report_wake(call, names));
    let report_wake_before_finalizer = report_wake
        .zip(finalizer_spawns.first().map(|(position, _)| *position))
        .is_some_and(|(wake, finalizer)| wake < finalizer);
    let finalizer_in_tree = observation.metrics.by_session.iter().any(|session| {
        session.session_id == names.finalizer_session
            && session.parent_session_id.as_deref()
                == Some(observation.metrics.root_session_id.as_str())
            && session.depth == 1
    });
    let finalizer_wrote_report = if finalizer_in_tree {
        common::function_calls(&context.transcript(&names.finalizer_session).await?)
            .iter()
            .any(|call| writes_relation(call, &names.report))
    } else {
        false
    };
    let root_wrote_report = calls
        .iter()
        .any(|call| writes_relation(call, &names.report));

    Ok(Evidence {
        existing_tables,
        writer_spawns,
        watch,
        order_summary,
        writers,
        direct_totals,
        stored_totals,
        reports,
        aggregate_deliveries,
        completion_wake_delivered,
        report_wake_before_finalizer,
        report_wake_delivered,
        finalizer_spawn_count: finalizer_spawns.len(),
        finalizer_in_tree,
        finalizer_wrote_report,
        root_wrote_report,
        active_run_triggers: active_run_trigger_count(context, names).await?,
    })
}

pub(super) async fn existing_tables(
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

pub(super) async fn drop_table(context: &E2eContext, table: &str) -> anyhow::Result<()> {
    let _: Value = context
        .trigger(
            "database::execute",
            json!({
                "db": DATABASE,
                "sql": format!("DROP TABLE IF EXISTS \"{table}\""),
            }),
        )
        .await?;
    Ok(())
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
) -> anyhow::Result<Option<Totals>> {
    if !table_exists {
        return Ok(None);
    }
    Ok(grouped_totals(query_rows(context, sql).await?))
}

fn grouped_totals(rows: Vec<Value>) -> Option<Totals> {
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

fn number_field(value: &Value, field: &str) -> Option<f64> {
    value
        .get(field)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
}

fn is_probe_wake(call: &common::ObservedFunctionCall, names: &ScenarioNames) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("label").and_then(Value::as_str)
            == Some(format!("{}-probe", names.run_label).as_str())
        && is_database_watch(&call.arguments, &names.orders)
        && common::requested_once(&call.arguments)
        && common::is_wake_registration(&call.arguments)
}

fn is_aggregate_reaction(call: &common::ObservedFunctionCall, names: &ScenarioNames) -> bool {
    if call.function_id != "engine::register_trigger"
        || call.arguments.get("label").and_then(Value::as_str)
            != Some(format!("{}-aggregate", names.run_label).as_str())
        || !is_database_watch(&call.arguments, &names.orders)
        || call
            .arguments
            .pointer("/lifecycle/max_fires")
            .and_then(Value::as_u64)
            != Some(15)
    {
        return false;
    }

    let (function_id, payload) = binding_target(&call.arguments);
    let sql = payload
        .and_then(|payload| {
            payload
                .get("sql")
                .or_else(|| payload.get("query"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    function_id == Some("database::execute")
        && payload
            .and_then(|payload| payload.get("db"))
            .and_then(Value::as_str)
            == Some(DATABASE)
        && sql.contains("insert")
        && sql.contains(&names.orders.to_ascii_lowercase())
        && sql.contains(&names.totals.to_ascii_lowercase())
        && sql.contains("group by")
        && !sql.contains("delete")
}

fn is_completion_wake(call: &common::ObservedFunctionCall, names: &ScenarioNames) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("label").and_then(Value::as_str)
            == Some(format!("{}-writers-complete", names.run_label).as_str())
        && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
        && call
            .arguments
            .pointer("/config/scope")
            .and_then(Value::as_str)
            == Some(names.run_label.as_str())
        && call
            .arguments
            .pointer("/config/key")
            .and_then(Value::as_str)
            == Some("writer_done")
        && common::requested_once(&call.arguments)
        && common::is_wake_registration(&call.arguments)
        && call
            .arguments
            .get("conditions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(barrier_covers_writers)
}

fn is_report_wake(call: &common::ObservedFunctionCall, names: &ScenarioNames) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("label").and_then(Value::as_str)
            == Some(format!("{}-report-ready", names.run_label).as_str())
        && is_database_watch(&call.arguments, &names.report)
        && common::requested_once(&call.arguments)
        && common::is_wake_registration(&call.arguments)
}

fn is_database_watch(arguments: &Value, table: &str) -> bool {
    arguments.get("trigger_type").and_then(Value::as_str) == Some("database::row-changed")
        && arguments.pointer("/config/db").and_then(Value::as_str) == Some(DATABASE)
        && arguments.pointer("/config/table").and_then(Value::as_str) == Some(table)
}

fn binding_target(arguments: &Value) -> (Option<&str>, Option<&Value>) {
    if let Some(target) = arguments.get("target").filter(|target| !target.is_null()) {
        (
            target.get("function_id").and_then(Value::as_str),
            target.get("payload"),
        )
    } else {
        (
            arguments.get("function_id").and_then(Value::as_str),
            arguments.pointer("/metadata/payload"),
        )
    }
}

fn barrier_covers_writers(condition: &Value) -> bool {
    let expected = BTreeSet::from(["writer-1", "writer-2", "writer-3"]);
    condition.get("function_id").and_then(Value::as_str) == Some("state::barrier")
        && condition
            .pointer("/config/expect")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>()
            == expected
        && condition
            .pointer("/config/key_from")
            .and_then(Value::as_str)
            == Some("/new_value/writer")
}

fn writes_relation(call: &common::ObservedFunctionCall, relation: &str) -> bool {
    matches!(
        call.function_id.as_str(),
        "database::execute" | "database::executeBatch" | "database::transaction"
    ) && sql_statements(&call.arguments)
        .into_iter()
        .any(|sql| mutates_relation(sql, relation))
}

fn sql_statements(arguments: &Value) -> Vec<&str> {
    arguments
        .get("sql")
        .and_then(Value::as_str)
        .into_iter()
        .chain(
            arguments
                .get("statements")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|statement| {
                    statement
                        .as_str()
                        .or_else(|| statement.get("sql").and_then(Value::as_str))
                }),
        )
        .collect()
}

fn mutates_relation(sql: &str, relation: &str) -> bool {
    let sql = sql.to_ascii_lowercase();
    let relation = relation.to_ascii_lowercase();
    let into =
        sql.contains(&format!("into {relation}")) || sql.contains(&format!("into \"{relation}\""));
    let update = sql.contains(&format!("update {relation}"))
        || sql.contains(&format!("update \"{relation}\""));
    let from =
        sql.contains(&format!("from {relation}")) || sql.contains(&format!("from \"{relation}\""));
    ((sql.contains("insert") || sql.contains("replace")) && into)
        || update
        || (sql.contains("delete") && from)
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

fn max_parallel_writer_calls(transcript: &Value, expected_sessions: &BTreeSet<String>) -> usize {
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
                    spawn_session(block).is_some_and(|session| expected_sessions.contains(session))
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn spawn_session(block: &Value) -> Option<&str> {
    match block.get("function_id").and_then(Value::as_str) {
        Some("harness::spawn") => block
            .pointer("/arguments/session_id")
            .and_then(Value::as_str),
        Some("agent_trigger")
            if block.pointer("/arguments/function").and_then(Value::as_str)
                == Some("harness::spawn") =>
        {
            block
                .pointer("/arguments/payload/session_id")
                .and_then(Value::as_str)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityWindow {
    started_at: i64,
    finished_at: i64,
}

async fn writer_activity_windows(
    context: &E2eContext,
    session_ids: &[String],
) -> anyhow::Result<BTreeMap<String, ActivityWindow>> {
    let mut windows = BTreeMap::new();
    for session_id in session_ids {
        let transcript = context.transcript(session_id).await?;
        if let Some(window) = activity_window(&transcript) {
            windows.insert(session_id.clone(), window);
        }
    }
    Ok(windows)
}

fn activity_window(transcript: &Value) -> Option<ActivityWindow> {
    let mut timestamps = transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("assistant" | "function_result")
            )
        })
        .filter_map(|message| message.get("timestamp").and_then(Value::as_i64));
    let first = timestamps.next()?;
    let (started_at, finished_at) =
        timestamps.fold((first, first), |(started_at, finished_at), timestamp| {
            (started_at.min(timestamp), finished_at.max(timestamp))
        });
    Some(ActivityWindow {
        started_at,
        finished_at,
    })
}

fn max_concurrent_sessions(windows: &BTreeMap<String, ActivityWindow>) -> usize {
    windows
        .values()
        .map(|candidate| {
            windows
                .values()
                .filter(|window| {
                    window.started_at <= candidate.started_at
                        && candidate.started_at < window.finished_at
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parallel_call_detection_ignores_non_writer_spawns() {
        let expected = BTreeSet::from([
            "writer-1".to_string(),
            "writer-2".to_string(),
            "writer-3".to_string(),
        ]);
        let transcript = json!({
            "messages": [{
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "function_call", "function_id": "harness::spawn",
                         "arguments": {"session_id": "writer-1"}},
                        {"type": "function_call", "function_id": "agent_trigger",
                         "arguments": {"function": "harness::spawn",
                                       "payload": {"session_id": "writer-2"}}},
                        {"type": "function_call", "function_id": "harness::spawn",
                         "arguments": {"session_id": "writer-3"}},
                        {"type": "function_call", "function_id": "harness::spawn",
                         "arguments": {"session_id": "finalizer"}}
                    ]
                }
            }]
        });
        assert_eq!(max_parallel_writer_calls(&transcript, &expected), 3);
    }

    #[test]
    fn activity_window_uses_model_execution_messages() {
        let transcript = json!({
            "messages": [
                {"message": {"role": "user", "timestamp": 1}},
                {"message": {"role": "assistant", "timestamp": 10}},
                {"message": {"role": "function_result", "timestamp": 20}},
                {"message": {"role": "assistant", "timestamp": 30}}
            ]
        });

        assert_eq!(
            activity_window(&transcript),
            Some(ActivityWindow {
                started_at: 10,
                finished_at: 30,
            })
        );
    }

    #[test]
    fn concurrent_session_detection_accepts_overlapping_writer_activity() {
        let windows = BTreeMap::from([
            (
                "writer-1".to_string(),
                ActivityWindow {
                    started_at: 10,
                    finished_at: 40,
                },
            ),
            (
                "writer-2".to_string(),
                ActivityWindow {
                    started_at: 20,
                    finished_at: 50,
                },
            ),
            (
                "writer-3".to_string(),
                ActivityWindow {
                    started_at: 30,
                    finished_at: 60,
                },
            ),
        ]);

        assert_eq!(max_concurrent_sessions(&windows), 3);
    }

    #[test]
    fn concurrent_session_detection_rejects_non_overlapping_writer_activity() {
        let windows = BTreeMap::from([
            (
                "writer-1".to_string(),
                ActivityWindow {
                    started_at: 10,
                    finished_at: 20,
                },
            ),
            (
                "writer-2".to_string(),
                ActivityWindow {
                    started_at: 20,
                    finished_at: 30,
                },
            ),
            (
                "writer-3".to_string(),
                ActivityWindow {
                    started_at: 30,
                    finished_at: 40,
                },
            ),
        ]);

        assert_eq!(max_concurrent_sessions(&windows), 1);
    }
}
