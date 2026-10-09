//! Triage over a real SQLite store, with the registry and the judge faked.

#[path = "support/sqlite.rs"]
mod sqlite;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sentinel::registry::{EngineRegistry, FunctionEntry, Registry, WorkerEntry};
use sentinel::service::TraceAvailability;
use sentinel::store::{Db, OccurrenceWrite};
use sentinel::triage::{Judge, Triage, BATCH};
use sentinel::{
    ErrorSourceV1, GroupsListRequestV1, SentinelError, Service, Store, TriageKindV1,
    TriageSourceV1, WorkerConfig,
};
use serde_json::{json, Value};
use sqlite::SqliteDb;

const MINUTE: i64 = 60_000;

struct FakeRegistry {
    functions: Vec<&'static str>,
}

#[async_trait]
impl EngineRegistry for FakeRegistry {
    async fn list_functions(&self) -> Result<Vec<FunctionEntry>, SentinelError> {
        Ok(self
            .functions
            .iter()
            .map(|id| FunctionEntry {
                function_id: id.to_string(),
                namespace: "my-project".into(),
                worker_name: id.split("::").next().unwrap_or_default().into(),
            })
            .collect())
    }

    async fn list_workers(&self) -> Result<Vec<WorkerEntry>, SentinelError> {
        Ok(Vec::new())
    }
}

/// Answers every evaluation with the kind `answer` picks from its state, or
/// with `reply` verbatim when one is set.
struct FakeJudge {
    requests: Mutex<Vec<Value>>,
    answer: fn(&Value) -> &'static str,
    reply: Option<Value>,
}

#[async_trait]
impl Judge for FakeJudge {
    async fn evaluate(&self, request: Value) -> Result<Value, SentinelError> {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(reply) = &self.reply {
            return Ok(reply.clone());
        }
        let results: serde_json::Map<String, Value> = request["evaluations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|evaluation| {
                let choice = (self.answer)(&evaluation["state"]);
                (
                    evaluation["id"].as_str().unwrap().to_string(),
                    json!({ "answers": { "kind": {
                        "type": "choice", "choice": choice, "confidence": 0.8,
                        "probabilities": {}
                    } } }),
                )
            })
            .collect();
        Ok(json!({ "status": "ok", "model": "jev-test", "results": results }))
    }
}

struct NoTrace;

#[async_trait]
impl TraceAvailability for NoTrace {
    async fn trace_exists(&self, _trace_id: &str) -> bool {
        false
    }
}

struct Setup {
    store: Arc<Store<SqliteDb>>,
    judge: Arc<FakeJudge>,
    triage: Triage<SqliteDb, FakeRegistry>,
}

async fn setup(functions: Vec<&'static str>, judge: FakeJudge) -> Setup {
    let store = Arc::new(Store::new(SqliteDb::in_memory()));
    store.migrate().await.expect("migrate");
    let registry = Arc::new(Registry::new(
        FakeRegistry { functions },
        Duration::from_secs(60),
    ));
    let judge = Arc::new(judge);
    let triage = Triage::new(store.clone(), registry, judge.clone());
    Setup {
        store,
        judge,
        triage,
    }
}

fn judge(answer: fn(&Value) -> &'static str) -> FakeJudge {
    FakeJudge {
        requests: Mutex::new(Vec::new()),
        answer,
        reply: None,
    }
}

/// One occurrence of a group at each of `at`.
async fn group(
    store: &Store<SqliteDb>,
    fingerprint: &str,
    function_id: &str,
    message: &str,
    at: &[i64],
) {
    for (index, at_ms) in at.iter().enumerate() {
        store
            .record_occurrence(&OccurrenceWrite {
                fingerprint: fingerprint.into(),
                source: ErrorSourceV1::Trace,
                dedupe_key: format!("{fingerprint}-{index}"),
                at_ms: *at_ms,
                namespace: "default".into(),
                service_name: "iii".into(),
                function_id: Some(function_id.into()),
                exception_type: None,
                title: format!("{function_id}: {message}"),
                message: message.into(),
                trace_id: None,
                span_id: None,
                session_id: None,
                turn_id: None,
                worker_version: None,
                evidence: None,
                namespace_ambiguous: false,
                pending_occurrence_id: None,
            })
            .await
            .expect("record");
    }
}

async fn triage_of(
    store: &Arc<Store<SqliteDb>>,
    function_id: &str,
) -> Option<(TriageKindV1, TriageSourceV1)> {
    let service = Service::new(store.clone(), Arc::new(NoTrace));
    let list = service
        .list(GroupsListRequestV1::default())
        .await
        .expect("list");
    list.groups
        .into_iter()
        .find(|group| group.function_id.as_deref() == Some(function_id))
        .and_then(|group| group.triage)
        .map(|triage| (triage.kind, triage.source))
}

fn calls(setup: &Setup) -> usize {
    setup.judge.requests.lock().unwrap().len()
}

