//! `sentinel::reports::ingest` over a real store (see support/db.rs): E2E
//! report observations become groups, occurrences and frozen evidence
//! through the same pipeline as a span, and stay readable after the test
//! stack is gone.

#[path = "support/db.rs"]
mod db;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use db::{plain, TestDb};
use sentinel::adapters::report::CONTRACT_QUESTION;
use sentinel::ingest::{ring::PhantomRing, CheckoutVersions, Ingest, Telemetry};
use sentinel::registry::{EngineRegistry, FunctionEntry, Registry, WorkerEntry};
use sentinel::service::TraceAvailability;
use sentinel::store::Db;
use sentinel::{
    Counters, ErrorSourceV1, EvidenceGetRequestV1, ExpectedErrorV1, GroupGetRequestV1,
    GroupStatusV1, IgnoreRequestV1, IgnoreRuleV1, ReportClassificationV1, ReportExcerptV1,
    ReportObservationV1, ReportOutcomeV1, ReportSideV1, ReportsIngestRequestV1,
    ReportsIngestResponseV1, ResolveRequestV1, SentinelError, Service, Store, TraceSummary,
    TriageKindV1, TriageSourceV1, WorkerConfig,
};
use serde_json::{json, Value};

struct NoTelemetry;

#[async_trait]
impl Telemetry for NoTelemetry {
    async fn traces(&self, _trace_ids: &[String]) -> Result<Vec<TraceSummary>, SentinelError> {
        Ok(Vec::new())
    }
    async fn tree(&self, _trace_id: &str) -> Result<Vec<Value>, SentinelError> {
        Ok(Vec::new())
    }
    async fn logs(&self, _trace_id: &str) -> Result<Vec<Value>, SentinelError> {
        Ok(Vec::new())
    }
}

struct NoRegistry;

#[async_trait]
impl EngineRegistry for NoRegistry {
    async fn list_functions(&self) -> Result<Vec<FunctionEntry>, SentinelError> {
        Ok(Vec::new())
    }
    async fn list_workers(&self) -> Result<Vec<WorkerEntry>, SentinelError> {
        Ok(Vec::new())
    }
}

struct NoCheckouts;

#[async_trait]
impl CheckoutVersions for NoCheckouts {
    async fn version_for(&self, _worker: &str) -> Option<String> {
        None
    }
}

struct NoTrace;

#[async_trait]
impl TraceAvailability for NoTrace {
    async fn trace_exists(&self, _trace_id: &str) -> bool {
        false
    }
}

struct Harness {
    ingest: Ingest<TestDb, NoRegistry>,
    store: Arc<Store<TestDb>>,
    service: Service<TestDb>,
    config: WorkerConfig,
}

async fn harness() -> Harness {
    let store = Arc::new(Store::new(db::test_db().await));
    store.migrate().await.expect("migrate");
    let ingest = Ingest::new(
        store.clone(),
        Arc::new(NoTelemetry),
        Arc::new(Registry::new(NoRegistry, Duration::from_secs(60))),
        Arc::new(NoCheckouts),
        Arc::new(Counters::default()),
        Arc::new(PhantomRing::default()),
    );
    Harness {
        ingest,
        service: Service::new(store.clone(), Arc::new(NoTrace)),
        store,
        config: WorkerConfig::default(),
    }
}

impl Harness {
    async fn send(&self, observations: Vec<ReportObservationV1>) -> ReportsIngestResponseV1 {
        let request = ReportsIngestRequestV1 {
            report_id: "pkg_1".into(),
            observations,
            ..ReportsIngestRequestV1::default()
        };
        self.ingest
            .ingest_report(&request, &self.config)
            .await
            .expect("the report records")
            .0
    }

    async fn total(&self, sql: &str) -> i64 {
        let rows = self.store.db().query(sql, vec![]).await.expect("query");
        plain(rows[0]["total"].clone()).as_i64().expect("a count")
    }

    async fn occurrences_of(&self, group_id: &str) -> u64 {
        self.service
            .get(GroupGetRequestV1::new(group_id))
            .await
            .expect("the group reads")
            .group
            .occurrence_count
    }
}

fn observation(side: ReportSideV1) -> ReportObservationV1 {
    ReportObservationV1 {
        execution_id: "exe_1".into(),
        run_id: "run_1".into(),
        attempt: 1,
        scenario_id: "checkout".into(),
        phase: "act".into(),
        side,
        revision: match side {
            ReportSideV1::Baseline => "git:base000".into(),
            ReportSideV1::Candidate => "git:cand111".into(),
        },
        artifact: "logs/orders.log".into(),
        occurrence_index: 0,
        at_ms: 1_789_000_000_000,
        service_name: "orders".into(),
        function_id: Some("orders::place".into()),
        exception_type: Some("Panic".into()),
        message: "index out of bounds: the len is 3 but the index is 7".into(),
        stacktrace: Some("at orders::place (src/place.rs:42)".into()),
        trace_id: None,
        span_id: None,
        session_id: None,
        expected_errors: Some(Vec::new()),
        excerpts: vec![ReportExcerptV1 {
            name: "orders.log".into(),
            content: "thread 'main' panicked at src/place.rs:42".into(),
        }],
    }
}

