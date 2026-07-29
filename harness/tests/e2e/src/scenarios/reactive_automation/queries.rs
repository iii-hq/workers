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

    let writer_spawns = WriterSpawnEvidence {
        call_count: calls
            .iter()
            .filter(|call| call.function_id == "harness::spawn")
            .count(),
        session_ids: calls
            .iter()
            .filter(|call| call.function_id == "harness::spawn")
            .filter_map(|call| {
                call.arguments
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect(),
        max_parallel_calls: max_parallel_calls(&observation.transcript, "harness::spawn"),
        max_concurrent_sessions: max_concurrent_sessions(
            &writer_activity_windows(context, &names.writer_sessions).await?,
        ),
        sessions_in_tree,
    };

    let watch = WatchEvidence {
        first_writer_spawn: calls
            .iter()
            .position(|call| call.function_id == "harness::spawn"),
        trigger_catalog: calls
            .iter()
            .position(|call| call.function_id == "engine::triggers::list"),
        row_change_probe: calls.iter().position(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str)
                    == Some("database::row-changed")
        }),
        state_reaction: calls.iter().position(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
                && call.arguments.get("function_id").and_then(Value::as_str)
                    == Some("harness::react")
        }),
    };

    let existing_tables = existing_tables(context, names).await?;

    let order_summary = if existing_tables.contains(&names.orders) {
        Some(OrderSummary::from_value(
            &first_row(
                context,
                &format!(
                    "SELECT COUNT(*) AS rows_written, COUNT(DISTINCT id) AS distinct_ids, \
                         COUNT(DISTINCT writer) AS writer_count, \
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
    let reactor_session = reports
        .first()
        .filter(|_| reports.len() == 1)
        .and_then(|report| report.reactor_session_id.as_deref())
        .unwrap_or_default();
    let reactor_spawned_by_trigger = reactor_session.starts_with(&names.run_label)
        && writer_spawns.sessions_in_tree.contains(reactor_session)
        && spawned_by_trigger(context, reactor_session).await?;
    let finalizer_spawned_by_trigger = writer_spawns
        .sessions_in_tree
        .contains(&names.finalizer_session)
        && spawned_by_trigger(context, &names.finalizer_session).await?;

    Ok(Evidence {
        existing_tables,
        writer_spawns,
        watch,
        order_summary,
        writers,
        direct_totals,
        stored_totals,
        reports,
        reactor_spawned_by_trigger,
        finalizer_spawned_by_trigger,
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
