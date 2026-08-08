use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::{
    common, CleanupFuture, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "receiving_operation";

const DATABASE: &str = "primary";
const SUPPLIERS: [&str; 3] = ["acme", "globex", "initech"];
const DATABASE_WRITES: [&str; 3] = [
    "database::execute",
    "database::executeBatch",
    "database::transaction",
];
/// What a courier may be GRANTED and may CALL. The database surface is the
/// work; the two `engine::functions::*` reads are contract lookup, which the
/// shipped prompts tell every agent to do before calling a function — it
/// grants no medium and no write, so a parent that hands it to a child is
/// still minimal-privilege. The scenario's real constraint is the MEDIUM
/// (database only, no state/timer/cron), enforced by `no_forbidden_calls`.
const COURIER_FUNCTIONS: [&str; 6] = [
    "database::execute",
    "database::executeBatch",
    "database::transaction",
    "database::query",
    "engine::functions::info",
    "engine::functions::list",
];
const COURIER_WORKLOAD: AssessmentSpec = AssessmentSpec::hard_gated(
    "courier_workload",
    30,
    "Three least-privilege couriers produce the exact shipments, corrections, and completion rows.",
);
const LIVE_LEDGER: AssessmentSpec = AssessmentSpec::hard_gated(
    "live_ledger",
    25,
    "A database-native or mechanical reaction keeps the ledger current without model turns.",
);
const DATABASE_FAN_IN: AssessmentSpec = AssessmentSpec::hard_gated(
    "database_fan_in",
    25,
    "The root learns completion from one database wake without polling.",
);
const VERIFICATION_CLEANUP: AssessmentSpec = AssessmentSpec::hard_gated(
    "verification_cleanup",
    20,
    "The root reports verified evidence and removes all standing machinery.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[
    COURIER_WORKLOAD,
    LIVE_LEDGER,
    DATABASE_FAN_IN,
    VERIFICATION_CLEANUP,
];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = Names::new(run_id);
    ScenarioSpec {
        id: ID,
        version: 4,
        prompt: prompt(&names),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 48,
            max_output_tokens: None,
            max_total_tokens: 1_200_000,
            stuck_timeout_seconds: 360,
        },
        denied_functions: &["state::*"],
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn prompt(names: &Names) -> String {
    format!(
        r#"You are running a small receiving operation on this stack. Everything below is a GOAL,
not a recipe — design the mechanics yourself with the tools you have.

Suppliers acme, globex and initech each send one courier. Each courier must record three shipments
(supplier, seq 1-3, value — acme: 10.0, 5.0, 3.0; globex: 7.5, 2.5, 4.0;
initech: 6.0, 8.0, 1.0) into database `primary`, then apply one correction (+1.0
to its seq-2 shipment), then signal that it is finished.

Requirements:

- The ONLY shared medium in this run is database `primary` plus trigger machinery on top of it.
  State functions are not granted. Do not use timers or cron.
- Maintain a live ledger in database `primary` — per supplier: shipment count and total value —
  kept current as data lands WITHOUT running a model turn for each change.
- Delegate the courier work to sub-agents and keep them minimal-privilege.
- Learn that the whole run is complete FROM THE DATABASE ITSELF, waking as few times as possible
  — ideally exactly once — and never by polling.
- When complete, verify the ledger matches the shipment data (expected: acme 3 / 19.0,
  globex 3 / 15.0, initech 3 / 16.0), clean up any standing trigger machinery, and report
  PASS or FAIL with the evidence.

Use this run-isolated database contract so the result can be evaluated:

- shipments: `{shipments}` (`supplier`, `seq`, `value`; primary key `supplier, seq`)
- live ledger: `{ledger}` (`supplier`, `shipment_count`, `total_value`)
- courier status: `{couriers}` (`supplier`, `status`)
- completion signal: `{completion}` (exactly one row when the run is complete)
- courier sessions: `{acme_session}`, `{globex_session}`, `{initech_session}`

Namespace any additional database object, trigger label, or session with `{run_label}`. Keep the
four result relations for verification; cleanup means retiring bindings and database triggers,
not deleting the evidence tables. Finish in this root session only after its database wake."#,
        shipments = names.shipments,
        ledger = names.ledger,
        couriers = names.couriers,
        completion = names.completion,
        acme_session = names.courier_sessions[0],
        globex_session = names.courier_sessions[1],
        initech_session = names.courier_sessions[2],
        run_label = names.run_label,
    )
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        if !context.function_exists("database::query").await? {
            return Ok(missing_database());
        }
        let databases = available_databases(context).await?;
        if !databases.contains(DATABASE) {
            return Ok(missing_primary(&databases));
        }

        let names = Names::new(run_id);
        let calls = timeline_calls(&observation.transcript);
        let objects = database_objects(context, &names).await?;
        let first_spawn = calls
            .iter()
            .find(|call| call.function_id == "harness::spawn")
            .map(|call| call.entry);
        let completion_notice = notification_entries(&observation.transcript)
            .into_iter()
            .find(|(_, text)| text.contains(&names.completion));

        let required_relations = [
            names.shipments.as_str(),
            names.ledger.as_str(),
            names.couriers.as_str(),
            names.completion.as_str(),
        ]
        .iter()
        .all(|name| {
            objects
                .get(*name)
                .is_some_and(|object| matches!(object.kind.as_str(), "table" | "view"))
        });

        let shipments = if objects.contains_key(&names.shipments) {
            shipment_rows(context, &names.shipments).await?
        } else {
            BTreeMap::new()
        };
        let ledger = if objects.contains_key(&names.ledger) {
            ledger_rows(context, &names.ledger).await?
        } else {
            BTreeMap::new()
        };
        let couriers = if objects.contains_key(&names.couriers) {
            courier_rows(context, &names.couriers).await?
        } else {
            BTreeMap::new()
        };
        let completion_rows = if objects.contains_key(&names.completion) {
            count_rows(context, &names.completion).await?
        } else {
            0
        };

        let expected_shipments = expected_shipments();
        let expected_ledger = expected_ledger();
        let shipments_match = numeric_map_matches(&shipments, &expected_shipments);
        let ledger_matches_expected = ledger_matches(&ledger, &expected_ledger);
        let ledger_matches_source = ledger_from_shipments(&shipments)
            .is_some_and(|direct| ledger_matches(&ledger, &direct));
        let couriers_done = SUPPLIERS.iter().all(|supplier| {
            couriers.get(*supplier).is_some_and(|status| {
                matches!(
                    status.to_ascii_lowercase().as_str(),
                    "done" | "finished" | "complete" | "completed"
                )
            })
        }) && couriers.len() == SUPPLIERS.len();

        let spawn_policy_ok = courier_spawns_are_minimal(&calls, &names);
        let courier_execution_ok = courier_execution(context, observation, &names).await?;
        let no_extra_sessions = observation.metrics.by_session.len() == SUPPLIERS.len() + 1;
        let no_shared_medium_violation =
            no_forbidden_calls(&calls) && courier_execution_ok.no_forbidden_calls;

        let live_strategy = first_spawn
            .is_some_and(|spawn| live_ledger_setup_before(&calls, &objects, &names, spawn));
        let no_late_root_repair = completion_notice.as_ref().is_some_and(|(notice, _)| {
            !calls.iter().any(|call| {
                call.entry > *notice
                    && DATABASE_WRITES.contains(&call.function_id.as_str())
                    && sql_statements(&call.arguments)
                        .iter()
                        .any(|sql| mutates_relation(sql, &names.ledger))
            })
        });

        let completion_watch_before_spawn = first_spawn.is_some_and(|spawn| {
            calls
                .iter()
                .any(|call| call.entry < spawn && is_completion_wake(call, &names))
        });
        let wake_records = common::trigger_fired_records(&observation.transcript)
            .into_iter()
            .filter(|record| {
                matches!(
                    record.get("target").and_then(Value::as_str),
                    Some("harness::send" | "notify")
                )
            })
            .count();
        let notifications = notification_entries(&observation.transcript);
        let one_database_wake =
            wake_records == 1 && notifications.len() == 1 && completion_notice.is_some();
        let no_polling = first_spawn
            .zip(completion_notice.as_ref().map(|(entry, _)| *entry))
            .is_some_and(|(spawn, wake)| {
                !calls.iter().any(|call| {
                    (spawn..wake).contains(&call.entry) && is_polling_call(&call.function_id)
                })
            });
        let root_did_not_signal_completion = first_spawn.is_some_and(|spawn| {
            !calls.iter().any(|call| {
                call.entry >= spawn
                    && DATABASE_WRITES.contains(&call.function_id.as_str())
                    && sql_statements(&call.arguments)
                        .iter()
                        .any(|sql| mutates_relation(sql, &names.completion))
            })
        });

        let active_bindings = common::active_binding_count(context, &names.root_session).await?;
        let standing_sql_triggers = objects
            .values()
            .filter(|object| object.kind == "trigger")
            .count();
        let response_has_evidence = valid_final_response(&observation.response);

        let workload_passed = required_relations
            && shipments_match
            && couriers_done
            && completion_rows == 1
            && spawn_policy_ok
            && courier_execution_ok.completed
            && no_extra_sessions
            && no_shared_medium_violation;
        let ledger_passed = live_strategy
            && no_late_root_repair
            && ledger_matches_expected
            && ledger_matches_source;
        let fan_in_passed = completion_watch_before_spawn
            && one_database_wake
            && no_polling
            && root_did_not_signal_completion;
        let cleanup_passed =
            response_has_evidence && active_bindings == 0 && standing_sql_triggers == 0;

        Ok(assessment::build_evaluation([
            COURIER_WORKLOAD.full_or_zero(
                workload_passed,
                format!(
                    "relations={required_relations}, shipments={shipments_match}, \
                         couriers_done={couriers_done}, completion_rows={completion_rows}, \
                         minimal_spawns={spawn_policy_ok}, courier_execution={}, \
                         sessions={}, shared_medium={no_shared_medium_violation}",
                    courier_execution_ok.completed,
                    observation.metrics.by_session.len(),
                ),
            ),
            LIVE_LEDGER.full_or_zero(
                ledger_passed,
                format!(
                    "live_strategy={live_strategy}, no_late_root_repair={no_late_root_repair}, \
                        expected={ledger_matches_expected}, source={ledger_matches_source}"
                ),
            ),
            DATABASE_FAN_IN.full_or_zero(
                fan_in_passed,
                format!(
                    "watch_before_spawn={completion_watch_before_spawn}, \
                         wake_records={wake_records}, notifications={}, no_polling={no_polling}, \
                         root_did_not_signal={root_did_not_signal_completion}",
                    notifications.len(),
                ),
            ),
            VERIFICATION_CLEANUP.full_or_zero(
                cleanup_passed,
                format!(
                    "response_evidence={response_has_evidence}, \
                        active_bindings={active_bindings}, sql_triggers={standing_sql_triggers}"
                ),
            ),
        ]))
    })
}

