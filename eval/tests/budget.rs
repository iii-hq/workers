//! The per-cycle budget: work is reserved (compare-and-set in the loop's
//! budget scope) before it is dispatched, limits on spend, time, calls and
//! candidates end the cycle inconclusive with reason `budget`, an unknown cost
//! is never zero, and a repeated event never starts another comparison.
//! The runtime runs over the real SDK wire protocol against an in-memory
//! engine; no credentials or remote services are used.

#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eval::contract::*;
use eval::cost::{self, Reservation};
use eval::decision;
use eval::events::EvalEvents;
use eval::runtime::{self, Deps};
use eval::{ids, state};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};

const WORK_FN: &str = "work::run";
const TIMEOUT_MS: u64 = 5_000;

// ---------------------------------------------------------------------------
// In-memory engine
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Engine {
    state: BTreeMap<(String, String), Value>,
    calls: Vec<(String, Value)>,
}

impl Engine {
    fn respond(&mut self, function: &str, data: &Value) -> Value {
        let key = (
            data["scope"].as_str().unwrap_or_default().to_string(),
            data["key"].as_str().unwrap_or_default().to_string(),
        );
        match function {
            "state::get" => self.state.get(&key).cloned().unwrap_or(Value::Null),
            "state::set" => {
                self.state.insert(key, data["value"].clone());
                json!({})
            }
            "state::compare-and-set" => {
                let current = self.state.get(&key).cloned();
                let matches = match data.get("expected") {
                    None | Some(Value::Null) => current.is_none(),
                    Some(expected) => current.as_ref() == Some(expected),
                };
                if matches {
                    self.state.insert(key, data["value"].clone());
                    json!({"swapped": true, "current": data["value"]})
                } else {
                    json!({"swapped": false, "current": current.unwrap_or(Value::Null)})
                }
            }
            "state::list" => Value::Array(
                self.state
                    .iter()
                    .filter(|((scope, _), _)| Some(scope.as_str()) == data["scope"].as_str())
                    .map(|(_, value)| value.clone())
                    .collect(),
            ),
            _ => json!({"ok": true}),
        }
    }
}

struct Rig {
    engine: Arc<Mutex<Engine>>,
    deps: Deps,
    iii: Arc<IIIClient>,
    _fake: fake_engine::FakeEngine,
}

