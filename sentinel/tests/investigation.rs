//! Every state an investigation can reach, over a real store and a harness
//! that answers what the test tells it to.
//!
//! The paths worth pinning are the ones a live stack only shows at the wrong
//! moment: a doorbell that beats the send's own answer back, a person
//! resolving a group while the agent is still typing, and a first pass that
//! ends having said nothing.

#[path = "support/sqlite.rs"]
mod sqlite;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sentinel::ingest::CheckoutVersions;
use sentinel::investigation::record::Caller;
use sentinel::investigation::{
    Announcement, Harness, Investigations, SendOutcome, TurnMetrics, TurnStatus,
};
use sentinel::service::TraceAvailability;
use sentinel::store::Db;
use sentinel::{
    ConfidenceV1, DiagnosisCategoryV1, DiagnosisRecordRequestV1, DiagnosisV1, ErrorSourceV1,
    GroupStatusV1, InvestigateRequestV1, InvestigationModeV1, InvestigationStatusV1,
    OccurrenceWrite, RecordOutcome, RepositoryConfigV1, ResolveRequestV1, RootCauseV1, Service,
    Store, TurnCompletedEventV1, WorkerConfig,
};
use serde_json::{json, Value};
use sqlite::SqliteDb;
use tokio::sync::RwLock;

const NOW: i64 = 1_789_000_000_000;
const MODEL: &str = "anthropic::claude-sonnet-5";

// ── the fake harness ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Grant {
        session_id: String,
        root: String,
    },
    Send {
        idempotency_key: String,
        body: String,
    },
    Stop {
        session_id: String,
        turn_id: String,
    },
    Ensure {
        session_id: String,
        title: String,
    },
    Append {
        entry_id: String,
        text: String,
        origin: Value,
    },
}

#[derive(Default)]
struct FakeHarnessState {
    calls: Vec<Call>,
    /// What `harness::status` answers, once the first pass is running.
    status: Option<TurnStatus>,
    /// Next turn id handed out by `send`.
    next_turn: u64,
    send_fails: bool,
    append_fails: bool,
    /// Rows observed inside the `send` call, to prove the store was written
    /// before the harness heard anything.
    seen_running: Option<usize>,
}

struct FakeHarness {
    state: Arc<Mutex<FakeHarnessState>>,
    /// Read by `send` so the test can look at the store mid-call.
    store: Arc<Store<SqliteDb>>,
}

impl FakeHarness {
    fn new(store: Arc<Store<SqliteDb>>) -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(FakeHarnessState::default())),
            store,
        })
    }

    fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    fn set_status(&self, status: &str, error: Option<&str>, turn_id: Option<&str>) {
        self.state.lock().unwrap().status = Some(TurnStatus {
            turn_id: turn_id.map(str::to_string),
            status: status.into(),
            expects_wake: false,
            error: error.map(str::to_string),
        });
    }

    fn forget_session(&self) {
        self.state.lock().unwrap().status = None;
    }

    fn fail_sends(&self) {
        self.state.lock().unwrap().send_fails = true;
    }

    fn fail_appends(&self) {
        self.state.lock().unwrap().append_fails = true;
    }

    fn running_rows_seen_during_send(&self) -> Option<usize> {
        self.state.lock().unwrap().seen_running
    }
}

#[async_trait]
impl Harness for FakeHarness {
    async fn grant_filesystem(&self, session_id: &str, root: &str) {
        self.state.lock().unwrap().calls.push(Call::Grant {
            session_id: session_id.into(),
            root: root.into(),
        });
    }

    async fn send(&self, request: Value) -> Result<SendOutcome, SentinelErrorAlias> {
        // Read the store from inside the call: this is the race the design
        // claims to have closed.
        let running = self
            .store
            .running_investigations(10)
            .await
            .expect("read the running investigations")
            .len();
        let mut state = self.state.lock().unwrap();
        state.seen_running = Some(running);
        state.calls.push(Call::Send {
            idempotency_key: request["idempotency_key"].as_str().unwrap_or("").into(),
            body: request["message"].as_str().unwrap_or("").into(),
        });
        if state.send_fails {
            return Err(sentinel::SentinelError::dependency("no agent worker"));
        }
        state.next_turn += 1;
        let turn_id = format!("turn-{}", state.next_turn);
        state.status = Some(TurnStatus {
            turn_id: Some(turn_id.clone()),
            status: "running".into(),
            expects_wake: false,
            error: None,
        });
        Ok(SendOutcome {
            session_id: request["session_id"].as_str().unwrap_or("").into(),
            turn_id,
        })
    }