fn missing_database() -> ObjectiveEvaluation {
    let reason = "database::query is unavailable";
    assessment::prerequisite_failure(ASSESSMENTS, "database_capability_available", reason)
}

fn missing_primary(databases: &BTreeSet<String>) -> ObjectiveEvaluation {
    let reason = format!(
        "database `primary` is unavailable; configured databases: {}",
        databases.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    assessment::prerequisite_failure(ASSESSMENTS, "primary_database_available", reason)
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        if !context.function_exists("database::query").await? {
            return Ok(());
        }
        if !available_databases(context).await?.contains(DATABASE) {
            return Ok(());
        }
        let names = Names::new(run_id);
        let objects = database_objects(context, &names).await?;
        for kind in ["trigger", "view", "table"] {
            for object in objects.values().filter(|object| object.kind == kind) {
                if !sql_safe_name(&object.name) {
                    continue;
                }
                let _: Value = context
                    .trigger(
                        "database::execute",
                        json!({
                            "db": DATABASE,
                            "sql": format!(
                                "DROP {} IF EXISTS \"{}\"",
                                kind.to_ascii_uppercase(),
                                object.name
                            ),
                        }),
                    )
                    .await?;
            }
        }
        Ok(())
    })
}

#[derive(Debug)]
struct CourierExecution {
    completed: bool,
    no_forbidden_calls: bool,
}