impl Rig {
    async fn start() -> Self {
        let engine = Arc::new(Mutex::new(Engine::default()));
        let shared = engine.clone();
        let fake = fake_engine::start(move |frame: Value| {
            if frame["type"] != "invokefunction" || frame["invocation_id"].is_null() {
                return vec![];
            }
            let function = frame["function_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let mut engine = shared.lock().unwrap();
            engine.calls.push((function.clone(), frame["data"].clone()));
            let result = engine.respond(&function, &frame["data"]);
            vec![json!({"type": "invocationresult",
                "invocation_id": frame["invocation_id"],
                "function_id": function,
                "result": result})]
        })
        .await;
        let iii = Arc::new(register_worker(&fake.url, InitOptions::default()));
        tokio::time::timeout(Duration::from_secs(10), async {
            while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("worker connects to the fake engine");
        let deps = Deps::new(iii.clone(), EvalEvents::register(&iii));
        Self {
            engine,
            deps,
            iii,
            _fake: fake,
        }
    }

    /// A fresh process on the same storage: empty locks and in-flight set.
    fn restart(&mut self) {
        self.deps = Deps::new(self.iii.clone(), EvalEvents::register(&self.iii));
    }

    fn calls_to(&self, function: &str) -> Vec<Value> {
        self.engine
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(id, _)| id.ends_with(function))
            .map(|(_, data)| data.clone())
            .collect()
    }

    /// Index of the first call to `function` matching `pred`.
    fn position(&self, function: &str, pred: impl Fn(&Value) -> bool) -> Option<usize> {
        self.engine
            .lock()
            .unwrap()
            .calls
            .iter()
            .position(|(id, data)| id.ends_with(function) && pred(data))
    }

    async fn seed_cycle(&self, cycle_id: &str) -> CycleRecordV1 {
        let now = ids::now_ms();
        let record = CycleRecordV1 {
            schema_version: CYCLE_SCHEMA_VERSION,
            cycle_id: cycle_id.into(),
            input: CycleInputV1::Objective("cut the cost of scenario s".into()),
            status: CycleStatusV1::Running,
            step: 1,
            created_at: now,
            updated_at: now,
            completed_at: None,
            history: Vec::new(),
            result: None,
            failure: None,
            plan: None,
            decision: None,
            inconclusive: None,
        };
        state::put_cycle(&self.iii, &record).await.unwrap();
        record
    }

    /// Stores a ledger with `budget` for an already seeded cycle.
    async fn seed_ledger(&self, cycle_id: &str, budget: CycleBudgetV1, started_at: i64) {
        state::update_ledger(&self.iii, cycle_id, |_| {
            Ok((
                Some(cost::new_ledger(cycle_id, budget.clone(), started_at)),
                (),
            ))
        })
        .await
        .unwrap();
    }

    async fn dispatch(&self, cycle_id: &str, work: &CycleWorkV1) -> Option<Value> {
        runtime::dispatch_cycle_work(
            &self.deps,
            cycle_id,
            work,
            WORK_FN,
            json!({"work_id": work.work_id}),
            TIMEOUT_MS,
        )
        .await
        .unwrap()
    }
}

fn work(work_id: &str, kind: CycleWorkKindV1, estimated_usd: f64, candidate: bool) -> CycleWorkV1 {
    CycleWorkV1 {
        work_id: work_id.into(),
        kind,
        estimated_usd,
        candidate,
    }
}

fn budget(
    max_spend_usd: f64,
    max_duration_ms: i64,
    max_calls: u32,
    max_candidates: u32,
) -> CycleBudgetV1 {
    CycleBudgetV1 {
        max_spend_usd,
        max_duration_ms,
        max_calls,
        max_candidates,
    }
}

fn over_limit(reservation: &Reservation, limit: &str) {
    match reservation {
        Reservation::OverLimit(reason) => {
            assert!(reason.starts_with(limit), "{limit}: got {reason}")
        }
        other => panic!("{limit}: expected OverLimit, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Decision helpers (a compatible pair, cost 0.10 → 0.05 a run)
// ---------------------------------------------------------------------------

fn criterion(metric: CriterionMetricV1, direction: DirectionV1, min_effect: f64) -> CriterionV1 {
    CriterionV1 {
        metric,
        pattern: None,
        direction,
        min_effect,
        min_runs: 3,
        scenario_id: "s".into(),
        registered_at: 10,
        registered_by: "ana".into(),
    }
}

fn plan() -> CyclePlanV1 {
    CyclePlanV1 {
        objective: "cut the cost of scenario s".into(),
        criterion: criterion(CriterionMetricV1::CostUsd, DirectionV1::Decrease, 0.01),
        preserved: vec![criterion(
            CriterionMetricV1::PassRate,
            DirectionV1::Increase,
            0.0,
        )],
        controls: vec!["control-a".into()],
        sampling: SamplingPlanV1 {
            runs_per_side: 3,
            seeds: vec!["seed-1".into(), "seed-2".into()],
        },
        policy_revision: "policy-7".into(),
        frozen_at: 100,
    }
}

fn planned_record() -> CycleRecordV1 {
    let mut record = CycleRecordV1 {
        schema_version: CYCLE_SCHEMA_VERSION,
        cycle_id: "cycle-d".into(),
        input: CycleInputV1::Objective("cut the cost of scenario s".into()),
        status: CycleStatusV1::Running,
        step: 1,
        created_at: 1,
        updated_at: 1,
        completed_at: None,
        history: Vec::new(),
        result: None,
        failure: None,
        plan: None,
        decision: None,
        inconclusive: None,
    };
    decision::freeze(&mut record, plan()).unwrap();
    record
}

fn execution(execution_id: &str, harness_version: &str) -> E2eExecutionRefV1 {
    serde_json::from_value(json!({
        "execution_id": execution_id,
        "status": "completed",
        "reports_available": true,
        "reports": [],
        "model": "model-a",
        "provider": "provider-a",
        "harness_version": harness_version,
        "engine_version": "engine-1",
        "e2e_revision": "e2e-rev-1",
        "scenarios": [{
            "scenario_id": "s",
            "behavior_sha256": "behavior-1",
            "contract_fingerprint": "contract-1",
            "run_count": 3,
            "measures": {}
        }]
    }))
    .expect("a valid execution ref")
}

fn pair() -> (DecisionSideV1, DecisionSideV1) {
    let side = |execution_id: &str, harness: &str| DecisionSideV1 {
        execution: execution(execution_id, harness),
        seeds: vec!["seed-2".into(), "seed-1".into()],
        policy_revision: Some("policy-7".into()),
    };
    (side("base", "harness-1"), side("cand", "harness-2"))
}

fn runs(count: usize, passed: usize, cost: f64) -> Vec<EvidenceRunV1> {
    (0..count)
        .map(|at| EvidenceRunV1 {
            run_id: "r".into(),
            completed: true,
            completion: Some("completed".into()),
            technical: Some("valid".into()),
            status: Some(if at < passed { "passed" } else { "failed" }.into()),
            cost_usd: Some(cost),
            wall_time_ms: None,
            total_tokens: None,
            function_calls: None,
            signals: None,
        })
        .collect()
}

fn evidence(baseline: Vec<EvidenceRunV1>, candidate: Vec<EvidenceRunV1>) -> Vec<EvidenceV1> {
    let side = |execution_id: &str, runs: Vec<EvidenceRunV1>| EvidenceSideV1 {
        execution_id: execution_id.into(),
        runs,
        n: 0,
        mean: None,
    };
    vec![EvidenceV1 {
        scenario_id: "s".into(),
        computed_at: 0,
        baseline: side("base", baseline),
        candidate: side("cand", candidate),
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    }]
}

fn cheaper_and_correct() -> Vec<EvidenceV1> {
    evidence(runs(3, 3, 0.10), runs(3, 3, 0.05))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn work_is_reserved_before_dispatch_so_restart_does_not_double_spend() {
    let mut rig = Rig::start().await;
    rig.seed_cycle("c1").await;
    let jev = work("jev-1", CycleWorkKindV1::Jev, 0.5, false);

    // The reservation is written to the budget scope before the dispatch.
    let answer = rig.dispatch("c1", &jev).await;
    assert_eq!(answer, Some(json!({"ok": true})));
    let reserved_at = rig
        .position("state::compare-and-set", |data| {
            data["scope"] == state::BUDGET_SCOPE
                && data["key"] == "c1"
                && !data["value"]["reservations"]["jev-1"].is_null()
        })
        .expect("the reservation is compare-and-set in the budget scope");
    let dispatched_at = rig
        .position(WORK_FN, |data| data["work_id"] == "jev-1")
        .expect("the work is dispatched");
    assert!(
        reserved_at < dispatched_at,
        "reserve {reserved_at} before dispatch {dispatched_at}"
    );
    let ledger = state::get_ledger(&rig.iii, "c1").await.unwrap().unwrap();
    assert_eq!(ledger.reservations["jev-1"].work, jev);
    assert_eq!(ledger.reservations["jev-1"].settled_at, None);

    // A redelivery of the same work is not dispatched again.
    assert_eq!(rig.dispatch("c1", &jev).await, None);
    assert_eq!(rig.calls_to(WORK_FN).len(), 1);

    // Nor after a restart: the reservation is durable, not in memory.
    rig.restart();
    assert_eq!(rig.dispatch("c1", &jev).await, None);
    assert!(matches!(
        runtime::reserve_cycle_work(&rig.deps, "c1", &jev)
            .await
            .unwrap(),
        Reservation::Existing(_)
    ));
    assert_eq!(rig.calls_to(WORK_FN).len(), 1);
    let ledger = state::get_ledger(&rig.iii, "c1").await.unwrap().unwrap();
    assert_eq!(ledger.reservations.len(), 1);
    assert_eq!(cost::usage(&ledger, ids::now_ms()).spend_usd, 0.5);
    assert_eq!(cost::usage(&ledger, ids::now_ms()).calls, 1);

    // Concurrent callers of the same work: exactly one dispatches.
    let build = work("build-1", CycleWorkKindV1::Build, 0.25, true);
    let (first, second) = tokio::join!(rig.dispatch("c1", &build), rig.dispatch("c1", &build));
    assert_eq!(
        [first.is_some(), second.is_some()]
            .iter()
            .filter(|dispatched| **dispatched)
            .count(),
        1
    );
    assert_eq!(rig.calls_to(WORK_FN).len(), 2);
    let ledger = state::get_ledger(&rig.iii, "c1").await.unwrap().unwrap();
    assert_eq!(ledger.reservations.len(), 2);
    assert_eq!(cost::usage(&ledger, ids::now_ms()).spend_usd, 0.75);

    // The pure ledger, reloaded from its stored form, keeps the reservation.
    let mut reloaded: CycleLedgerV1 =
        serde_json::from_value(serde_json::to_value(&ledger).unwrap()).unwrap();
    assert!(matches!(
        cost::reserve(&mut reloaded, &jev, ids::now_ms()),
        Reservation::Existing(_)
    ));
    assert_eq!(reloaded, ledger);
}

#[tokio::test]
async fn limits_on_spend_time_calls_and_candidates_stop_the_cycle() {
    let now = 1_000;

    // Spend: what is already reserved plus the estimate must fit.
    let mut ledger = cost::new_ledger("c", budget(1.0, 60_000, 100, 10), now);
    assert!(matches!(
        cost::reserve(
            &mut ledger,
            &work("a", CycleWorkKindV1::Investigation, 0.6, false),
            now
        ),
        Reservation::Reserved(_)
    ));
    let before = ledger.clone();
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("b", CycleWorkKindV1::E2e, 0.6, false),
            now,
        ),
        "spend",
    );
    assert_eq!(ledger, before, "a refused reservation changes nothing");
    // A negative or non-finite estimate is never a free pass.
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("n", CycleWorkKindV1::E2e, -1.0, false),
            now,
        ),
        "spend",
    );
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("x", CycleWorkKindV1::E2e, f64::NAN, false),
            now,
        ),
        "spend",
    );

