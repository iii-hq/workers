use std::collections::BTreeSet;

use super::evidence::{Evidence, FinalReport};
use super::names::ScenarioNames;
use super::{EXPECTED_ORDERS, EXPECTED_WRITERS, ORDERS_PER_WRITER};
use crate::scenarios::{common, ObjectiveEvaluation};

pub(super) fn missing_database() -> ObjectiveEvaluation {
    let reason = "database capability is unavailable; the agent was expected to discover and \
                  install the database worker before executing the scenario";
    ObjectiveEvaluation {
        hard_gates: vec![common::gate("database_capability_available", false, reason)],
        awards: vec![
            common::award("parallel_writes", 0, reason),
            common::award("reactive_aggregates", 0, reason),
            common::award("trigger_orchestration", 0, reason),
            common::award("finalization_cleanup", 0, reason),
        ],
    }
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

    ObjectiveEvaluation {
        hard_gates: vec![
            common::gate(
                "parallel_writers_completed",
                workload_passed,
                format!(
                    "tables={all_tables_exist}, parallel_writers={parallel_writers}, \
                     orders_complete={orders_complete}, invalid_amounts={invalid_amounts:?}, \
                     writers_done={writers_done}"
                ),
            ),
            common::gate(
                "reactive_totals_match",
                aggregates_passed,
                format!("totals_match={totals_match}, report_counts_match={report_counts_match}"),
            ),
            common::gate(
                "trigger_orchestration_proven",
                orchestration_passed,
                format!(
                    "watch_before_writers={watch_before_writers}, \
                     watch_documented={watch_documented}, \
                     aggregate_deliveries={}, completion_wake={}",
                    evidence.aggregate_deliveries, evidence.completion_wake_delivered
                ),
            ),
            common::gate(
                "finalizer_and_cleanup_completed",
                finalization_passed,
                format!(
                    "report_rows={}, report_identity={report_identity_matches}, \
                     report_checks={report_checks_pass}, completion_wake={}, \
                     report_watch_before_finalizer={}, report_wake={}, finalizer_spawns={}, \
                     finalizer_in_tree={}, finalizer_wrote_report={}, root_wrote_report={}, \
                     active_run_triggers={}",
                    evidence.reports.len(), evidence.completion_wake_delivered,
                    evidence.report_wake_before_finalizer, evidence.report_wake_delivered,
                    evidence.finalizer_spawn_count, evidence.finalizer_in_tree,
                    evidence.finalizer_wrote_report, evidence.root_wrote_report,
                    evidence.active_run_triggers,
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
                "awarded for bounded discovery, a mechanical aggregate reaction, and a barrier wake",
            ),
            common::award(
                "finalization_cleanup",
                if finalization_passed { 20 } else { 0 },
                "awarded when the barrier-woken root directly spawns one report-writing finalizer and cleans up",
            ),
        ],
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::scenarios::reactive_automation::evidence::{
        FinalReport, OrderSummary, WatchEvidence, WriterSpawnEvidence, WriterStatus,
    };

    #[test]
    fn complete_typed_evidence_earns_every_award() {
        let names = ScenarioNames::new("aB19-rest");
        let expected_sessions: BTreeSet<_> = names.writer_sessions.iter().cloned().collect();
        let mut sessions_in_tree = expected_sessions.clone();
        sessions_in_tree.insert(names.finalizer_session.clone());
        let totals = BTreeMap::from([
            ("writer-1".to_string(), (5, 15.0)),
            ("writer-2".to_string(), (5, 20.0)),
            ("writer-3".to_string(), (5, 25.0)),
        ]);
        let evidence = Evidence {
            existing_tables: BTreeSet::from([
                names.orders.clone(),
                names.writers.clone(),
                names.totals.clone(),
                names.report.clone(),
            ]),
            writer_spawns: WriterSpawnEvidence {
                call_count: 3,
                session_ids: expected_sessions,
                max_parallel_calls: 1,
                max_concurrent_sessions: 3,
                sessions_in_tree,
            },
            watch: WatchEvidence {
                first_writer_spawn: Some(3),
                trigger_catalog: Some(0),
                row_change_probe: Some(1),
                aggregate_reaction: Some(2),
                completion_wake: Some(2),
            },
            order_summary: Some(OrderSummary {
                rows_written: Some(15),
                distinct_ids: Some(15),
                writer_count: Some(3),
                invalid_amounts: Some(0),
                missing_created_at: Some(0),
            }),
            writers: (1..=3)
                .map(|index| WriterStatus {
                    writer: Some(format!("writer-{index}")),
                    status: Some("done".to_string()),
                })
                .collect(),
            direct_totals: Some(totals.clone()),
            stored_totals: Some(totals),
            reports: vec![FinalReport {
                run_id: Some(names.run_label.clone()),
                watch_mechanism: Some("database::row-changed".to_string()),
                fallback_reason: Some("no fallback; probe fired".to_string()),
                events_received: Some(15),
                rows_written: Some(15),
                elapsed_ms: Some(1),
                totals_match: Some(true),
                no_notification_loss: Some(true),
                no_double_counting: Some(true),
                mechanical_reaction: Some(true),
                no_inline_waiting: Some(true),
                reaction_function_id: Some("database::execute".to_string()),
                reaction_event: Some("database::row-changed insert".to_string()),
                finalizer_session_id: Some(names.finalizer_session.clone()),
            }],
            aggregate_deliveries: 15,
            completion_wake_delivered: true,
            report_wake_before_finalizer: true,
            report_wake_delivered: true,
            finalizer_spawn_count: 1,
            finalizer_in_tree: true,
            finalizer_wrote_report: true,
            root_wrote_report: false,
            active_run_triggers: 0,
        };

        let evaluation = score(&evidence, &names);

        assert!(evaluation.hard_gates.iter().all(|gate| gate.passed));
        assert_eq!(
            evaluation
                .awards
                .iter()
                .map(|award| u16::from(award.awarded))
                .sum::<u16>(),
            100
        );
    }

    #[test]
    fn numeric_order_amounts_satisfy_the_order_summary() {
        let summary = OrderSummary {
            rows_written: Some(15),
            distinct_ids: Some(15),
            writer_count: Some(3),
            invalid_amounts: Some(0),
            missing_created_at: Some(0),
        };

        assert!(order_summary_complete(Some(&summary)));
    }

    #[test]
    fn invalid_order_amounts_fail_the_order_summary() {
        let summary = OrderSummary {
            rows_written: Some(15),
            distinct_ids: Some(15),
            writer_count: Some(3),
            invalid_amounts: Some(1),
            missing_created_at: Some(0),
        };

        assert!(!order_summary_complete(Some(&summary)));
    }

    #[test]
    fn missing_database_is_a_behavioral_hard_gate() {
        let evaluation = missing_database();

        assert_eq!(evaluation.hard_gates.len(), 1);
        assert_eq!(evaluation.hard_gates[0].id, "database_capability_available");
        assert!(!evaluation.hard_gates[0].passed);
        assert_eq!(
            evaluation
                .awards
                .iter()
                .map(|award| u16::from(award.awarded))
                .sum::<u16>(),
            0
        );
    }

    #[test]
    fn database_watch_requires_the_mechanical_reaction_evidence() {
        let report = FinalReport {
            watch_mechanism: Some("database::row-changed".to_string()),
            fallback_reason: Some("no fallback; probe fired".to_string()),
            reaction_function_id: Some("database::execute".to_string()),
            reaction_event: Some("database::row-changed insert".to_string()),
            ..Default::default()
        };

        assert!(watch_documented(Some(&report)));
    }

    #[test]
    fn unrecognized_watch_mechanism_is_rejected() {
        let report = FinalReport {
            watch_mechanism: Some("custom watcher".to_string()),
            fallback_reason: Some("none".to_string()),
            reaction_function_id: Some("database::execute".to_string()),
            reaction_event: Some("custom event".to_string()),
            ..Default::default()
        };

        assert!(!watch_documented(Some(&report)));
    }

    #[test]
    fn database_watch_rejects_missing_fallback_note() {
        let mut report = FinalReport {
            watch_mechanism: Some("database::row-changed".to_string()),
            reaction_function_id: Some("database::execute".to_string()),
            reaction_event: Some("database::row-changed insert".to_string()),
            ..Default::default()
        };

        assert!(!watch_documented(Some(&report)));
        report.fallback_reason = Some("no fallback; probe fired".to_string());
        assert!(watch_documented(Some(&report)));
    }
}