async fn courier_execution(
    context: &E2eContext,
    observation: &ScenarioObservation,
    names: &Names,
) -> anyhow::Result<CourierExecution> {
    let mut completed = true;
    let mut no_forbidden_calls = true;
    for (supplier, session_id) in SUPPLIERS.iter().zip(&names.courier_sessions) {
        let metric = observation
            .metrics
            .by_session
            .iter()
            .find(|metric| metric.session_id == *session_id);
        if !metric.is_some_and(|metric| {
            metric.parent_session_id.as_deref() == Some(names.root_session.as_str())
                && metric.depth == 1
        }) {
            completed = false;
            continue;
        }

        let transcript = context.transcript(session_id).await?;
        let calls = common::function_calls(&transcript);
        no_forbidden_calls &= calls
            .iter()
            .all(|call| COURIER_FUNCTIONS.contains(&call.function_id.as_str()));
        let serialized = calls
            .iter()
            .map(|call| call.arguments.to_string().to_ascii_lowercase())
            .collect::<String>();
        let statements: Vec<_> = calls
            .iter()
            .flat_map(|call| sql_statements(&call.arguments))
            .map(str::to_ascii_lowercase)
            .collect();
        completed &= !calls.is_empty()
            && serialized.contains(supplier)
            && courier_writes_are_ordered(&statements, names);
    }
    Ok(CourierExecution {
        completed,
        no_forbidden_calls,
    })
}