    // Calls: every kind of work, retries included, counts as one call.
    let mut ledger = cost::new_ledger("c", budget(100.0, 60_000, 3, 10), now);
    for (id, kind) in [
        ("i", CycleWorkKindV1::Investigation),
        ("c", CycleWorkKindV1::Control),
        ("r", CycleWorkKindV1::Retry),
    ] {
        assert!(matches!(
            cost::reserve(&mut ledger, &work(id, kind, 0.0, false), now),
            Reservation::Reserved(_)
        ));
    }
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("j", CycleWorkKindV1::Jev, 0.0, false),
            now,
        ),
        "calls",
    );
    assert!(cost::exhausted(&ledger, now).unwrap().starts_with("calls"));

    // Candidates: only work that tries a new candidate counts.
    let mut ledger = cost::new_ledger("c", budget(100.0, 60_000, 100, 1), now);
    assert!(matches!(
        cost::reserve(
            &mut ledger,
            &work("b1", CycleWorkKindV1::Build, 0.1, true),
            now
        ),
        Reservation::Reserved(_)
    ));
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("b2", CycleWorkKindV1::Build, 0.1, true),
            now,
        ),
        "candidates",
    );
    assert!(matches!(
        cost::reserve(
            &mut ledger,
            &work("e", CycleWorkKindV1::E2e, 0.1, false),
            now
        ),
        Reservation::Reserved(_)
    ));

    // Time: runs from when the cycle started.
    let mut ledger = cost::new_ledger("c", budget(100.0, 500, 100, 10), now);
    assert!(cost::exhausted(&ledger, now + 499).is_none());
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("t", CycleWorkKindV1::Jev, 0.0, false),
            now + 500,
        ),
        "time",
    );

    // In the runtime, work over a limit is not dispatched and the cycle ends
    // inconclusive with reason `budget`.
    let rig = Rig::start().await;
    let record = rig.seed_cycle("c2").await;
    rig.seed_ledger("c2", budget(1.0, 3_600_000, 100, 10), record.created_at)
        .await;
    assert!(rig
        .dispatch(
            "c2",
            &work("w1", CycleWorkKindV1::Investigation, 0.8, false)
        )
        .await
        .is_some());
    assert_eq!(
        rig.dispatch("c2", &work("w2", CycleWorkKindV1::E2e, 0.5, false))
            .await,
        None
    );
    assert_eq!(rig.calls_to(WORK_FN).len(), 1);
    let ended = state::get_cycle(&rig.iii, "c2").await.unwrap().unwrap();
    assert_eq!(ended.status, CycleStatusV1::Completed);
    let inconclusive = ended.inconclusive.expect("the cycle ends inconclusive");
    assert_eq!(inconclusive.reason, cost::BUDGET_REASON);
    assert_eq!(inconclusive.reason, "budget");
    assert!(
        inconclusive.detail.starts_with("spend"),
        "{}",
        inconclusive.detail
    );
    assert_eq!(ended.decision, None);
    let ledger = state::get_ledger(&rig.iii, "c2").await.unwrap().unwrap();
    assert!(
        !ledger.reservations.contains_key("w2"),
        "over-limit work is not reserved"
    );

    // A stopped cycle dispatches nothing more, even work that would fit.
    assert!(runtime::reserve_cycle_work(
        &rig.deps,
        "c2",
        &work("w3", CycleWorkKindV1::Control, 0.0, false)
    )
    .await
    .is_err());
    assert_eq!(rig.calls_to(WORK_FN).len(), 1);

    // The calls limit stops the cycle the same way.
    let record = rig.seed_cycle("c3").await;
    rig.seed_ledger("c3", budget(100.0, 3_600_000, 1, 10), record.created_at)
        .await;
    assert!(rig
        .dispatch("c3", &work("r1", CycleWorkKindV1::Retry, 0.0, false))
        .await
        .is_some());
    assert_eq!(
        rig.dispatch("c3", &work("r2", CycleWorkKindV1::Retry, 0.0, false))
            .await,
        None
    );
    let ended = state::get_cycle(&rig.iii, "c3").await.unwrap().unwrap();
    let inconclusive = ended.inconclusive.expect("the cycle ends inconclusive");
    assert_eq!(inconclusive.reason, "budget");
    assert!(
        inconclusive.detail.starts_with("calls"),
        "{}",
        inconclusive.detail
    );
}

