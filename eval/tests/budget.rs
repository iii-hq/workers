//! The per-cycle budget: limits on spend, time, calls and candidates, a
//! reservation found again is never dispatched twice, an unknown cost is
//! never zero, and a cost reduction is never accepted while a cost is
//! unknown.

use eval::contract::*;
use eval::cost::{self, Reservation};
use eval::decision;
use eval::error::EvalError;
use eval::evidence;

fn budget() -> CycleBudgetV1 {
    CycleBudgetV1 {
        max_spend_usd: 1.0,
        max_duration_ms: 1_000,
        max_calls: 3,
        max_candidates: 1,
    }
}

fn work(work_id: &str, estimated_usd: f64, candidate: bool) -> CycleWorkV1 {
    CycleWorkV1 {
        work_id: work_id.into(),
        kind: CycleWorkKindV1::E2e,
        estimated_usd,
        candidate,
    }
}

fn trusted_work(work_id: &str, estimated_usd: f64) -> CycleWorkV1 {
    CycleWorkV1 {
        kind: CycleWorkKindV1::Investigation,
        ..work(work_id, estimated_usd, false)
    }
}

#[test]
fn a_reservation_found_again_is_never_reserved_twice() {
    let mut ledger = cost::new_ledger("c", budget(), 0);
    assert!(matches!(
        cost::reserve(&mut ledger, &work("c:baseline", 0.4, false), 1).unwrap(),
        Reservation::Reserved(_)
    ));
    // Redelivery, a concurrent caller or a restart reserves the same id.
    assert!(matches!(
        cost::reserve(&mut ledger, &work("c:baseline", 0.4, false), 2).unwrap(),
        Reservation::Existing(_)
    ));
    let usage = cost::usage(&ledger, 2);
    assert_eq!((usage.calls, usage.spend_usd), (1, 0.4));
}

#[test]
fn limits_on_spend_time_calls_and_candidates_stop_new_work() {
    let over = |reservation: Reservation, wanted: &str| match reservation {
        Reservation::OverLimit(reason) => assert!(reason.starts_with(wanted), "{reason}"),
        other => panic!("{wanted}: {other:?}"),
    };
    let mut ledger = cost::new_ledger("c", budget(), 0);
    over(
        cost::reserve(&mut ledger, &work("a", 1.5, false), 1).unwrap(),
        "spend",
    );
    over(
        cost::reserve(&mut ledger, &work("a", 0.1, false), 1_000).unwrap(),
        "time",
    );
    cost::reserve(&mut ledger, &work("cand-1", 0.1, true), 1).unwrap();
    over(
        cost::reserve(&mut ledger, &work("cand-2", 0.1, true), 1).unwrap(),
        "candidates",
    );
    cost::reserve(&mut ledger, &work("b", 0.1, false), 1).unwrap();
    cost::reserve(&mut ledger, &work("c", 0.1, false), 1).unwrap();
    over(
        cost::reserve(&mut ledger, &work("d", 0.1, false), 1).unwrap(),
        "calls",
    );
    assert!(cost::exhausted(&ledger, 1).is_some());
    // The third call was reserved within the limit: it still runs, only a
    // passed limit (or the time) ends the cycle.
    assert_eq!(cost::exceeded(&ledger, 1), None);
    assert!(cost::exceeded(&ledger, 1_000).is_some());
}

#[test]
fn an_unusable_estimate_is_an_invalid_request_not_a_budget_limit() {
    let mut ledger = cost::new_ledger("c", budget(), 0);
    for estimate in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            cost::reserve(&mut ledger, &work("a", estimate, false), 1),
            Err(EvalError::InvalidRequest(_))
        ));
    }
    assert!(ledger.reservations.is_empty());
}

#[test]
fn an_unknown_cost_stays_charged_at_its_estimate_and_settles_once() {
    let mut ledger = cost::new_ledger("c", budget(), 0);
    cost::reserve(&mut ledger, &work("a", 0.4, false), 1).unwrap();
    cost::reserve(&mut ledger, &trusted_work("b", 0.3), 1).unwrap();
    assert!(cost::settle(&mut ledger, "a", None, 2).unwrap());
    assert!(cost::settle(&mut ledger, "b", Some(0.1), 2).unwrap());
    assert!(
        !cost::settle(&mut ledger, "b", Some(9.0), 3).unwrap(),
        "settled once"
    );
    let usage = cost::usage(&ledger, 3);
    assert!((usage.spend_usd - 0.5).abs() < 1e-9, "{}", usage.spend_usd);
    assert_eq!(usage.unknown_cost, 1);
    assert!(cost::settle(&mut ledger, "missing", None, 3).is_err());
}

#[test]
fn an_e2e_report_cannot_release_its_reserved_estimate() {
    let mut ledger = cost::new_ledger("c", budget(), 0);
    cost::reserve(&mut ledger, &work("e2e", 1.0, false), 1).unwrap();
    cost::settle(&mut ledger, "e2e", Some(0.0), 2).unwrap();

    let usage = cost::usage(&ledger, 2);
    assert_eq!(usage.spend_usd, 1.0);
    assert!(matches!(
        cost::reserve(&mut ledger, &trusted_work("next", 0.01), 3).unwrap(),
        Reservation::OverLimit(_)
    ));
    assert_eq!(ledger.reservations["e2e"].cost_usd, Some(0.0));

    let mut trusted = cost::new_ledger("trusted", budget(), 0);
    cost::reserve(&mut trusted, &trusted_work("analysis", 0.5), 1).unwrap();
    cost::settle(&mut trusted, "analysis", Some(0.1), 2).unwrap();
    assert_eq!(cost::usage(&trusted, 2).spend_usd, 0.1);
}