fn courier_writes_are_ordered(statements: &[String], names: &Names) -> bool {
    let shipment_insert = statements
        .iter()
        .position(|sql| sql.contains("insert") && sql.contains(&names.shipments));
    let correction = statements
        .iter()
        .position(|sql| sql.contains("update") && sql.contains(&names.shipments));
    let done = statements
        .iter()
        .position(|sql| mutates_relation(sql, &names.couriers));
    shipment_insert
        .zip(correction)
        .zip(done)
        .is_some_and(|((insert, update), done)| insert < update && update < done)
}

fn courier_spawns_are_minimal(calls: &[CallAt], names: &Names) -> bool {
    let spawns: Vec<_> = calls
        .iter()
        .filter(|call| call.function_id == "harness::spawn")
        .filter(|call| {
            call.arguments
                .get("session_id")
                .and_then(Value::as_str)
                .is_some_and(|session_id| {
                    names
                        .courier_sessions
                        .iter()
                        .any(|expected| expected == session_id)
                })
        })
        .collect();
    let expected: BTreeSet<_> = names.courier_sessions.iter().map(String::as_str).collect();
    spawns.len() == SUPPLIERS.len()
        && spawns
            .iter()
            .filter_map(|call| call.arguments.get("session_id").and_then(Value::as_str))
            .collect::<BTreeSet<_>>()
            == expected
        && spawns.iter().all(|call| {
            call.arguments
                .pointer("/options/orchestrator")
                .and_then(Value::as_bool)
                != Some(true)
                && call
                    .arguments
                    .pointer("/options/functions/allow")
                    .and_then(Value::as_array)
                    .is_some_and(|allow| {
                        !allow.is_empty()
                            && allow.iter().all(|function| {
                                function
                                    .as_str()
                                    .is_some_and(|function| COURIER_FUNCTIONS.contains(&function))
                            })
                    })
        })
}

fn no_forbidden_calls(calls: &[CallAt]) -> bool {
    calls.iter().all(|call| {
        !call.function_id.starts_with("state::")
            && !call.function_id.starts_with("cron::")
            && !(call.function_id == "engine::register_trigger"
                && matches!(
                    call.arguments.get("trigger_type").and_then(Value::as_str),
                    Some("timer" | "cron")
                ))
    })
}

fn live_ledger_setup_before(
    calls: &[CallAt],
    objects: &BTreeMap<String, DatabaseObject>,
    names: &Names,
    first_spawn: usize,
) -> bool {
    let ledger_is_view = objects.get(&names.ledger).is_some_and(|object| {
        object.kind == "view" && object.sql.to_ascii_lowercase().contains(&names.shipments)
    }) || calls.iter().any(|call| {
        call.entry < first_spawn
            && sql_statements(&call.arguments).iter().any(|sql| {
                let sql = sql.to_ascii_lowercase();
                sql.contains("create view")
                    && sql.contains(&names.ledger)
                    && sql.contains(&names.shipments)
            })
    });
    let database_trigger = calls.iter().any(|call| {
        call.entry < first_spawn
            && sql_statements(&call.arguments).iter().any(|sql| {
                let sql = sql.to_ascii_lowercase();
                sql.contains("create trigger")
                    && sql.contains(&names.ledger)
                    && sql.contains(&names.shipments)
            })
    });
    let mechanical_binding = calls.iter().any(|call| {
        call.entry < first_spawn
            && call.function_id == "engine::register_trigger"
            && call.arguments.get("trigger_type").and_then(Value::as_str)
                == Some("database::row-changed")
            && call.arguments.pointer("/config/db").and_then(Value::as_str) == Some(DATABASE)
            && call
                .arguments
                .pointer("/config/table")
                .and_then(Value::as_str)
                == Some(names.shipments.as_str())
            && target_function(&call.arguments)
                .is_some_and(|target| DATABASE_WRITES.contains(&target))
    });
    ledger_is_view || database_trigger || mechanical_binding
}