#[tokio::test]
async fn unknown_cost_is_not_zero() {
    let now = 1_000;
    let mut ledger = cost::new_ledger("c", budget(1.0, 60_000, 100, 10), now);
    let jev = work("jev", CycleWorkKindV1::Jev, 0.4, false);
    assert!(matches!(
        cost::reserve(&mut ledger, &jev, now),
        Reservation::Reserved(_)
    ));

    // Settled without a cost: still charged at the estimate, counted unknown.
    assert!(cost::settle(&mut ledger, "jev", None, now + 1).unwrap());
    let usage = cost::usage(&ledger, now + 1);
    assert_eq!(usage.spend_usd, 0.4);
    assert_eq!(usage.unknown_cost, 1);
    assert_eq!(usage.calls, 1);
    // Settling is once: a later report changes nothing.
    assert!(!cost::settle(&mut ledger, "jev", Some(0.0), now + 2).unwrap());
    assert_eq!(cost::usage(&ledger, now + 2).unknown_cost, 1);
    assert_eq!(cost::usage(&ledger, now + 2).spend_usd, 0.4);
    // The unknown cost consumes the spend limit.
    over_limit(
        &cost::reserve(
            &mut ledger,
            &work("e2e", CycleWorkKindV1::E2e, 0.7, false),
            now,
        ),
        "spend",
    );
    // A non-finite or negative report is unknown, not zero.
    let build = work("build", CycleWorkKindV1::Build, 0.2, false);
    assert!(matches!(
        cost::reserve(&mut ledger, &build, now),
        Reservation::Reserved(_)
    ));
    assert!(cost::settle(&mut ledger, "build", Some(f64::NAN), now).unwrap());
    let usage = cost::usage(&ledger, now);
    assert_eq!(usage.unknown_cost, 2);
    assert!((usage.spend_usd - 0.6).abs() < 1e-9, "{}", usage.spend_usd);
    // A reported cost replaces the estimate.
    let control = work("control", CycleWorkKindV1::Control, 0.3, false);
    assert!(matches!(
        cost::reserve(&mut ledger, &control, now),
        Reservation::Reserved(_)
    ));
    assert!(cost::settle(&mut ledger, "control", Some(0.1), now).unwrap());
    let usage = cost::usage(&ledger, now);
    assert_eq!(usage.unknown_cost, 2);
    assert!((usage.spend_usd - 0.7).abs() < 1e-9, "{}", usage.spend_usd);
    // Settling unknown work is an error, not a silent zero.
    assert!(cost::settle(&mut ledger, "missing", Some(0.0), now).is_err());

    // An unknown cost blocks a cost-reduction claim.
    let (baseline, candidate) = pair();
    let mut known = planned_record();
    let accepted = decision::decide_cycle_spent(
        &mut known,
        None,
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        now,
    )
    .unwrap();
    assert_eq!(
        accepted.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        accepted.reason
    );
    let mut unknown = planned_record();
    let blocked = decision::decide_cycle_spent(
        &mut unknown,
        Some(&ledger),
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        now,
    )
    .unwrap();
    assert_eq!(blocked.outcome, DecisionOutcomeV1::Inconclusive);
    assert!(
        blocked.reason.contains("unknown cost"),
        "{}",
        blocked.reason
    );
    assert_eq!(unknown.decision.as_ref(), Some(&blocked));
    // With every cost known, the same evidence is accepted.
    let mut all_known = cost::new_ledger("c", budget(1.0, 60_000, 100, 10), now);
    assert!(matches!(
        cost::reserve(&mut all_known, &jev, now),
        Reservation::Reserved(_)
    ));
    assert!(cost::settle(&mut all_known, "jev", Some(0.3), now).unwrap());
    let mut settled = planned_record();
    let accepted = decision::decide_cycle_spent(
        &mut settled,
        Some(&all_known),
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        now,
    )
    .unwrap();
    assert_eq!(
        accepted.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        accepted.reason
    );

    // In the runtime the unknown cost is stored as such and still charged.
    let rig = Rig::start().await;
    rig.seed_cycle("c4").await;
    let investigation = work("inv", CycleWorkKindV1::Investigation, 0.9, false);
    assert!(rig.dispatch("c4", &investigation).await.is_some());
    assert!(runtime::settle_cycle_work(&rig.deps, "c4", "inv", None)
        .await
        .unwrap());
    assert!(
        !runtime::settle_cycle_work(&rig.deps, "c4", "inv", Some(0.0))
            .await
            .unwrap()
    );
    let stored = state::get_ledger(&rig.iii, "c4").await.unwrap().unwrap();
    assert!(stored.reservations["inv"].settled_at.is_some());
    assert_eq!(stored.reservations["inv"].cost_usd, None);
    let usage = cost::usage(&stored, ids::now_ms());
    assert_eq!(usage.unknown_cost, 1);
    assert_eq!(usage.spend_usd, 0.9);
}