    async fn status(&self, _session_id: &str) -> Result<Option<TurnStatus>, SentinelErrorAlias> {
        Ok(self.state.lock().unwrap().status.clone())
    }

    async fn stop(&self, session_id: &str, turn_id: &str) -> Result<(), SentinelErrorAlias> {
        self.state.lock().unwrap().calls.push(Call::Stop {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
        });
        Ok(())
    }

    async fn metrics(&self, _root_session_id: &str) -> TurnMetrics {
        TurnMetrics {
            turns: Some(3),
            duration_ms: Some(12_000),
            cost_usd: Some(0.42),
        }
    }

    async fn ensure_session(
        &self,
        session_id: &str,
        title: &str,
        _metadata: Value,
    ) -> Result<(), SentinelErrorAlias> {
        self.state.lock().unwrap().calls.push(Call::Ensure {
            session_id: session_id.into(),
            title: title.into(),
        });
        Ok(())
    }

    async fn append_message(
        &self,
        _session_id: &str,
        entry_id: &str,
        text: &str,
        origin: Value,
    ) -> Result<(), SentinelErrorAlias> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(Call::Append {
            entry_id: entry_id.into(),
            text: text.into(),
            origin,
        });
        if state.append_fails {
            return Err(sentinel::SentinelError::dependency(
                "session-manager is down",
            ));
        }
        Ok(())
    }
}

type SentinelErrorAlias = sentinel::SentinelError;

struct MappedCheckout;

#[async_trait]
impl CheckoutVersions for MappedCheckout {
    async fn version_for(&self, _worker: &str) -> Option<String> {
        Some("git:abc1234".into())
    }
}

struct NoTrace;