fn expected(mut seen: ReportObservationV1) -> ReportObservationV1 {
    seen.expected_errors = Some(vec![ExpectedErrorV1 {
        exception_type: Some("Panic".into()),
        ..ExpectedErrorV1::default()
    }]);
    seen
}

#[tokio::test]
async fn baseline_and_candidate_meet_in_one_group_and_a_reimport_counts_nothing() {
    let h = harness().await;
    let first = h
        .send(vec![
            observation(ReportSideV1::Baseline),
            observation(ReportSideV1::Candidate),
        ])
        .await;
    assert_eq!((first.recorded, first.deduped, first.skipped), (2, 0, 0));
    assert_eq!(first.results[0].outcome, ReportOutcomeV1::Created);
    assert_eq!(first.results[1].outcome, ReportOutcomeV1::Counted);
    assert!(first
        .results
        .iter()
        .all(|result| result.classification == ReportClassificationV1::Unexpected));
    let group_id = first.results[0].group_id.clone().expect("a group");
    assert_eq!(
        first.results[1].group_id.as_deref(),
        Some(group_id.as_str())
    );
    assert_eq!(h.occurrences_of(&group_id).await, 2);

    let again = h
        .send(vec![
            observation(ReportSideV1::Baseline),
            observation(ReportSideV1::Candidate),
        ])
        .await;
    assert_eq!((again.recorded, again.deduped), (0, 2));
    assert!(again
        .results
        .iter()
        .all(|result| result.outcome == ReportOutcomeV1::Deduped));
    assert_eq!(
        again.results[1].occurrence_id, first.results[1].occurrence_id,
        "a re-import answers with the occurrence already recorded"
    );
    assert_eq!(
        h.occurrences_of(&group_id).await,
        2,
        "the count did not move"
    );
    assert_eq!(
        h.total("SELECT COUNT(*) AS total FROM sentinel_groups")
            .await,
        1
    );

    // A retry of the same scenario is a new occurrence.
    let mut retry = observation(ReportSideV1::Candidate);
    retry.attempt = 2;
    let retried = h.send(vec![retry]).await;
    assert_eq!(retried.results[0].outcome, ReportOutcomeV1::Counted);
    assert_eq!(h.occurrences_of(&group_id).await, 3);

    let detail = h
        .service
        .get(GroupGetRequestV1::new(&group_id))
        .await
        .unwrap();
    assert_eq!(detail.group.source, ErrorSourceV1::Report);
    assert_eq!(detail.group.service_name, "orders");
    assert_eq!(detail.group.namespace, "e2e");
    assert!(detail.group.relevant, "an unexpected failure is work");
}

#[tokio::test]
async fn the_frozen_evidence_keeps_its_provenance_after_the_stack_is_gone() {
    let h = harness().await;
    let response = h
        .send(vec![
            observation(ReportSideV1::Baseline),
            observation(ReportSideV1::Candidate),
        ])
        .await;
    let occurrence_id = response.results[1]
        .occurrence_id
        .clone()
        .expect("an occurrence");
    let evidence = h
        .service
        .evidence(EvidenceGetRequestV1 {
            occurrence_id,
            ..EvidenceGetRequestV1::default()
        })
        .await
        .expect("the evidence reads");
    assert!(!evidence.pruned);
    let bundle = evidence.evidence.expect("a bundle");
    assert!(bundle.settled);
    assert_eq!(bundle.worker.version.as_deref(), Some("git:cand111"));
    let report = bundle.report.expect("the report provenance");
    assert_eq!(report.report_id, "pkg_1");
    assert_eq!(report.side, ReportSideV1::Candidate);
    assert_eq!(report.revision, "git:cand111");
    assert_eq!(report.artifact, "logs/orders.log");
    assert_eq!(report.classification, ReportClassificationV1::Unexpected);
    assert_eq!(
        report.stacktrace.as_deref(),
        Some("at orders::place (src/place.rs:42)")
    );
    assert_eq!(report.excerpts.len(), 1);
}