fn is_completion_wake(call: &CallAt, names: &Names) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("trigger_type").and_then(Value::as_str)
            == Some("database::row-changed")
        && call.arguments.pointer("/config/db").and_then(Value::as_str) == Some(DATABASE)
        && call
            .arguments
            .pointer("/config/table")
            .and_then(Value::as_str)
            == Some(names.completion.as_str())
        && wake_is_once(&call.arguments)
        && common::is_wake_registration(&call.arguments)
}

fn wake_is_once(arguments: &Value) -> bool {
    common::requested_once(arguments)
        || (arguments.get("once").is_none() && arguments.pointer("/lifecycle/once").is_none())
        || arguments
            .pointer("/lifecycle/max_fires")
            .and_then(Value::as_u64)
            == Some(1)
}

fn target_function(arguments: &Value) -> Option<&str> {
    arguments
        .get("function_id")
        .and_then(Value::as_str)
        .or_else(|| {
            arguments
                .pointer("/target/function_id")
                .and_then(Value::as_str)
        })
}

fn is_polling_call(function_id: &str) -> bool {
    matches!(
        function_id,
        "database::query"
            | "harness::status"
            | "harness::metrics"
            | "harness::session-tree"
            | "session::get"
            | "session::messages"
    )
}

fn valid_final_response(response: &str) -> bool {
    let response = response.to_ascii_lowercase();
    response.contains("pass")
        && [("acme", "19"), ("globex", "15"), ("initech", "16")]
            .iter()
            .all(|(supplier, total)| response.contains(supplier) && response.contains(total))
}

fn notification_entries(transcript: &Value) -> Vec<(usize, String)> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(entry, item)| {
            let message = item.get("message")?;
            (message.get("role").and_then(Value::as_str) == Some("user")
                && item
                    .pointer("/origin/notification")
                    .and_then(Value::as_bool)
                    == Some(true))
            .then(|| {
                let text = message
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect::<String>();
                (entry, text)
            })
        })
        .collect()
}

#[derive(Debug)]
struct CallAt {
    entry: usize,
    function_id: String,
    arguments: Value,
}

fn timeline_calls(transcript: &Value) -> Vec<CallAt> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .flat_map(|(entry, item)| {
            item.get("message")
                .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(move |block| normalize_call(entry, block))
        })
        .collect()
}

fn normalize_call(entry: usize, block: &Value) -> Option<CallAt> {
    if block.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let function_id = block.get("function_id")?.as_str()?;
    let arguments = block.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if function_id == "agent_trigger" {
        return Some(CallAt {
            entry,
            function_id: arguments.get("function")?.as_str()?.to_string(),
            arguments: arguments
                .get("payload")
                .cloned()
                .unwrap_or_else(|| json!({})),
        });
    }
    Some(CallAt {
        entry,
        function_id: function_id.to_string(),
        arguments,
    })
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

#[derive(Debug)]
struct DatabaseObject {
    name: String,
    kind: String,
    sql: String,
}

async fn database_objects(
    context: &E2eContext,
    names: &Names,
) -> anyhow::Result<BTreeMap<String, DatabaseObject>> {
    let rows = query_rows(
        context,
        &format!(
            "SELECT type, name, COALESCE(sql, '') AS sql FROM sqlite_master \
             WHERE name LIKE '{}%' OR (type = 'trigger' AND sql LIKE '%{}%') \
             ORDER BY type, name",
            names.table_prefix, names.table_prefix,
        ),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let object = DatabaseObject {
                name: row.get("name")?.as_str()?.to_string(),
                kind: row.get("type")?.as_str()?.to_string(),
                sql: row
                    .get("sql")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            };
            Some((object.name.clone(), object))
        })
        .collect())
}

async fn shipment_rows(
    context: &E2eContext,
    table: &str,
) -> anyhow::Result<BTreeMap<(String, i64), f64>> {
    query_rows(
        context,
        &format!("SELECT supplier, seq, value FROM {table} ORDER BY supplier, seq"),
    )
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            (string_field(&row, "supplier")?, integer_field(&row, "seq")?),
            number_field(&row, "value")?,
        ))
    })
    .collect()
}