#[async_trait]
impl TraceAvailability for NoTrace {
    async fn trace_exists(&self, _trace_id: &str) -> bool {
        false
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────

struct Fixture {
    store: Arc<Store<SqliteDb>>,
    harness: Arc<FakeHarness>,
    investigations: Investigations<SqliteDb>,
    service: Service<SqliteDb>,
    group_id: String,
}

fn config(model: &str, repositories: Vec<RepositoryConfigV1>) -> sentinel::ConfigCell {
    let mut config = WorkerConfig::default();
    config.investigation.model = model.into();
    config.repositories = repositories;
    Arc::new(RwLock::new(Arc::new(config)))
}

async fn fixture(model: &str) -> Fixture {
    fixture_with(model, vec![]).await
}

async fn fixture_with(model: &str, repositories: Vec<RepositoryConfigV1>) -> Fixture {
    let store = Arc::new(Store::new(SqliteDb::in_memory()));
    store.migrate().await.expect("migrate");
    let group_id = seed(&store).await;
    let harness = FakeHarness::new(store.clone());
    let investigations = Investigations::new(
        store.clone(),
        harness.clone(),
        Arc::new(MappedCheckout),
        config(model, repositories),
    );
    let service = Service::new(store.clone(), Arc::new(NoTrace));
    Fixture {
        store,
        harness,
        investigations,
        service,
        group_id,
    }
}

/// One group with one occurrence that still has its evidence.
async fn seed(store: &Store<SqliteDb>) -> String {
    let write = OccurrenceWrite {
        fingerprint: "fp-boom".into(),
        source: ErrorSourceV1::Trace,
        dedupe_key: "trace:t1:s1".into(),
        at_ms: NOW,
        namespace: "default".into(),
        service_name: "compose".into(),
        function_id: Some("compose::operation".into()),
        exception_type: Some("UnknownOperation".into()),
        title: "UnknownOperation: unknown compose operation".into(),
        message: "unknown compose operation `<str>`".into(),
        trace_id: Some("t1".into()),
        span_id: Some("s1".into()),
        session_id: Some("s_user".into()),
        turn_id: None,
        worker_version: Some("0.24.0".into()),
        evidence: Some(evidence()),
        namespace_ambiguous: false,
        pending_occurrence_id: None,
    };
    match store.record_occurrence(&write).await.expect("record") {
        RecordOutcome::Created { group_id } => group_id,
        other => panic!("expected a new group, got {other:?}"),
    }
}

fn evidence() -> String {
    json!({
        "version": 1,
        "captured_at_ms": NOW,
        "settled": true,
        "trace_id": "t1",
        "origin_span_id": "s1",
        "propagated_through": ["s0"],
        "trace_tags": { "iii.session.id": "s_user" },
        "spans": [
            {
                "span_id": "s0",
                "name": "execute harness::turn",
                "service_name": "harness",
                "start_time_unix_nano": 1,
                "end_time_unix_nano": 2,
                "status": "error",
                "status_description": "child failed",
                "attributes": {},
                "events": [],
                "depth": 0
            },
            {
                "span_id": "s1",
                "parent_span_id": "s0",
                "name": "execute compose::operation",
                "service_name": "compose",
                "function_id": "compose::operation",
                "start_time_unix_nano": 1,
                "end_time_unix_nano": 2,
                "status": "error",
                "status_description": "unknown compose operation `<str>`",
                "attributes": { "function_id": "compose::operation" },
                "events": [{
                    "name": "exception",
                    "timestamp_unix_nano": 2,
                    "attributes": { "exception.message": "unknown compose operation `up`" }
                }],
                "depth": 1
            }
        ],
        "logs": [{
            "timestamp_unix_nano": 2,
            "severity_text": "ERROR",
            "body": "compose::operation failed",
            "attributes": {}
        }],
        "worker": { "service_name": "compose", "version": "0.24.0" },
        "truncated": { "spans": 0, "logs": 0, "attributes": 0 }
    })
    .to_string()
}

fn a_diagnosis(summary: &str) -> DiagnosisV1 {
    DiagnosisV1 {
        summary: summary.into(),
        category: DiagnosisCategoryV1::Bug,
        confidence: ConfidenceV1::Medium,
        root_cause: RootCauseV1 {
            description: "the operation table has no `up`".into(),
            evidence: vec![],
        },
        proposed_fix: None,
        reproduction: None,
        missing_evidence: vec![],
        version_note: None,
        related_groups: vec![],
    }
}

async fn status_of(fixture: &Fixture) -> GroupStatusV1 {
    fixture
        .store
        .group_by_id(&fixture.group_id)
        .await
        .expect("read")
        .expect("the group exists")
        .state
        .status
}

// ── opening one ──────────────────────────────────────────────────────────

#[tokio::test]
async fn the_investigation_is_on_record_before_the_harness_hears_anything() {
    let fixture = fixture(MODEL).await;
    let outcome = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect("the investigation opens");

    assert!(!outcome.value.existing);
    assert!(outcome.value.session_id.starts_with("sentinel-inv-"));
    assert_eq!(
        fixture.harness.running_rows_seen_during_send(),
        Some(1),
        "a doorbell or a diagnosis can arrive before send returns, so the row must already exist"
    );
    assert_eq!(status_of(&fixture).await, GroupStatusV1::Investigating);
    assert_eq!(
        outcome.value.first_pass_turn_id.as_deref(),
        Some("turn-1"),
        "the turn the doorbell will name is remembered"
    );
}

#[tokio::test]
async fn the_first_pass_carries_the_evidence_and_the_group_it_must_record_against() {
    let fixture = fixture_with(
        MODEL,
        vec![RepositoryConfigV1 {
            id: "workers".into(),
            path: "/home/dev/workers".into(),
            workers: vec!["compose".into()],
        }],
    )
    .await;
    fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect("the investigation opens");

    let calls = fixture.harness.calls();
    assert!(
        matches!(&calls[0], Call::Grant { root, .. } if root == "/home/dev/workers"),
        "the checkout is granted before the first read is attempted: {calls:?}"
    );
    let Call::Send {
        idempotency_key,
        body,
    } = &calls[1]
    else {
        panic!("expected a send, got {calls:?}");
    };
    assert!(idempotency_key.ends_with(":analysis"));
    assert!(body.contains(&fixture.group_id), "{body}");
    assert!(body.contains("unknown compose operation"), "{body}");
    assert!(body.contains("execute compose::operation"), "{body}");
    assert!(body.contains("/home/dev/workers"), "{body}");
    assert!(body.contains("sentinel::diagnosis::record"), "{body}");
    assert!(
        body.contains("git:abc1234"),
        "the checkout read is named: {body}"
    );
}

#[tokio::test]
async fn a_second_investigate_hands_back_the_one_already_running() {
    let fixture = fixture(MODEL).await;
    let first = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect("the first opens")
        .value;
    let second = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect("the second is answered")
        .value;

    assert!(second.existing);
    assert_eq!(second.investigation_id, first.investigation_id);
    assert_eq!(second.session_id, first.session_id);
    assert_eq!(
        fixture
            .harness
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Send { .. }))
            .count(),
        1,
        "the second investigate must not start a second turn"
    );
}

