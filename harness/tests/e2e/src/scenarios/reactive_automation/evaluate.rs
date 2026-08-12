use std::collections::BTreeSet;

use super::evidence::{Evidence, FinalReport};
use super::names::ScenarioNames;
use super::{ASSESSMENTS, EXPECTED_ORDERS, EXPECTED_WRITERS, ORDERS_PER_WRITER};
use super::{FINALIZATION_CLEANUP, PARALLEL_WRITES, REACTIVE_AGGREGATES, TRIGGER_ORCHESTRATION};
use crate::scenarios::{assessment, ObjectiveEvaluation};

pub(super) fn missing_database() -> ObjectiveEvaluation {
    assessment::prerequisite_failure(
        ASSESSMENTS,
        "database_capability_available",
        "database capability is unavailable; the agent was expected to discover and install the database worker before executing the scenario",
    )
}

pub(super) fn score(evidence: &Evidence, names: &ScenarioNames) -> ObjectiveEvaluation {
    let expected_writer_sessions: BTreeSet<_> = names.writer_sessions.iter().cloned().collect();
    let expected_writer_names: BTreeSet<_> = (1..=EXPECTED_WRITERS)
        .map(|index| format!("writer-{index}"))
        .collect();

    let parallel_writers = evidence.writer_spawns.call_count == EXPECTED_WRITERS
        && evidence.writer_spawns.session_ids == expected_writer_sessions
        && (evidence.writer_spawns.max_parallel_calls == EXPECTED_WRITERS
            || evidence.writer_spawns.max_concurrent_sessions == EXPECTED_WRITERS)
        && expected_writer_sessions
            .iter()
            .all(|session| evidence.writer_spawns.sessions_in_tree.contains(session));
    let watch_before_writers = evidence
        .watch
        .first_writer_spawn
        .is_some_and(|first_spawn| {
            evidence
                .watch
                .trigger_catalog
                .is_some_and(|catalog| catalog < first_spawn)
                && evidence
                    .watch
                    .row_change_probe
                    .is_some_and(|probe| probe < first_spawn)
                && evidence
                    .watch
                    .aggregate_reaction
                    .is_some_and(|reaction| reaction < first_spawn)
                && evidence
                    .watch
                    .completion_wake
                    .is_some_and(|wake| wake < first_spawn)
        });

    let all_tables_exist = [
        names.orders.as_str(),
        names.writers.as_str(),
        names.totals.as_str(),
        names.report.as_str(),
    ]
    .iter()
    .all(|name| evidence.existing_tables.contains(*name));

    let orders_complete = order_summary_complete(evidence.order_summary.as_ref());
    let invalid_amounts = evidence
        .order_summary
        .as_ref()
        .and_then(|summary| summary.invalid_amounts);
    let writers_done = evidence.writers.len() == EXPECTED_WRITERS
        && evidence.writers.iter().all(|row| {
            row.writer
                .as_ref()
                .is_some_and(|writer| expected_writer_names.contains(writer))
                && row
                    .status
                    .as_deref()
                    .is_some_and(|status| status.eq_ignore_ascii_case("done"))
        });
    let totals_match = evidence.direct_totals.as_ref().is_some_and(|totals| {
        totals.len() == EXPECTED_WRITERS
            && totals.keys().cloned().collect::<BTreeSet<_>>() == expected_writer_names
            && totals
                .values()
                .all(|(count, _amount)| *count == ORDERS_PER_WRITER)
    }) && evidence.direct_totals == evidence.stored_totals;

    let report = single_report(&evidence.reports);
    let report_counts_match = report.is_some_and(|report| {
        report.events_received == Some(EXPECTED_ORDERS)
            && report.rows_written == Some(EXPECTED_ORDERS)
            && report.elapsed_ms.is_some_and(|elapsed| elapsed > 0)
    });
    let report_checks_pass = report.is_some_and(|report| {
        [
            report.totals_match,
            report.no_notification_loss,
            report.no_double_counting,
            report.mechanical_reaction,
            report.no_inline_waiting,
        ]
        .iter()
        .all(|check| *check == Some(true))
    });
    let watch_documented = watch_documented(report);
    let report_identity_matches = report.is_some_and(|report| {
        report.run_id.as_deref() == Some(names.run_label.as_str())
            && report.finalizer_session_id.as_deref() == Some(names.finalizer_session.as_str())
    });
    let aggregate_reaction_proven = evidence.aggregate_deliveries == EXPECTED_ORDERS as usize;
    let finalizer_completed = evidence.completion_wake_delivered
        && evidence.report_wake_before_finalizer
        && evidence.report_wake_delivered
        && evidence.finalizer_spawn_count == 1
        && evidence.finalizer_in_tree
        && evidence.finalizer_wrote_report
        && !evidence.root_wrote_report;

    let workload_passed = all_tables_exist && parallel_writers && orders_complete && writers_done;
    let aggregates_passed = totals_match && report_counts_match;
    let orchestration_passed = watch_before_writers
        && watch_documented
        && aggregate_reaction_proven
        && evidence.completion_wake_delivered;
    let finalization_passed = evidence.reports.len() == 1
        && report_identity_matches
        && report_checks_pass
        && finalizer_completed
        && evidence.active_run_triggers == 0;

    assessment::build_evaluation([
        PARALLEL_WRITES.full_or_zero(
            workload_passed,
            format!(
                "tables={all_tables_exist}, parallel_writers={parallel_writers}, \
                 orders_complete={orders_complete}, invalid_amounts={invalid_amounts:?}, \
                 writers_done={writers_done}"
            ),
        ),
        REACTIVE_AGGREGATES.full_or_zero(
            aggregates_passed,
            format!("totals_match={totals_match}, report_counts_match={report_counts_match}"),
        ),
        TRIGGER_ORCHESTRATION.full_or_zero(
            orchestration_passed,
            format!(
                "watch_before_writers={watch_before_writers}, \
                 watch_documented={watch_documented}, \
                 aggregate_deliveries={}, completion_wake={}",
                evidence.aggregate_deliveries, evidence.completion_wake_delivered
            ),
        ),
        FINALIZATION_CLEANUP.full_or_zero(
            finalization_passed,
            format!(
                "report_rows={}, report_identity={report_identity_matches}, \
                 report_checks={report_checks_pass}, completion_wake={}, \
                 report_watch_before_finalizer={}, report_wake={}, finalizer_spawns={}, \
                 finalizer_in_tree={}, finalizer_wrote_report={}, root_wrote_report={}, \
                 active_run_triggers={}",
                evidence.reports.len(),
                evidence.completion_wake_delivered,
                evidence.report_wake_before_finalizer,
                evidence.report_wake_delivered,
                evidence.finalizer_spawn_count,
                evidence.finalizer_in_tree,
                evidence.finalizer_wrote_report,
                evidence.root_wrote_report,
                evidence.active_run_triggers,
            ),
        ),
    ])
}

fn order_summary_complete(summary: Option<&super::evidence::OrderSummary>) -> bool {
    summary.is_some_and(|summary| {
        summary.rows_written == Some(EXPECTED_ORDERS)
            && summary.distinct_ids == Some(EXPECTED_ORDERS)
            && summary.writer_count == Some(EXPECTED_WRITERS as i64)
            && summary.invalid_amounts == Some(0)
            && summary.missing_created_at == Some(0)
    })
}

fn single_report(reports: &[FinalReport]) -> Option<&FinalReport> {
    reports.first().filter(|_| reports.len() == 1)
}

fn watch_documented(report: Option<&FinalReport>) -> bool {
    let Some(report) = report else {
        return false;
    };
    let contains = |value: Option<&str>, needle: &str| {
        value.is_some_and(|value| value.to_ascii_lowercase().contains(needle))
    };
    contains(report.watch_mechanism.as_deref(), "database")
        && report.reaction_function_id.as_deref() == Some("database::execute")
        && contains(report.reaction_event.as_deref(), "database::row-changed")
        && report
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty())
}