async fn ledger_rows(
    context: &E2eContext,
    table: &str,
) -> anyhow::Result<BTreeMap<String, (i64, f64)>> {
    query_rows(
        context,
        &format!("SELECT supplier, shipment_count, total_value FROM {table} ORDER BY supplier"),
    )
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            string_field(&row, "supplier")?,
            (
                integer_field(&row, "shipment_count")?,
                number_field(&row, "total_value")?,
            ),
        ))
    })
    .collect()
}

async fn courier_rows(
    context: &E2eContext,
    table: &str,
) -> anyhow::Result<BTreeMap<String, String>> {
    query_rows(
        context,
        &format!("SELECT supplier, status FROM {table} ORDER BY supplier"),
    )
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            string_field(&row, "supplier")?,
            string_field(&row, "status")?,
        ))
    })
    .collect()
}

async fn count_rows(context: &E2eContext, table: &str) -> anyhow::Result<i64> {
    let row = query_rows(context, &format!("SELECT COUNT(*) AS count FROM {table}"))
        .await?
        .into_iter()
        .next()
        .context("count query returned no row")?;
    integer_field(&row, "count")
}

async fn query_rows(context: &E2eContext, sql: &str) -> anyhow::Result<Vec<Value>> {
    context
        .trigger_value("database::query", json!({ "db": DATABASE, "sql": sql }))
        .await?
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .with_context(|| format!("database::query returned malformed rows for {sql}"))
}