#[tokio::test]
async fn a_send_that_fails_leaves_a_failed_investigation_not_a_running_one() {
    let fixture = fixture(MODEL).await;
    fixture.harness.fail_sends();
    let error = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect_err("a first pass that never started is a failure");
    assert_eq!(error.code(), "harness_unavailable");

    let (rows, _) = fixture
        .store
        .list_investigations(Some(&fixture.group_id), &[], 0, 10)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, InvestigationStatusV1::Failed);
    assert!(rows[0].error.is_some());
    assert_eq!(
        status_of(&fixture).await,
        GroupStatusV1::New,
        "the group never entered investigating, so it has nothing to come back from"
    );
}

#[tokio::test]
async fn without_a_model_nothing_is_opened() {
    let fixture = fixture("").await;
    let error = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect_err("a model is required");
    assert_eq!(error.code(), "no_model");
    assert!(fixture.harness.calls().is_empty());
}

#[tokio::test]
async fn without_evidence_nothing_is_opened() {
    let fixture = fixture(MODEL).await;
    fixture
        .store
        .db()
        .execute(
            "UPDATE sentinel_occurrences SET evidence = NULL, evidence_bytes = 0",
            vec![],
        )
        .await
        .expect("prune the evidence");
    let error = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect_err("there is nothing to investigate from");
    assert_eq!(error.code(), "no_evidence");
}

#[tokio::test]
async fn chat_mode_seats_the_evidence_and_leaves_the_group_where_it_is() {
    let fixture = fixture(MODEL).await;
    let outcome = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            mode: Some(InvestigationModeV1::Chat),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect("the session opens");

    let calls = fixture.harness.calls();
    assert!(
        !calls.iter().any(|call| matches!(call, Call::Send { .. })),
        "chat mode runs no turn: {calls:?}"
    );
    let Call::Append {
        entry_id,
        text,
        origin,
    } = calls
        .iter()
        .find(|call| matches!(call, Call::Append { .. }))
        .expect("the evidence is appended")
    else {
        unreachable!()
    };
    assert!(entry_id.ends_with(":evidence"));
    assert!(
        text.starts_with("## Sentinel · evidence — "),
        "the console recognises the entry by this heading: {text}"
    );
    assert_eq!(origin["sentinel_evidence"], json!(true));
    assert_eq!(origin["sentinel_group_id"], json!(fixture.group_id));

    assert_eq!(
        status_of(&fixture).await,
        GroupStatusV1::New,
        "nothing is running, so the group is not investigating"
    );
    let investigation = fixture
        .store
        .investigation_by_id(&outcome.value.investigation_id)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(investigation.status, InvestigationStatusV1::Open);
}

#[tokio::test]
async fn opening_the_chat_twice_reopens_the_session_that_already_has_the_evidence() {
    let fixture = fixture(MODEL).await;
    let request = || InvestigateRequestV1 {
        group_id: fixture.group_id.clone(),
        mode: Some(InvestigationModeV1::Chat),
        ..InvestigateRequestV1::default()
    };
    let first = fixture
        .investigations
        .investigate(request())
        .await
        .expect("the session opens")
        .value;
    let second = fixture
        .investigations
        .investigate(request())
        .await
        .expect("the second click is answered")
        .value;

    assert!(second.existing);
    assert_eq!(second.session_id, first.session_id);
    assert_eq!(
        fixture
            .harness
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Append { .. }))
            .count(),
        1,
        "the evidence is seated once, not once per click"
    );
}

#[tokio::test]
async fn a_chat_session_that_could_not_be_seated_is_not_left_waiting() {
    let fixture = fixture(MODEL).await;
    fixture.harness.fail_appends();
    let error = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            mode: Some(InvestigationModeV1::Chat),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect_err("a session without its evidence is not a session");
    assert_eq!(error.code(), "harness_unavailable");

    let (rows, _) = fixture
        .store
        .list_investigations(Some(&fixture.group_id), &[], 0, 10)
        .await
        .expect("list");
    assert_eq!(rows[0].status, InvestigationStatusV1::Failed);
    assert!(
        rows[0].error.is_some(),
        "and it says why, rather than sitting open forever"
    );
}

// ── recording ────────────────────────────────────────────────────────────