#[test]
fn a_report_with_any_invalid_native_cost_is_unknown() {
    let bundle = serde_json::json!({"detail": {"reports": [{"report": {
        "scenarios": [{"runs": [
            {"cost": {"total_usd": 1.0}},
            {"cost": {"total_usd": -1.0}}
        ]}]
    }}]}});
    assert_eq!(evidence::reported_cost(&bundle), None);
}

#[test]
fn no_cost_reduction_is_accepted_while_a_cost_is_unknown() {
    let criterion = |metric, direction, min_effect| CriterionV1 {
        metric,
        pattern: None,
        criterion_id: None,
        direction,
        min_effect,
        min_runs: 1,
        scenario_id: "s".into(),
    };
    let plan = CyclePlanV1 {
        objective: "cut cost".into(),
        tests: vec![SelectedTestV1 {
            scenario_id: "s".into(),
            case_id: "case".into(),
            seed: 1,
            behavior_sha256: "b".into(),
            contract_sha256: "k".into(),
            inputs_sha256: "i".into(),
            criteria: Vec::new(),
            invariants: Vec::new(),
            matched_terms: Vec::new(),
            rationale: "r".into(),
            history_note: None,
        }],
        criterion: criterion(CriterionMetricV1::CostUsd, DirectionV1::Decrease, 0.01),
        preserved: vec![criterion(
            CriterionMetricV1::PassRate,
            DirectionV1::Increase,
            0.0,
        )],
        controls: Vec::new(),
        sampling: SamplingPlanV1 {
            runs_per_side: 5,
            seeds: Vec::new(),
        },
        parameters: ExecutionParametersV1 {
            provider: "p".into(),
            model: "m".into(),
            runs: 1,
            scenarios: vec!["s".into()],
            technical_retries: 0,
            where_: WhereV1::Harness,
            stack: None,
            agent: None,
        },
        estimated_usd: 0.1,
        policy_revision: "p1".into(),
        frozen_at: 0,
    };
    let identity = EvidenceIdentityV1 {
        scenarios: vec![ScenarioIdentityV1 {
            scenario_id: "s".into(),
            case_id: Some("case".into()),
            behavior_sha256: Some("b".into()),
            inputs_sha256: Some("i".into()),
            seed: Some(1),
            contract_sha256: Some("k".into()),
        }],
        subject_sha256: Some("subject".into()),
        fixed_stack_digest: Some("stack".into()),
        contract_digest: Some("c".into()),
        model: Some("m".into()),
        provider: Some("p".into()),
        engine_version: Some("e".into()),
        e2e_revision: Some("r".into()),
        ..EvidenceIdentityV1::default()
    };
    let side = |execution_id: &str| DecisionSideV1 {
        execution_id: execution_id.into(),
        identity: identity.clone(),
        seeds: vec!["case".into()],
        policy_revision: Some("p1".into()),
        expected_revision: None,
        expected_stack: None,
        observed_stack: None,
    };
    let run = |at: usize, cost: f64| EvidenceRunV1 {
        run_id: format!("r{at}"),
        case_id: Some("case".into()),
        completed: true,
        completion: Some("completed".into()),
        technical: Some("valid".into()),
        status: Some("passed".into()),
        cost_usd: Some(cost),
        wall_time_ms: None,
        total_tokens: None,
        function_calls: None,
        criteria: Default::default(),
        signals: None,
    };
    let evidence = vec![EvidenceV1 {
        scenario_id: "s".into(),
        computed_at: 0,
        baseline: EvidenceSideV1 {
            execution_id: "base".into(),
            runs: (0..5).map(|at| run(at, 0.10)).collect(),
            n: 0,
            mean: None,
        },
        candidate: EvidenceSideV1 {
            execution_id: "cand".into(),
            runs: (0..5).map(|at| run(at, 0.05)).collect(),
            n: 0,
            mean: None,
        },
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    }];
    let mut ledger = cost::new_ledger("c", budget(), 0);
    cost::reserve(&mut ledger, &work("a", 0.1, false), 1).unwrap();
    cost::settle(&mut ledger, "a", None, 1).unwrap();
    let record = |plan: &CyclePlanV1| {
        let mut record: CycleRecordV1 = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "cycle_id": "c", "input": {"objective": "cut cost"},
            "status": "running", "stage": "analysis", "step": 1, "created_at": 0,
            "updated_at": 0, "history": []
        }))
        .unwrap();
        decision::freeze(&mut record, plan.clone()).unwrap();
        record
    };
    let mut unknown = record(&plan);
    let decided = decision::decide_cycle(
        &mut unknown,
        Some(&ledger),
        &side("base"),
        &side("cand"),
        &evidence,
        5,
    )
    .unwrap();
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(decided.reason.contains("unknown cost"));
    let mut known = record(&plan);
    let accepted =
        decision::decide_cycle(&mut known, None, &side("base"), &side("cand"), &evidence, 5)
            .unwrap();
    assert_eq!(
        accepted.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        accepted.reason
    );
    // A repeated event never starts another comparison.
    let again =
        decision::decide_cycle(&mut known, None, &side("base"), &side("cand"), &[], 9).unwrap();
    assert_eq!(again, accepted);
}