#[tokio::test]
async fn repeated_event_does_not_start_another_comparison() {
    // The decision on the cycle's pair is made once: a repeated event, even
    // with other evidence or another ledger, returns the recorded one.
    let (baseline, candidate) = pair();
    let mut record = planned_record();
    let first = decision::decide_cycle_spent(
        &mut record,
        None,
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        10,
    )
    .unwrap();
    assert_eq!(
        first.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        first.reason
    );
    let stored = record.clone();
    let mut unknown = cost::new_ledger("cycle-d", cost::default_budget(), 0);
    let jev = work("jev", CycleWorkKindV1::Jev, 0.1, false);
    assert!(matches!(
        cost::reserve(&mut unknown, &jev, 0),
        Reservation::Reserved(_)
    ));
    cost::settle(&mut unknown, "jev", None, 1).unwrap();
    let worse = evidence(runs(3, 3, 0.05), runs(3, 0, 0.10));
    for (ledger, evidence) in [(None, &worse), (Some(&unknown), &cheaper_and_correct())] {
        let again =
            decision::decide_cycle_spent(&mut record, ledger, &baseline, &candidate, evidence, 99)
                .unwrap();
        assert_eq!(again, first);
        assert_eq!(record, stored, "a repeated event changes nothing");
    }
    assert_eq!(
        decision::decide_cycle(&mut record, &baseline, &candidate, &worse, 100).unwrap(),
        first
    );

    // The comparison work of a repeated event is reserved under the same
    // work id: it is never dispatched a second time.
    let mut rig = Rig::start().await;
    rig.seed_cycle("c5").await;
    let comparison = work("compare-base-cand", CycleWorkKindV1::E2e, 0.5, true);
    assert!(rig.dispatch("c5", &comparison).await.is_some());
    for _ in 0..3 {
        assert_eq!(rig.dispatch("c5", &comparison).await, None);
    }
    rig.restart();
    assert_eq!(rig.dispatch("c5", &comparison).await, None);
    assert_eq!(rig.calls_to(WORK_FN).len(), 1);
    let ledger = state::get_ledger(&rig.iii, "c5").await.unwrap().unwrap();
    let usage = cost::usage(&ledger, ids::now_ms());
    assert_eq!((usage.calls, usage.candidates), (1, 1));
    assert_eq!(usage.spend_usd, 0.5);
}