async fn open(fixture: &Fixture) -> (String, String) {
    let outcome = fixture
        .investigations
        .investigate(InvestigateRequestV1 {
            group_id: fixture.group_id.clone(),
            ..InvestigateRequestV1::default()
        })
        .await
        .expect("the investigation opens")
        .value;
    (outcome.investigation_id, outcome.session_id)
}

#[tokio::test]
async fn a_call_with_no_session_records_nothing() {
    let fixture = fixture(MODEL).await;
    let (_, _) = open(&fixture).await;
    let error = fixture
        .investigations
        .record(
            Caller::default(),
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("nope"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect_err("identity comes from the invocation, never from the payload");
    assert_eq!(error.code(), "no_investigation");
    assert_eq!(status_of(&fixture).await, GroupStatusV1::Investigating);
}

#[tokio::test]
async fn a_call_from_someone_elses_session_records_nothing() {
    let fixture = fixture(MODEL).await;
    open(&fixture).await;
    for session in ["s_user", "sentinel-inv-somebody-else"] {
        let error = fixture
            .investigations
            .record(
                Caller {
                    session_id: Some(session.into()),
                    turn_id: Some("turn-1".into()),
                },
                DiagnosisRecordRequestV1 {
                    group_id: fixture.group_id.clone(),
                    diagnosis: a_diagnosis("nope"),
                    _caller_worker_id: None,
                },
            )
            .await
            .expect_err("only this worker's own sessions record");
        assert_eq!(error.code(), "no_investigation", "session {session}");
    }
}

#[tokio::test]
async fn a_call_about_another_group_records_nothing() {
    let fixture = fixture(MODEL).await;
    let (_, session_id) = open(&fixture).await;
    let error = fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id),
                turn_id: Some("turn-1".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: "grp_somebody_else".into(),
                diagnosis: a_diagnosis("nope"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect_err("a session investigates one group");
    assert_eq!(error.code(), "not_this_group");
}

#[tokio::test]
async fn a_diagnosis_moves_the_group_forward_and_versions_up() {
    let fixture = fixture(MODEL).await;
    let (investigation_id, session_id) = open(&fixture).await;

    let first = fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id.clone()),
                turn_id: Some("turn-1".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("the operation table has no `up`"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect("the diagnosis is recorded");
    assert_eq!(first.value.version, 1);
    assert_eq!(first.value.group_status, GroupStatusV1::Diagnosed);
    assert!(first
        .events
        .iter()
        .any(|event| matches!(event, Announcement::Group(state) if state.status == GroupStatusV1::Diagnosed)));

    let second = fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id),
                turn_id: Some("turn-9".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("it is the parser, not the table"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect("a second recording supersedes the first");
    assert_eq!(second.value.version, 2);

    let records = fixture
        .store
        .diagnoses_for_investigation(&investigation_id)
        .await
        .expect("read");
    assert_eq!(records.len(), 2, "nothing is overwritten");
    assert_eq!(
        records[0].diagnosis.as_ref().map(|d| d.summary.as_str()),
        Some("it is the parser, not the table"),
        "newest first"
    );
    assert_eq!(
        records[0].source,
        sentinel::DiagnosisSourceV1::Conversation,
        "a turn that is not the first pass is the conversation"
    );
    assert_eq!(records[1].source, sentinel::DiagnosisSourceV1::FirstPass);
    assert_eq!(records[0].session_id, records[1].session_id);
    assert_eq!(records[0].model, MODEL);
}

#[tokio::test]
async fn a_resolve_that_lands_first_keeps_the_group_and_takes_the_diagnosis_anyway() {
    let fixture = fixture(MODEL).await;
    let (_, session_id) = open(&fixture).await;

    // The person clicks Resolve while the agent is still typing. A resolve
    // from `investigating` is refused by the lifecycle, so the console's real
    // race is the one after the pass released the group.
    fixture.investigations.reconcile().await;
    fixture
        .harness
        .set_status("completed", None, Some("turn-1"));
    fixture.investigations.reconcile().await;
    fixture
        .harness
        .set_status("completed", None, Some("turn-2"));
    fixture.investigations.reconcile().await;

    fixture
        .service
        .resolve(ResolveRequestV1 {
            group_id: fixture.group_id.clone(),
            until_version_change: false,
            _caller_worker_id: None,
        })
        .await
        .expect("a person resolves it");
    assert_eq!(status_of(&fixture).await, GroupStatusV1::Resolved);

    let outcome = fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id),
                turn_id: Some("turn-9".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("late, but true"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect("the record still lands");

    assert_eq!(
        outcome.value.group_status,
        GroupStatusV1::Resolved,
        "a decision a person made is not undone by an agent finishing after it"
    );
    assert_eq!(status_of(&fixture).await, GroupStatusV1::Resolved);
    assert_eq!(
        fixture
            .store
            .diagnoses_for_group(&fixture.group_id, 5)
            .await
            .expect("read")
            .len(),
        1
    );
}

#[tokio::test]
async fn the_history_keeps_every_version_and_says_how_many_there_are() {
    let fixture = fixture(MODEL).await;
    let (_, session_id) = open(&fixture).await;
    for summary in [
        "the operation table has no `up`",
        "it is the parser",
        "no, it is the cache",
    ] {
        fixture
            .investigations
            .record(
                Caller {
                    session_id: Some(session_id.clone()),
                    turn_id: Some("turn-1".into()),
                },
                DiagnosisRecordRequestV1 {
                    group_id: fixture.group_id.clone(),
                    diagnosis: a_diagnosis(summary),
                    _caller_worker_id: None,
                },
            )
            .await
            .expect("recorded");
    }

    let page = fixture
        .service
        .diagnoses(sentinel::DiagnosesListRequestV1 {
            group_id: fixture.group_id.clone(),
            limit: Some(2),
            ..sentinel::DiagnosesListRequestV1::default()
        })
        .await
        .expect("the history reads");
    assert_eq!(page.total, 3, "nothing is overwritten");
    assert_eq!(
        page.diagnoses.len(),
        2,
        "and the page is the page asked for"
    );
    assert_eq!(
        page.diagnoses[0]
            .diagnosis
            .as_ref()
            .map(|d| d.summary.as_str()),
        Some("no, it is the cache"),
        "newest first: the head is the one in force"
    );

    let rest = fixture
        .service
        .diagnoses(sentinel::DiagnosesListRequestV1 {
            group_id: fixture.group_id.clone(),
            offset: Some(2),
            limit: Some(2),
            ..sentinel::DiagnosesListRequestV1::default()
        })
        .await
        .expect("the second page reads");
    assert_eq!(rest.diagnoses.len(), 1);
    assert_eq!(
        rest.diagnoses[0]
            .diagnosis
            .as_ref()
            .map(|d| d.summary.as_str()),
        Some("the operation table has no `up`"),
    );
    assert_eq!(
        rest.diagnoses[0].session_id, session_id,
        "each version carries the session it was recorded from"
    );
}

// ── finishing ────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_first_pass_that_said_nothing_is_nudged_exactly_once() {
    let fixture = fixture(MODEL).await;
    open(&fixture).await;
    fixture
        .harness
        .set_status("completed", None, Some("turn-1"));

    let events = fixture.investigations.reconcile().await;
    assert!(
        events.is_empty(),
        "a nudge is not a state change: {events:?}"
    );
    let sends = fixture
        .harness
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Send {
                idempotency_key, ..
            } => Some(idempotency_key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sends.len(), 2);
    assert!(sends[1].ends_with(":nudge"));
    assert_eq!(
        status_of(&fixture).await,
        GroupStatusV1::Investigating,
        "the pass is still going: it was asked to finish properly"
    );

    // The nudge's own turn ends, still with nothing recorded.
    fixture
        .harness
        .set_status("completed", None, Some("turn-2"));
    let events = fixture.investigations.reconcile().await;
    assert_eq!(
        status_of(&fixture).await,
        GroupStatusV1::New,
        "an empty pass leaves no mark on the lifecycle"
    );
    assert!(events
        .iter()
        .any(|event| matches!(event, Announcement::Investigation(_))));
    assert_eq!(
        fixture
            .harness
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Send { .. }))
            .count(),
        2,
        "one nudge, ever"
    );
}

#[tokio::test]
async fn a_pass_that_recorded_leaves_the_group_diagnosed_and_the_cost_on_the_record() {
    let fixture = fixture(MODEL).await;
    let (investigation_id, session_id) = open(&fixture).await;
    fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id.clone()),
                turn_id: Some("turn-1".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("found it"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect("recorded");
    fixture
        .harness
        .set_status("completed", None, Some("turn-1"));

    let outcome = fixture
        .investigations
        .on_turn_completed(TurnCompletedEventV1 {
            session_id: session_id.clone(),
            turn_id: Some("turn-1".into()),
            ..TurnCompletedEventV1::default()
        })
        .await
        .expect("the doorbell is answered");
    assert!(outcome.value.handled);

    assert_eq!(status_of(&fixture).await, GroupStatusV1::Diagnosed);
    let investigation = fixture
        .store
        .investigation_by_id(&investigation_id)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(investigation.status, InvestigationStatusV1::Completed);
    assert_eq!(investigation.turns, Some(3));
    assert_eq!(investigation.cost_usd, Some(0.42));
    assert!(investigation.finished_ms.is_some());
}

#[tokio::test]
async fn a_pass_that_failed_puts_the_group_back_and_says_why() {
    let fixture = fixture(MODEL).await;
    let (investigation_id, _) = open(&fixture).await;
    fixture.harness.set_status(
        "failed",
        Some("the provider refused the request"),
        Some("turn-1"),
    );

    fixture.investigations.reconcile().await;

    assert_eq!(status_of(&fixture).await, GroupStatusV1::New);
    let investigation = fixture
        .store
        .investigation_by_id(&investigation_id)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(investigation.status, InvestigationStatusV1::Failed);
    assert_eq!(
        investigation.error.as_deref(),
        Some("the provider refused the request")
    );
}

#[tokio::test]
async fn a_session_the_harness_forgot_does_not_stay_running_forever() {
    let fixture = fixture(MODEL).await;
    let (investigation_id, _) = open(&fixture).await;
    fixture.harness.forget_session();

    fixture.investigations.reconcile().await;

    let investigation = fixture
        .store
        .investigation_by_id(&investigation_id)
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(investigation.status, InvestigationStatusV1::Failed);
    assert_eq!(status_of(&fixture).await, GroupStatusV1::New);
}

#[tokio::test]
async fn a_doorbell_for_a_turn_that_is_not_the_first_pass_changes_nothing() {
    let fixture = fixture(MODEL).await;
    let (_, session_id) = open(&fixture).await;
    fixture
        .harness
        .set_status("completed", None, Some("turn-7"));

    let outcome = fixture
        .investigations
        .on_turn_completed(TurnCompletedEventV1 {
            session_id,
            turn_id: Some("turn-7".into()),
            ..TurnCompletedEventV1::default()
        })
        .await
        .expect("answered");

    assert!(!outcome.value.handled, "the person's own turns are theirs");
    assert_eq!(status_of(&fixture).await, GroupStatusV1::Investigating);
}

#[tokio::test]
async fn a_doorbell_for_a_session_that_is_not_ours_is_ignored() {
    let fixture = fixture(MODEL).await;
    open(&fixture).await;
    let outcome = fixture
        .investigations
        .on_turn_completed(TurnCompletedEventV1 {
            session_id: "s_someone_working".into(),
            turn_id: Some("turn-1".into()),
            ..TurnCompletedEventV1::default()
        })
        .await
        .expect("answered");
    assert!(!outcome.value.handled);
    assert_eq!(status_of(&fixture).await, GroupStatusV1::Investigating);
}

#[tokio::test]
async fn cancelling_stops_the_turn_and_releases_the_group() {
    let fixture = fixture(MODEL).await;
    let (investigation_id, session_id) = open(&fixture).await;

    let outcome = fixture
        .investigations
        .cancel(sentinel::InvestigationCancelRequestV1 {
            investigation_id: investigation_id.clone(),
            _caller_worker_id: None,
        })
        .await
        .expect("cancelled");

    assert_eq!(outcome.value.status, InvestigationStatusV1::Cancelled);
    assert!(fixture.harness.calls().contains(&Call::Stop {
        session_id,
        turn_id: "turn-1".into(),
    }));
    assert_eq!(status_of(&fixture).await, GroupStatusV1::New);

    // Cancelling twice is not an error, and does not stop anything twice.
    let again = fixture
        .investigations
        .cancel(sentinel::InvestigationCancelRequestV1 {
            investigation_id,
            _caller_worker_id: None,
        })
        .await
        .expect("idempotent");
    assert_eq!(again.value.status, InvestigationStatusV1::Cancelled);
    assert_eq!(
        fixture
            .harness
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Stop { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn the_group_page_shows_the_diagnosis_and_the_session_to_reopen() {
    let fixture = fixture(MODEL).await;
    let (investigation_id, session_id) = open(&fixture).await;
    fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id.clone()),
                turn_id: Some("turn-1".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("found it"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect("recorded");

    let response = fixture
        .service
        .get(sentinel::GroupGetRequestV1::new(fixture.group_id.clone()))
        .await
        .expect("the page reads");
    assert_eq!(
        response
            .diagnosis
            .as_ref()
            .and_then(|record| record.diagnosis.as_ref())
            .map(|d| d.summary.as_str()),
        Some("found it")
    );
    assert_eq!(
        response.active_investigation.as_ref().map(|i| i.id.clone()),
        Some(investigation_id.clone())
    );
    assert_eq!(
        response
            .latest_investigation
            .as_ref()
            .map(|i| i.session_id.clone()),
        Some(session_id)
    );
    assert!(response.group.has_diagnosis);
}

#[tokio::test]
async fn the_status_surface_counts_what_is_running() {
    let fixture = fixture(MODEL).await;
    assert_eq!(
        fixture.store.investigation_counts().await.expect("counts"),
        sentinel::InvestigationCountsV1::default()
    );
    open(&fixture).await;
    let counts = fixture.store.investigation_counts().await.expect("counts");
    assert_eq!(counts.running, 1);
    assert_eq!(counts.open_sessions, 1);

    fixture
        .harness
        .set_status("completed", None, Some("turn-1"));
    fixture.investigations.reconcile().await;
    fixture
        .harness
        .set_status("completed", None, Some("turn-2"));
    fixture.investigations.reconcile().await;
    let counts = fixture.store.investigation_counts().await.expect("counts");
    assert_eq!(counts.running, 0);
    assert_eq!(
        counts.open_sessions, 1,
        "a finished pass leaves a session somebody can still talk to"
    );
}

// ── history ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_history_records_who_moved_the_group_and_why() {
    let fixture = fixture(MODEL).await;
    let (_, session_id) = open(&fixture).await;
    fixture
        .investigations
        .record(
            Caller {
                session_id: Some(session_id),
                turn_id: Some("turn-1".into()),
            },
            DiagnosisRecordRequestV1 {
                group_id: fixture.group_id.clone(),
                diagnosis: a_diagnosis("found it"),
                _caller_worker_id: None,
            },
        )
        .await
        .expect("recorded");
    fixture
        .service
        .resolve(ResolveRequestV1 {
            group_id: fixture.group_id.clone(),
            until_version_change: false,
            _caller_worker_id: None,
        })
        .await
        .expect("a person resolves it");

    let history = fixture
        .service
        .history(sentinel::GroupHistoryRequestV1 {
            group_id: fixture.group_id.clone(),
            ..sentinel::GroupHistoryRequestV1::default()
        })
        .await
        .expect("the history reads");

    let moves: Vec<(Option<GroupStatusV1>, GroupStatusV1, &str)> = history
        .transitions
        .iter()
        .map(|row| (row.from_status, row.to_status, row.actor.as_str()))
        .collect();
    assert_eq!(
        moves,
        vec![
            (
                Some(GroupStatusV1::Diagnosed),
                GroupStatusV1::Resolved,
                "console"
            ),
            (
                Some(GroupStatusV1::Investigating),
                GroupStatusV1::Diagnosed,
                "agent"
            ),
            (
                Some(GroupStatusV1::New),
                GroupStatusV1::Investigating,
                "investigation"
            ),
            (None, GroupStatusV1::New, "ingest"),
        ],
        "newest first, and the group's own beginning is the last row"
    );
    assert_eq!(history.total, 4);
    assert_eq!(
        history.transitions[0].reason,
        Some(sentinel::GroupChangeReasonV1::Resolved)
    );
}

#[tokio::test]
async fn a_decision_and_its_history_row_land_together_or_not_at_all() {
    // The row is written in the transaction that moves the group, so a
    // refused move leaves no trace claiming it happened.
    let fixture = fixture(MODEL).await;
    open(&fixture).await;
    let refused = fixture
        .service
        .resolve(ResolveRequestV1 {
            group_id: fixture.group_id.clone(),
            until_version_change: false,
            _caller_worker_id: None,
        })
        .await;
    assert!(
        refused.is_err(),
        "a running first pass cannot be resolved out from under"
    );

    let history = fixture
        .service
        .history(sentinel::GroupHistoryRequestV1 {
            group_id: fixture.group_id.clone(),
            ..sentinel::GroupHistoryRequestV1::default()
        })
        .await
        .expect("the history reads");
    assert!(
        !history
            .transitions
            .iter()
            .any(|row| row.to_status == GroupStatusV1::Resolved),
        "{:?}",
        history.transitions
    );
}