#[tokio::test]
async fn a_deliberate_failure_is_evidence_in_its_own_group_and_never_work() {
    let h = harness().await;
    let real = h.send(vec![observation(ReportSideV1::Candidate)]).await;
    let real_group = real.results[0].group_id.clone().unwrap();

    let mut provoked = expected(observation(ReportSideV1::Candidate));
    provoked.scenario_id = "checkout-fault-injection".into();
    let response = h.send(vec![provoked.clone()]).await;
    let result = &response.results[0];
    assert_eq!(result.classification, ReportClassificationV1::Expected);
    assert_eq!(result.outcome, ReportOutcomeV1::Created);
    let provoked_group = result.group_id.clone().unwrap();
    assert_ne!(
        provoked_group, real_group,
        "the same text provoked on purpose never lands on the real defect's group"
    );
    let triage = result.triage.clone().expect("labelled by the contract");
    assert_eq!(triage.kind, TriageKindV1::TestTraffic);
    assert_eq!(triage.source, TriageSourceV1::Rule);
    assert_eq!(triage.question.as_deref(), Some(CONTRACT_QUESTION));
    assert!(triage.criterion.is_some() && triage.evidence.is_some());

    let provoked_detail = h
        .service
        .get(GroupGetRequestV1::new(&provoked_group))
        .await
        .unwrap();
    assert!(!provoked_detail.group.relevant, "test traffic is not work");

    // The real defect's group is untouched by any of it.
    let real_detail = h
        .service
        .get(GroupGetRequestV1::new(&real_group))
        .await
        .unwrap();
    assert_eq!(real_detail.group.occurrence_count, 1);
    assert!(real_detail.group.triage.is_none());
    assert_eq!(real_detail.group.status, GroupStatusV1::New);

    // A person resolved the provoked group: the next provoked run keeps it so.
    h.service
        .resolve(ResolveRequestV1 {
            group_id: provoked_group.clone(),
            ..ResolveRequestV1::default()
        })
        .await
        .unwrap();
    provoked.execution_id = "exe_2".into();
    let later = h.send(vec![provoked]).await;
    assert_eq!(later.results[0].outcome, ReportOutcomeV1::Counted);
    assert_eq!(later.results[0].group_status, Some(GroupStatusV1::Resolved));
}

#[tokio::test]
async fn a_different_error_than_the_contract_expected_is_work() {
    let h = harness().await;
    let mut seen = observation(ReportSideV1::Candidate);
    seen.expected_errors = Some(vec![ExpectedErrorV1 {
        exception_type: Some("Timeout".into()),
        ..ExpectedErrorV1::default()
    }]);
    let response = h.send(vec![seen]).await;
    assert_eq!(
        response.results[0].classification,
        ReportClassificationV1::ExpectedMismatch
    );
    assert!(response.results[0].triage.is_none());
    let detail = h
        .service
        .get(GroupGetRequestV1::new(
            response.results[0].group_id.as_deref().unwrap(),
        ))
        .await
        .unwrap();
    assert!(detail.group.relevant);
}

