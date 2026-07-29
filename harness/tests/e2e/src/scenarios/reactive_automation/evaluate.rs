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
                    .state_reaction
                    .is_some_and(|fallback| fallback < first_spawn)
        });

    let all_tables_exist = [
        names.orders.as_str(),
        names.writers.as_str(),
        names.totals.as_str(),
        names.report.as_str(),
    ]
    .iter()
    .all(|name| evidence.existing_tables.contains(*name));

    let orders_complete = evidence.order_summary.as_ref().is_some_and(|summary| {
        summary.rows_written == Some(EXPECTED_ORDERS)
            && summary.distinct_ids == Some(EXPECTED_ORDERS)
            && summary.writer_count == Some(EXPECTED_WRITERS as i64)
            && summary.missing_created_at == Some(0)
    });
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
            report.trigger_spawned_reactor,
            report.no_inline_waiting,
        ]
        .iter()
        .all(|check| *check == Some(true))
    });
    let fallback_documented = fallback_documented(report);
    let report_identity_matches = report.is_some_and(|report| {
        report.run_id.as_deref() == Some(names.run_label.as_str())
            && report.finalizer_session_id.as_deref() == Some(names.finalizer_session.as_str())
    });
    let reactor_triggered = report
        .and_then(|report| report.reactor_session_id.as_deref())
        .is_some_and(|session| {
            session.starts_with(&names.run_label)
                && evidence.writer_spawns.sessions_in_tree.contains(session)
                && evidence.reactor_spawned_by_trigger
        });
    let spawning_event_cited = report
        .and_then(|report| report.spawning_event.as_deref())
        .is_some_and(|event| !event.trim().is_empty());
    let finalizer_triggered = evidence
        .writer_spawns
        .sessions_in_tree
        .contains(&names.finalizer_session)
        && evidence.finalizer_spawned_by_trigger;

    let workload_passed = all_tables_exist && parallel_writers && orders_complete && writers_done;
    let aggregates_passed = totals_match && report_counts_match;
    let orchestration_passed =
        watch_before_writers && fallback_documented && reactor_triggered && spawning_event_cited;
    let finalization_passed = evidence.reports.len() == 1
        && report_identity_matches
        && report_checks_pass
        && finalizer_triggered
        && evidence.active_run_triggers == 0;

    ObjectiveEvaluation {
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
                format!("totals_match={totals_match}, report_counts_match={report_counts_match}"),
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
                     active_run_triggers={}",
                    evidence.reports.len(),
                    evidence.active_run_triggers
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
    }
}

fn single_report(reports: &[FinalReport]) -> Option<&FinalReport> {
    reports.first().filter(|_| reports.len() == 1)
}

fn fallback_documented(report: Option<&FinalReport>) -> bool {
    let Some(report) = report else {
        return false;
    };
    let contains = |value: Option<&str>, needle: &str| {
        value.is_some_and(|value| value.to_ascii_lowercase().contains(needle))
    };
    let state_fallback_used = contains(report.watch_mechanism.as_deref(), "state")
        || contains(report.spawning_event.as_deref(), "state")
        || contains(report.reactor_session_id.as_deref(), "fallback");
    if !state_fallback_used {
        return contains(report.watch_mechanism.as_deref(), "database");
    }
    contains(report.watch_mechanism.as_deref(), "state")
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
        let reactor_session = format!("{}-reactor-1", names.run_label);
        let expected_sessions: BTreeSet<_> = names.writer_sessions.iter().cloned().collect();
        let mut sessions_in_tree = expected_sessions.clone();
        sessions_in_tree.insert(reactor_session.clone());
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
                state_reaction: Some(2),
            },
            order_summary: Some(OrderSummary {
                rows_written: Some(15),
                distinct_ids: Some(15),
                writer_count: Some(3),
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
                watch_mechanism: Some("state".to_string()),
                fallback_reason: Some("database trigger unavailable".to_string()),
                events_received: Some(15),
                rows_written: Some(15),
                elapsed_ms: Some(1),
                totals_match: Some(true),
                no_notification_loss: Some(true),
                no_double_counting: Some(true),
                trigger_spawned_reactor: Some(true),
                no_inline_waiting: Some(true),
                reactor_session_id: Some(reactor_session),
                spawning_event: Some("state change".to_string()),
                finalizer_session_id: Some(names.finalizer_session.clone()),
            }],
            reactor_spawned_by_trigger: true,
            finalizer_spawned_by_trigger: true,
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
    fn database_watch_does_not_require_an_unused_fallback_reason() {
        let report = FinalReport {
            watch_mechanism: Some("database::row-changed".to_string()),
            reactor_session_id: Some("rctest-aB19-reactor".to_string()),
            spawning_event: Some("database::row-changed".to_string()),
            ..Default::default()
        };

        assert!(fallback_documented(Some(&report)));
    }

    #[test]
    fn unrecognized_watch_mechanism_is_rejected() {
        let report = FinalReport {
            watch_mechanism: Some("custom watcher".to_string()),
            reactor_session_id: Some("rctest-aB19-reactor".to_string()),
            spawning_event: Some("custom event".to_string()),
            ..Default::default()
        };

        assert!(!fallback_documented(Some(&report)));
    }

    #[test]
    fn state_fallback_requires_matching_mechanism_and_reason() {
        let mut report = FinalReport {
            watch_mechanism: Some("database::row-changed".to_string()),
            reactor_session_id: Some("rctest-aB19-reactor-fallback".to_string()),
            spawning_event: Some("state:order_signal".to_string()),
            ..Default::default()
        };

        assert!(!fallback_documented(Some(&report)));
        report.watch_mechanism = Some("state".to_string());
        assert!(!fallback_documented(Some(&report)));
        report.fallback_reason = Some("database trigger did not fire".to_string());
        assert!(fallback_documented(Some(&report)));
    }
}