async fn available_databases(context: &E2eContext) -> anyhow::Result<BTreeSet<String>> {
    Ok(context
        .trigger_value("database::listDatabases", json!({}))
        .await?
        .get("databases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|database| {
            database
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn string_field(value: &Value, field: &str) -> anyhow::Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("row is missing text field {field}"))
}

fn integer_field(value: &Value, field: &str) -> anyhow::Result<i64> {
    value
        .get(field)
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .with_context(|| format!("row is missing integer field {field}"))
}

fn number_field(value: &Value, field: &str) -> anyhow::Result<f64> {
    value
        .get(field)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        .with_context(|| format!("row is missing numeric field {field}"))
}

fn expected_shipments() -> BTreeMap<(String, i64), f64> {
    [
        ("acme", [10.0, 6.0, 3.0]),
        ("globex", [7.5, 3.5, 4.0]),
        ("initech", [6.0, 9.0, 1.0]),
    ]
    .into_iter()
    .flat_map(|(supplier, values)| {
        values
            .into_iter()
            .enumerate()
            .map(move |(index, value)| ((supplier.to_string(), index as i64 + 1), value))
    })
    .collect()
}

fn expected_ledger() -> BTreeMap<String, (i64, f64)> {
    BTreeMap::from([
        ("acme".to_string(), (3, 19.0)),
        ("globex".to_string(), (3, 15.0)),
        ("initech".to_string(), (3, 16.0)),
    ])
}

fn ledger_from_shipments(
    shipments: &BTreeMap<(String, i64), f64>,
) -> Option<BTreeMap<String, (i64, f64)>> {
    if shipments.is_empty() {
        return None;
    }
    let mut ledger = BTreeMap::new();
    for ((supplier, _), value) in shipments {
        let entry = ledger.entry(supplier.clone()).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += value;
    }
    Some(ledger)
}

fn numeric_map_matches<K: Ord>(actual: &BTreeMap<K, f64>, expected: &BTreeMap<K, f64>) -> bool {
    actual.len() == expected.len()
        && actual.iter().all(|(key, value)| {
            expected
                .get(key)
                .is_some_and(|expected| approximately(*value, *expected))
        })
}

fn ledger_matches(
    actual: &BTreeMap<String, (i64, f64)>,
    expected: &BTreeMap<String, (i64, f64)>,
) -> bool {
    actual.len() == expected.len()
        && actual.iter().all(|(supplier, (count, total))| {
            expected
                .get(supplier)
                .is_some_and(|expected| *count == expected.0 && approximately(*total, expected.1))
        })
}

fn approximately(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.000_001
}

fn sql_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

struct Names {
    run_label: String,
    table_prefix: String,
    root_session: String,
    shipments: String,
    ledger: String,
    couriers: String,
    completion: String,
    courier_sessions: [String; 3],
}

impl Names {
    fn new(run_id: &str) -> Self {
        let mut suffix: String = run_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(8)
            .collect();
        while suffix.len() < 8 {
            suffix.push('0');
        }
        let run_label = format!("receiving-{suffix}");
        let table_prefix = format!("receiving_{suffix}");
        Self {
            root_session: format!("e2e_{run_id}"),
            shipments: format!("{table_prefix}_shipments"),
            ledger: format!("{table_prefix}_ledger"),
            couriers: format!("{table_prefix}_couriers"),
            completion: format!("{table_prefix}_completion"),
            courier_sessions: [
                format!("{run_label}-acme"),
                format!("{run_label}-globex"),
                format!("{run_label}-initech"),
            ],
            run_label,
            table_prefix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_is_valid_and_removes_state_capability() {
        let scenario = scenario("aB19");
        scenario.validate().unwrap();
        assert_eq!(scenario.version, 4);
        assert_eq!(scenario.denied_functions, ["state::*"]);
        assert!(!scenario.needs_judge());
        assert_eq!(
            scenario
                .criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("courier_workload", 30),
                ("live_ledger", 25),
                ("database_fan_in", 25),
                ("verification_cleanup", 20),
            ]
        );
    }

    #[test]
    fn accepts_minimal_courier_policies_and_both_live_ledger_shapes() {
        let names = Names::new("aB19");
        let mut calls = Vec::new();
        calls.push(CallAt {
            entry: 0,
            function_id: "engine::register_trigger".to_string(),
            arguments: json!({
                "trigger_type": "database::row-changed",
                "config": { "db": DATABASE, "table": names.shipments },
                "target": {
                    "function_id": "database::execute",
                    "payload": { "db": DATABASE, "sql": "UPDATE ledger" }
                },
                "once": false
            }),
        });
        for session_id in &names.courier_sessions {
            calls.push(CallAt {
                entry: 1,
                function_id: "harness::spawn".to_string(),
                arguments: json!({
                    "session_id": session_id,
                    "options": {
                        "functions": { "allow": ["database::executeBatch"] }
                    }
                }),
            });
        }

        assert!(courier_spawns_are_minimal(&calls, &names));
        assert!(live_ledger_setup_before(
            &calls,
            &BTreeMap::new(),
            &names,
            1
        ));

        calls[1].arguments["options"]["functions"]["allow"] = json!(["database::*"]);
        assert!(!courier_spawns_are_minimal(&calls, &names));
    }

    #[test]
    fn expected_ledger_is_derived_from_corrected_shipments() {
        assert!(ledger_matches(
            &ledger_from_shipments(&expected_shipments()).unwrap(),
            &expected_ledger()
        ));
    }

    #[test]
    fn courier_can_signal_done_by_updating_its_precreated_status_row() {
        let names = Names::new("ab19");
        let statements = [
            format!("insert into {} values ('acme', 1, 10)", names.shipments),
            format!("update {} set value = 11", names.shipments),
            format!("update {} set status = 'done'", names.couriers),
        ];

        assert!(courier_writes_are_ordered(&statements, &names));
    }

    #[test]
    fn missing_database_emits_one_prerequisite_gate_and_ordered_zero_awards() {
        assert_prerequisite_failure(
            missing_database(),
            "database_capability_available",
            "database::query is unavailable",
        );
    }

    #[test]
    fn missing_primary_emits_one_prerequisite_gate_and_ordered_zero_awards() {
        assert_prerequisite_failure(
            missing_primary(&BTreeSet::from([
                "mysql".to_string(),
                "postgres".to_string(),
                "sqlite".to_string(),
            ])),
            "primary_database_available",
            "postgres",
        );
    }

    fn assert_prerequisite_failure(
        evaluation: ObjectiveEvaluation,
        gate_id: &str,
        reason_fragment: &str,
    ) {
        assert_eq!(evaluation.hard_gates.len(), 1);
        assert_eq!(evaluation.hard_gates[0].id, gate_id);
        assert!(!evaluation.hard_gates[0].passed);
        assert!(evaluation.hard_gates[0].reason.contains(reason_fragment));
        let prerequisite_reason = &evaluation.hard_gates[0].reason;
        assert_eq!(
            evaluation
                .awards
                .iter()
                .map(|award| (award.id.as_str(), award.awarded))
                .collect::<Vec<_>>(),
            vec![
                ("courier_workload", 0),
                ("live_ledger", 0),
                ("database_fan_in", 0),
                ("verification_cleanup", 0),
            ]
        );
        assert!(evaluation
            .awards
            .iter()
            .all(|award| &award.reason == prerequisite_reason));
    }
}