#[tokio::test]
async fn human_decisions_hold_and_an_unplaced_error_regresses_nothing() {
    let h = harness().await;
    let first = h.send(vec![observation(ReportSideV1::Baseline)]).await;
    let group_id = first.results[0].group_id.clone().unwrap();
    h.service
        .resolve(ResolveRequestV1 {
            group_id: group_id.clone(),
            ..ResolveRequestV1::default()
        })
        .await
        .unwrap();

    // The contract does not say: counted, but the resolve stands.
    let mut unplaced = observation(ReportSideV1::Candidate);
    unplaced.expected_errors = None;
    let response = h.send(vec![unplaced]).await;
    assert_eq!(
        response.results[0].classification,
        ReportClassificationV1::InsufficientEvidence
    );
    assert_eq!(
        response.results[0].group_id.as_deref(),
        Some(group_id.as_str())
    );
    assert_eq!(
        response.results[0].group_status,
        Some(GroupStatusV1::Resolved)
    );
    assert_eq!(h.occurrences_of(&group_id).await, 2);

    // Ignored forever stays ignored while the counter keeps running.
    h.service
        .reopen(sentinel::GroupActionRequestV1 {
            group_id: group_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    h.service
        .ignore(IgnoreRequestV1 {
            group_id: group_id.clone(),
            rule: IgnoreRuleV1::Forever,
            _caller_worker_id: None,
        })
        .await
        .unwrap();
    let mut later = observation(ReportSideV1::Candidate);
    later.execution_id = "exe_2".into();
    let response = h.send(vec![later]).await;
    assert_eq!(
        response.results[0].group_status,
        Some(GroupStatusV1::Ignored)
    );
    assert_eq!(h.occurrences_of(&group_id).await, 3);
}

#[tokio::test]
async fn a_score_drop_without_an_error_invents_no_runtime_error() {
    let h = harness().await;
    let mut quiet = observation(ReportSideV1::Candidate);
    quiet.exception_type = None;
    quiet.message = String::new();
    quiet.stacktrace = None;
    let response = h.send(vec![quiet]).await;
    assert_eq!((response.recorded, response.skipped), (0, 1));
    assert_eq!(response.results[0].outcome, ReportOutcomeV1::Skipped);
    assert!(response.results[0].group_id.is_none());
    assert_eq!(
        h.total("SELECT COUNT(*) AS total FROM sentinel_groups")
            .await,
        0
    );
    assert_eq!(
        h.total("SELECT COUNT(*) AS total FROM sentinel_occurrences")
            .await,
        0
    );
}

#[tokio::test]
async fn every_field_of_the_envelope_passes_the_redactor() {
    let h = harness().await;
    let secret = "Bearer abc.def-ghi_jkl";
    let mut leaky = observation(ReportSideV1::Candidate);
    leaky.message = format!("upstream refused {secret}");
    leaky.artifact = format!("logs/{secret}.log");
    leaky.stacktrace = Some(format!("Authorization: {secret}"));
    leaky.excerpts = vec![ReportExcerptV1 {
        name: format!("headers {secret}"),
        content: format!("Authorization: {secret}"),
    }];
    let response = h.send(vec![leaky]).await;
    let result = &response.results[0];
    let evidence = h
        .service
        .evidence(EvidenceGetRequestV1 {
            occurrence_id: result.occurrence_id.clone().unwrap(),
            ..EvidenceGetRequestV1::default()
        })
        .await
        .unwrap();
    let frozen = json!(evidence.evidence).to_string();
    assert!(!frozen.contains("abc.def"), "{frozen}");
    let detail = h
        .service
        .get(GroupGetRequestV1::new(result.group_id.as_deref().unwrap()))
        .await
        .unwrap();
    assert!(!detail.message_sample.contains("abc.def"));
    assert!(!detail.group.title.contains("abc.def"));
}

#[tokio::test]
async fn an_observation_that_cannot_name_its_occurrence_is_refused() {
    let h = harness().await;
    let mut nameless = observation(ReportSideV1::Candidate);
    nameless.execution_id = " ".into();
    let request = ReportsIngestRequestV1 {
        report_id: "pkg_1".into(),
        observations: vec![nameless],
        ..ReportsIngestRequestV1::default()
    };
    let error = h
        .ingest
        .ingest_report(&request, &h.config)
        .await
        .expect_err("refused");
    assert_eq!(error.code(), "invalid_request");
    assert_eq!(
        h.total("SELECT COUNT(*) AS total FROM sentinel_groups")
            .await,
        0
    );
}

#[tokio::test]
async fn a_reimport_that_expects_the_failure_never_labels_the_group_it_was_counted_in() {
    let h = harness().await;
    // First sent with no contract, as Eval sends an E2E failure today.
    let mut seen = observation(ReportSideV1::Candidate);
    seen.expected_errors = None;
    let first = h.send(vec![seen.clone()]).await;
    let group_id = first.results[0].group_id.clone().unwrap();
    let again = h.send(vec![expected(seen)]).await;
    assert_eq!(again.results[0].outcome, ReportOutcomeV1::Deduped);
    let detail = h
        .service
        .get(GroupGetRequestV1::new(&group_id))
        .await
        .unwrap();
    assert!(detail.group.triage.is_none(), "{:?}", detail.group.triage);
}

#[tokio::test]
async fn a_secret_the_message_cap_would_cut_is_still_redacted() {
    let h = harness().await;
    // The cap (64 KiB, with its 3-byte marker) falls right before the `@`
    // the URL-credential rule needs: capping first would keep the password.
    let secret = "postgres://admin:hunter2";
    let max = h.config.evidence.message_max_bytes as usize;
    let mut leaky = observation(ReportSideV1::Candidate);
    leaky.message = format!("{}{secret}@db:5432/app", "x".repeat(max - 3 - secret.len()));
    let response = h.send(vec![leaky]).await;
    let result = &response.results[0];
    let evidence = h
        .service
        .evidence(EvidenceGetRequestV1 {
            occurrence_id: result.occurrence_id.clone().unwrap(),
            ..EvidenceGetRequestV1::default()
        })
        .await
        .unwrap();
    assert!(!json!(evidence.evidence).to_string().contains("hunter2"));
    let detail = h
        .service
        .get(GroupGetRequestV1::new(result.group_id.as_deref().unwrap()))
        .await
        .unwrap();
    assert!(!detail.message_sample.contains("hunter2"));
}