#[tokio::test]
async fn a_start_up_race_is_labelled_by_rule_without_asking_the_judge() {
    let setup = setup(vec!["state::get", "judge::evaluate"], judge(|_| "defect")).await;
    group(
        &setup.store,
        "fp_race",
        "state::get",
        "Function not found",
        &[0, MINUTE],
    )
    .await;

    let config = WorkerConfig::default();
    let labelled = setup
        .triage
        .sweep(&config, 10 * MINUTE)
        .await
        .expect("sweep");

    assert_eq!(labelled, 1);
    assert_eq!(
        triage_of(&setup.store, "state::get").await,
        Some((TriageKindV1::Transient, TriageSourceV1::Rule))
    );
    assert_eq!(calls(&setup), 0, "a rule needs no model");
}

#[tokio::test]
async fn a_long_outage_is_the_environment_and_only_a_truly_missing_function_reaches_the_judge() {
    let setup = setup(
        vec!["state::get", "judge::evaluate"],
        judge(|_| "caller_error"),
    )
    .await;
    // Registered now, but it failed for two hours: the worker was down.
    group(
        &setup.store,
        "fp_long",
        "state::get",
        "Function not found",
        &[0, 120 * MINUTE],
    )
    .await;
    group(
        &setup.store,
        "fp_gone",
        "harness::status",
        "remote error (function_not_found): Function stream::set not found in namespace my-project.",
        &[0],
    )
    .await;

    setup
        .triage
        .sweep(&WorkerConfig::default(), 130 * MINUTE)
        .await
        .expect("sweep");

    assert_eq!(
        triage_of(&setup.store, "state::get").await,
        Some((TriageKindV1::Environment, TriageSourceV1::Rule))
    );
    let requests = setup.judge.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let evaluations = requests[0]["evaluations"].as_array().unwrap();
    assert_eq!(evaluations.len(), 1);
    let state = &evaluations[0]["state"];
    assert_eq!(state["function"], "harness::status");
    assert_eq!(
        state["missing_function"],
        json!({ "id": "stream::set", "registered_now": false })
    );
    assert_eq!(
        triage_of(&setup.store, "harness::status").await,
        Some((TriageKindV1::CallerError, TriageSourceV1::Judge))
    );
}

#[tokio::test]
async fn each_group_gets_the_kind_the_judge_chose_and_young_groups_wait() {
    let setup = setup(
        vec!["judge::evaluate"],
        judge(|state| {
            if state["message"]
                .as_str()
                .unwrap()
                .contains("unknown session")
            {
                "caller_error"
            } else {
                "defect"
            }
        }),
    )
    .await;
    group(
        &setup.store,
        "fp_caller",
        "browser::screenshot",
        "unknown session s_1",
        &[0],
    )
    .await;
    group(
        &setup.store,
        "fp_bug",
        "fp::get",
        "index out of bounds",
        &[0],
    )
    .await;
    group(
        &setup.store,
        "fp_young",
        "fp::map",
        "index out of bounds",
        &[8 * MINUTE],
    )
    .await;

    let labelled = setup
        .triage
        .sweep(&WorkerConfig::default(), 10 * MINUTE)
        .await
        .expect("sweep");

    assert_eq!(labelled, 2);
    assert_eq!(
        triage_of(&setup.store, "browser::screenshot").await,
        Some((TriageKindV1::CallerError, TriageSourceV1::Judge))
    );
    assert_eq!(
        triage_of(&setup.store, "fp::get").await,
        Some((TriageKindV1::Defect, TriageSourceV1::Judge))
    );
    assert_eq!(
        triage_of(&setup.store, "fp::map").await,
        None,
        "inside the delay"
    );

    // A labelled group is never asked about again.
    setup
        .triage
        .sweep(&WorkerConfig::default(), 20 * MINUTE)
        .await
        .expect("sweep");
    let requests = setup.judge.requests.lock().unwrap().clone();
    assert_eq!(requests[1]["evaluations"].as_array().unwrap().len(), 1);
    assert_eq!(
        requests[1]["evaluations"][0]["state"]["function"],
        "fp::map"
    );
}

#[tokio::test]
async fn a_failing_judge_leaves_groups_untriaged_and_is_left_alone_for_a_while() {
    let mut failing = judge(|_| "defect");
    failing.reply = Some(json!({ "status": "error", "code": "http", "http_status": 402 }));
    let setup = setup(vec!["judge::evaluate"], failing).await;
    group(
        &setup.store,
        "fp_bug",
        "fp::get",
        "index out of bounds",
        &[0],
    )
    .await;
    let config = WorkerConfig::default();

    assert_eq!(setup.triage.sweep(&config, 10 * MINUTE).await.unwrap(), 0);
    assert_eq!(triage_of(&setup.store, "fp::get").await, None);
    assert_eq!(calls(&setup), 1);

    setup.triage.sweep(&config, 11 * MINUTE).await.unwrap();
    assert_eq!(calls(&setup), 1, "paused after a failure");

    setup.triage.sweep(&config, 16 * MINUTE).await.unwrap();
    assert_eq!(calls(&setup), 2, "asked again once the pause ran out");
}

#[tokio::test]
async fn groups_the_judge_cannot_label_do_not_keep_newer_ones_waiting() {
    // No judge deployed: a full page of groups only it could label stays
    // untriaged, and the newer group the rule settles is past that page.
    let setup = setup(vec!["state::get"], judge(|_| "defect")).await;
    for index in 0..BATCH {
        let fingerprint = format!("fp_bug_{index}");
        let function_id = format!("bug{index}::get");
        group(
            &setup.store,
            &fingerprint,
            &function_id,
            "index out of bounds",
            &[index as i64],
        )
        .await;
    }
    group(
        &setup.store,
        "fp_race",
        "state::get",
        "Function not found",
        &[MINUTE, 2 * MINUTE],
    )
    .await;
    let config = WorkerConfig::default();

    assert_eq!(setup.triage.sweep(&config, 10 * MINUTE).await.unwrap(), 0);
    assert_eq!(setup.triage.sweep(&config, 11 * MINUTE).await.unwrap(), 1);
    assert_eq!(
        triage_of(&setup.store, "state::get").await,
        Some((TriageKindV1::Transient, TriageSourceV1::Rule))
    );
    assert_eq!(
        setup.triage.sweep(&config, 12 * MINUTE).await.unwrap(),
        0,
        "the end of the list starts the sweeps over from the oldest"
    );
}

#[tokio::test]
async fn without_a_judge_deployed_or_with_triage_off_nothing_is_asked() {
    let setup = setup(vec![], judge(|_| "defect")).await;
    group(
        &setup.store,
        "fp_bug",
        "fp::get",
        "index out of bounds",
        &[0],
    )
    .await;

    let mut config = WorkerConfig::default();
    assert_eq!(setup.triage.sweep(&config, 10 * MINUTE).await.unwrap(), 0);
    config.triage.enabled = false;
    assert_eq!(setup.triage.sweep(&config, 10 * MINUTE).await.unwrap(), 0);
    assert_eq!(calls(&setup), 0);
    assert_eq!(triage_of(&setup.store, "fp::get").await, None);
}

#[tokio::test]
async fn the_list_filter_and_the_summary_flag_agree_on_what_is_relevant() {
    use sentinel::{GroupTriageV1, RelevanceV1};
    let setup = setup(vec![], judge(|_| "defect")).await;
    let long: Vec<i64> = (0..25).map(|index| index * 5 * MINUTE).collect();
    // (function, occurrences at, kind): relevant when marked so.
    let cases: [(&str, &[i64], Option<TriageKindV1>, bool); 7] = [
        ("a::defect", &[0], Some(TriageKindV1::Defect), true),
        ("a::untriaged", &[0], None, true),
        (
            "a::persistent-caller",
            &long,
            Some(TriageKindV1::CallerError),
            true,
        ),
        (
            "a::one-off-caller",
            &[0, MINUTE],
            Some(TriageKindV1::CallerError),
            false,
        ),
        (
            "a::persistent-env",
            &long,
            Some(TriageKindV1::Environment),
            true,
        ),
        ("a::transient", &long, Some(TriageKindV1::Transient), false),
        ("a::test", &[0], Some(TriageKindV1::TestTraffic), false),
    ];
    for (function, at, kind, _) in cases {
        group(&setup.store, function, function, "boom", at).await;
        if let Some(kind) = kind {
            let id = setup
                .store
                .db()
                .query(
                    "SELECT id FROM sentinel_groups WHERE fingerprint = ?",
                    vec![json!(function)],
                )
                .await
                .unwrap()[0]["id"]
                .as_str()
                .unwrap()
                .to_string();
            let triage = GroupTriageV1 {
                kind,
                confidence: 0.9,
                source: TriageSourceV1::Judge,
                model: None,
                at_ms: 0,
            };
            setup.store.set_triage(&id, &triage).await.unwrap();
        }
    }

    let service = Service::new(setup.store.clone(), Arc::new(NoTrace));
    let list = |relevance| {
        let service = &service;
        async move {
            service
                .list(GroupsListRequestV1 {
                    relevance,
                    ..GroupsListRequestV1::default()
                })
                .await
                .unwrap()
        }
    };
    let all = list(None).await;
    let relevant = list(Some(RelevanceV1::Relevant)).await;
    let noise = list(Some(RelevanceV1::Noise)).await;

    for (function, _, _, expected) in cases {
        let summary = all
            .groups
            .iter()
            .find(|group| group.function_id.as_deref() == Some(function))
            .unwrap();
        assert_eq!(summary.relevant, expected, "{function}: the summary flag");
        let side = if expected { &relevant } else { &noise };
        assert!(
            side.groups
                .iter()
                .any(|group| group.function_id.as_deref() == Some(function)),
            "{function}: the list filter"
        );
    }
    assert_eq!((all.relevant_total, all.noise_total), (4, 3));
    assert_eq!((relevant.total, noise.total, all.total), (4, 3, 7));
    assert_eq!(
        (relevant.relevant_total, noise.noise_total),
        (4, 3),
        "each side still counts the other"
    );
}
