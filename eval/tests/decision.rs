//! The cycle decision: a frozen plan, compatibility and preserved correctness
//! as gates of the verdict, and a persisted accepted/rejected/inconclusive
//! outcome with its evidence.

use eval::contract::*;
use eval::decision;
use eval::error::EvalError;

const NOW: i64 = 1_000;

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

/// Cut the cost by at least 1 cent a run, without losing any pass rate.
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

fn record() -> CycleRecordV1 {
    CycleRecordV1 {
        schema_version: CYCLE_SCHEMA_VERSION,
        cycle_id: "cycle-1".into(),
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
    }
}

fn execution(execution_id: &str, harness_version: &str) -> E2eExecutionRefV1 {
    serde_json::from_value(serde_json::json!({
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

/// A compatible pair: only the Harness version differs.
fn pair() -> (DecisionSideV1, DecisionSideV1) {
    let side = |execution_id: &str, harness: &str| DecisionSideV1 {
        execution: execution(execution_id, harness),
        seeds: vec!["seed-2".into(), "seed-1".into()],
        policy_revision: Some("policy-7".into()),
    };
    (side("base", "harness-1"), side("cand", "harness-2"))
}

fn run(status: Option<&str>, cost: Option<f64>) -> EvidenceRunV1 {
    EvidenceRunV1 {
        run_id: "r".into(),
        completed: true,
        completion: Some("completed".into()),
        technical: Some("valid".into()),
        status: status.map(Into::into),
        cost_usd: cost,
        wall_time_ms: None,
        total_tokens: None,
        function_calls: None,
        signals: None,
    }
}

fn broken() -> EvidenceRunV1 {
    EvidenceRunV1 {
        completed: false,
        completion: Some("error".into()),
        technical: Some("infrastructure".into()),
        ..run(Some("failed"), None)
    }
}

/// `count` runs, the first `passed` of them passing, each costing `cost`.
fn runs(count: usize, passed: usize, cost: f64) -> Vec<EvidenceRunV1> {
    (0..count)
        .map(|at| {
            run(
                Some(if at < passed { "passed" } else { "failed" }),
                Some(cost),
            )
        })
        .collect()
}

fn evidence(
    baseline_id: &str,
    baseline: Vec<EvidenceRunV1>,
    candidate_id: &str,
    candidate: Vec<EvidenceRunV1>,
) -> EvidenceV1 {
    let side = |execution_id: &str, runs: Vec<EvidenceRunV1>| EvidenceSideV1 {
        execution_id: execution_id.into(),
        runs,
        n: 0,
        mean: None,
    };
    EvidenceV1 {
        scenario_id: "s".into(),
        computed_at: 0,
        baseline: side(baseline_id, baseline),
        candidate: side(candidate_id, candidate),
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    }
}

/// Cost 0.10 → 0.05 a run with every run passing on both sides.
fn cheaper_and_correct() -> Vec<EvidenceV1> {
    vec![evidence("base", runs(3, 3, 0.10), "cand", runs(3, 3, 0.05))]
}

fn is_conflict(error: &EvalError) -> bool {
    matches!(error, EvalError::Conflict(_))
}

#[test]
fn plan_is_frozen_before_results_with_policy_revision() {
    let (baseline, candidate) = pair();
    // No result without a frozen plan.
    let mut cycle = record();
    let error = decision::decide_cycle(
        &mut cycle,
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, EvalError::InvalidRequest(_)), "{error}");
    assert_eq!(cycle.decision, None);

    // The plan is frozen with its policy_revision.
    decision::freeze(&mut cycle, plan()).unwrap();
    let frozen = cycle.plan.clone().expect("the plan is frozen");
    assert_eq!(frozen.policy_revision, "policy-7");
    assert_eq!(frozen, plan());
    let stored = serde_json::to_value(&cycle).unwrap();
    assert_eq!(stored["plan"]["policy_revision"], "policy-7");
    assert_eq!(stored["plan"]["objective"], "cut the cost of scenario s");

    // Freezing the same plan again, at another time, changes nothing.
    decision::freeze(
        &mut cycle,
        CyclePlanV1 {
            frozen_at: 999,
            ..plan()
        },
    )
    .unwrap();
    assert_eq!(cycle.plan.as_ref(), Some(&frozen));

    // Any other plan is refused: objective, metric, minimum effect, preserved
    // conditions, controls, sampling and policy are all frozen.
    let changes: Vec<(&str, fn(&mut CyclePlanV1))> = vec![
        ("objective", |plan| plan.objective = "something else".into()),
        ("metric", |plan| {
            plan.criterion.metric = CriterionMetricV1::Tokens
        }),
        ("min_effect", |plan| plan.criterion.min_effect = 0.001),
        ("preserved", |plan| plan.preserved.clear()),
        ("controls", |plan| plan.controls.clear()),
        ("sampling", |plan| plan.sampling.runs_per_side = 1),
        ("seeds", |plan| plan.sampling.seeds = vec!["seed-9".into()]),
        ("policy_revision", |plan| {
            plan.policy_revision = "policy-8".into()
        }),
    ];
    for (field, change) in changes {
        let mut other = plan();
        change(&mut other);
        let error = decision::freeze(&mut cycle, other.clone()).unwrap_err();
        assert!(is_conflict(&error), "{field}: {error}");
        assert_eq!(cycle.plan.as_ref(), Some(&frozen), "{field}");
        // A record write that changes or drops the frozen plan is refused.
        let rewritten = CycleRecordV1 {
            plan: Some(other),
            ..cycle.clone()
        };
        let error = decision::check_write(Some(&cycle), &rewritten).unwrap_err();
        assert!(is_conflict(&error), "{field}: {error}");
    }
    let dropped = CycleRecordV1 {
        plan: None,
        ..cycle.clone()
    };
    assert!(is_conflict(
        &decision::check_write(Some(&cycle), &dropped).unwrap_err()
    ));
    // A write that keeps the plan passes.
    let progressed = CycleRecordV1 {
        step: 2,
        updated_at: 50,
        ..cycle.clone()
    };
    decision::check_write(Some(&cycle), &progressed).unwrap();

    // A plan without its policy_revision cannot be frozen.
    let mut fresh = record();
    let error = decision::freeze(
        &mut fresh,
        CyclePlanV1 {
            policy_revision: " ".into(),
            ..plan()
        },
    )
    .unwrap_err();
    assert!(matches!(error, EvalError::InvalidRequest(_)), "{error}");
    assert_eq!(fresh.plan, None);

    // A decision is never recorded without a frozen plan, and once results
    // exist no plan can be frozen after them.
    let decided = decision::decide(&plan(), &baseline, &candidate, &cheaper_and_correct(), NOW);
    let unplanned = CycleRecordV1 {
        decision: Some(decided.clone()),
        ..record()
    };
    assert!(is_conflict(
        &decision::check_write(None, &unplanned).unwrap_err()
    ));
    let mut late = unplanned.clone();
    assert!(is_conflict(
        &decision::freeze(&mut late, plan()).unwrap_err()
    ));
    assert_eq!(late.plan, None);
}

#[test]
fn incompatible_pairs_never_accept() {
    let plan = plan();
    let evidence = cheaper_and_correct();
    // The compatible pair is accepted on this evidence.
    let (baseline, candidate) = pair();
    let accepted = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
    assert_eq!(
        accepted.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        accepted.reason
    );
    assert!(accepted.comparability.comparable);
    for field in ["seeds", "policy_revision", "model", "provider"] {
        assert!(
            accepted
                .comparability
                .checks
                .iter()
                .any(|check| check.field == field && check.matches),
            "{field}"
        );
    }

    // The same evidence on any incompatible pair is never accepted.
    type Change = fn(&mut DecisionSideV1, &mut DecisionSideV1);
    let cases: Vec<(&str, Change)> = vec![
        ("scenarios", |_, candidate| {
            candidate.execution.scenarios[0].scenario_id = "other".into()
        }),
        ("behavior_sha256", |_, candidate| {
            candidate.execution.scenarios[0].behavior_sha256 = Some("behavior-2".into())
        }),
        ("behavior_sha256", |baseline, candidate| {
            baseline.execution.scenarios[0].behavior_sha256 = None;
            candidate.execution.scenarios[0].behavior_sha256 = None;
        }),
        ("contract_fingerprint", |_, candidate| {
            candidate.execution.scenarios[0].contract_fingerprint = Some("contract-2".into())
        }),
        ("contract_fingerprint", |_, candidate| {
            candidate.execution.scenarios[0].contract_fingerprint = None
        }),
        ("contract_fingerprint", |baseline, candidate| {
            baseline.execution.scenarios[0].contract_fingerprint = None;
            candidate.execution.scenarios[0].contract_fingerprint = None;
        }),
        ("model", |_, candidate| {
            candidate.execution.model = Some("model-b".into())
        }),
        ("model", |baseline, candidate| {
            baseline.execution.model = None;
            candidate.execution.model = None;
        }),
        ("provider", |_, candidate| {
            candidate.execution.provider = Some("provider-b".into())
        }),
        ("e2e_revision", |_, candidate| {
            candidate.execution.e2e_revision = Some("e2e-rev-2".into())
        }),
        ("engine_version", |_, candidate| {
            candidate.execution.engine_version = Some("engine-2".into())
        }),
        ("seeds", |_, candidate| {
            candidate.seeds = vec!["seed-1".into(), "seed-3".into()]
        }),
        ("seeds", |baseline, candidate| {
            baseline.seeds = vec!["seed-9".into()];
            candidate.seeds = vec!["seed-9".into()];
        }),
        ("seeds", |baseline, candidate| {
            baseline.seeds.clear();
            candidate.seeds.clear();
        }),
        ("policy_revision", |_, candidate| {
            candidate.policy_revision = Some("policy-8".into())
        }),
        ("policy_revision", |baseline, candidate| {
            baseline.policy_revision = Some("policy-8".into());
            candidate.policy_revision = Some("policy-8".into());
        }),
        ("policy_revision", |baseline, candidate| {
            baseline.policy_revision = None;
            candidate.policy_revision = None;
        }),
    ];
    for (field, change) in cases {
        let (mut baseline, mut candidate) = pair();
        change(&mut baseline, &mut candidate);
        let decided = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
        assert_ne!(decided.outcome, DecisionOutcomeV1::Accepted, "{field}");
        assert_eq!(
            decided.outcome,
            DecisionOutcomeV1::Inconclusive,
            "{field}: {}",
            decided.reason
        );
        assert!(!decided.comparability.comparable, "{field}");
        assert!(
            decided
                .comparability
                .checks
                .iter()
                .any(|check| check.field == field && !check.matches),
            "{field}: {:?}",
            decided.comparability
        );
        assert!(
            decided.reason.contains("not comparable") && decided.reason.contains(field),
            "{field}: {}",
            decided.reason
        );
        // Recorded through the cycle, the decision is not an approval either.
        let mut cycle = record();
        decision::freeze(&mut cycle, plan.clone()).unwrap();
        let recorded =
            decision::decide_cycle(&mut cycle, &baseline, &candidate, &evidence, NOW).unwrap();
        assert_ne!(recorded.outcome, DecisionOutcomeV1::Accepted, "{field}");
    }
}

#[test]
fn cheaper_candidate_losing_correctness_is_rejected() {
    let plan = plan();
    let (baseline, candidate) = pair();
    // Half the cost, but a third of the runs now fail.
    let failing = vec![evidence("base", runs(3, 3, 0.10), "cand", runs(3, 2, 0.05))];
    let decided = decision::decide(&plan, &baseline, &candidate, &failing, NOW);
    // The metric did improve…
    assert_eq!(
        decided.primary.computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement,
        "{}",
        decided.primary.reason
    );
    // …and the lost correctness rejects it anyway.
    assert_eq!(decided.preserved.len(), 1);
    assert_eq!(
        decided.preserved[0].computed_outcome,
        ValidationOutcomeV1::Regression,
        "{}",
        decided.preserved[0].reason
    );
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Rejected,
        "{}",
        decided.reason
    );
    assert!(
        decided
            .reason
            .contains("preserved correctness condition was lost"),
        "{}",
        decided.reason
    );

    // Even a much cheaper candidate that loses every pass is rejected.
    let broke_all = vec![evidence("base", runs(3, 3, 0.50), "cand", runs(3, 0, 0.01))];
    let decided = decision::decide(&plan, &baseline, &candidate, &broke_all, NOW);
    assert_eq!(decided.outcome, DecisionOutcomeV1::Rejected);

    // The same saving with correctness held is accepted.
    let decided = decision::decide(&plan, &baseline, &candidate, &cheaper_and_correct(), NOW);
    assert_eq!(
        decided.preserved[0].computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decided.reason
    );

    // A metric that moved the wrong way, or not enough, is rejected too.
    let pricier = vec![evidence("base", runs(3, 3, 0.05), "cand", runs(3, 3, 0.10))];
    let decided = decision::decide(&plan, &baseline, &candidate, &pricier, NOW);
    assert_eq!(decided.outcome, DecisionOutcomeV1::Rejected);
    assert!(
        decided.reason.starts_with("regression"),
        "{}",
        decided.reason
    );
    let same = vec![evidence("base", runs(3, 3, 0.10), "cand", runs(3, 3, 0.10))];
    let decided = decision::decide(&plan, &baseline, &candidate, &same, NOW);
    assert_eq!(decided.outcome, DecisionOutcomeV1::Rejected);
    assert!(
        decided.reason.starts_with("no improvement"),
        "{}",
        decided.reason
    );
}

#[test]
fn missing_measure_or_technical_failure_is_inconclusive() {
    let plan = plan();
    let (baseline, candidate) = pair();
    let decide = |plan: &CyclePlanV1, evidence: Vec<EvidenceV1>| {
        decision::decide(plan, &baseline, &candidate, &evidence, NOW)
    };
    let inconclusive = |name: &str, decided: CycleDecisionV1, wanted: &str| {
        assert_eq!(
            decided.outcome,
            DecisionOutcomeV1::Inconclusive,
            "{name}: {}",
            decided.reason
        );
        assert!(
            decided.reason.contains(wanted),
            "{name}: {}",
            decided.reason
        );
    };

    // No evidence of the scenario at all.
    inconclusive("no evidence", decide(&plan, Vec::new()), "no run of s");

    // Evidence of another execution is never mixed into the pair.
    inconclusive(
        "other execution",
        decide(
            &plan,
            vec![evidence(
                "base",
                runs(3, 3, 0.10),
                "other",
                runs(3, 3, 0.05),
            )],
        ),
        "no run of s",
    );

    // Every candidate run broke on the infrastructure.
    let broken_runs = vec![broken(), broken(), broken()];
    inconclusive(
        "technical failure",
        decide(
            &plan,
            vec![evidence("base", runs(3, 3, 0.10), "cand", broken_runs)],
        ),
        "did not complete",
    );

    // Technical failures leave too few measured runs.
    let mut mixed = runs(2, 2, 0.05);
    mixed.extend([broken(), broken()]);
    inconclusive(
        "partly broken",
        decide(
            &plan,
            vec![evidence("base", runs(3, 3, 0.10), "cand", mixed)],
        ),
        "the candidate has 2 completed run(s)",
    );

    // The candidate runs carry no cost: the metric is not measured, and an
    // unknown cost is not zero.
    let unpriced: Vec<_> = (0..3).map(|_| run(Some("passed"), None)).collect();
    let decided = decide(
        &plan,
        vec![evidence("base", runs(3, 3, 0.10), "cand", unpriced)],
    );
    assert_eq!(decided.primary.candidate.n, 0);
    assert_eq!(decided.primary.candidate.mean, None);
    inconclusive(
        "no cost",
        decided,
        "the candidate has 0 completed run(s) with cost (USD)",
    );

    // The preserved correctness condition is not measured, though the metric
    // improved.
    let unjudged: Vec<_> = (0..3).map(|_| run(None, Some(0.05))).collect();
    let decided = decide(
        &plan,
        vec![evidence("base", runs(3, 3, 0.10), "cand", unjudged)],
    );
    assert_eq!(
        decided.primary.computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );
    inconclusive("no correctness measure", decided, "not measured");

    // Fewer runs than the frozen sampling plan asks for, though the
    // criterion alone would settle for one.
    let mut lenient = plan.clone();
    lenient.criterion.min_runs = 1;
    lenient.preserved[0].min_runs = 1;
    inconclusive(
        "below the sampling plan",
        decide(
            &lenient,
            vec![evidence("base", runs(2, 2, 0.10), "cand", runs(2, 2, 0.05))],
        ),
        "below the 3",
    );
}

#[test]
fn decision_persists_outcome_and_evidence() {
    let (baseline, candidate) = pair();
    let mut cycle = record();
    decision::freeze(&mut cycle, plan()).unwrap();
    let before = cycle.clone();

    let decided = decision::decide_cycle(
        &mut cycle,
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        NOW,
    )
    .unwrap();
    assert_eq!(cycle.decision.as_ref(), Some(&decided));
    assert_eq!(cycle.updated_at, NOW);
    assert_eq!(cycle.plan, before.plan);
    // The write passes the record guard.
    decision::check_write(Some(&before), &cycle).unwrap();

    // The outcome and the evidence it rests on.
    assert_eq!(decided.outcome, DecisionOutcomeV1::Accepted);
    assert!(!decided.reason.is_empty());
    assert_eq!(decided.policy_revision, "policy-7");
    assert_eq!(decided.decided_at, NOW);
    assert!(decided.comparability.comparable);
    let primary = &decided.primary;
    assert_eq!(primary.scenario_id, "s");
    assert_eq!(
        (
            primary.baseline.execution_id.as_str(),
            primary.candidate.execution_id.as_str()
        ),
        ("base", "cand")
    );
    assert_eq!((primary.baseline.n, primary.candidate.n), (3, 3));
    assert!((primary.baseline.mean.unwrap() - 0.10).abs() < 1e-9);
    assert!((primary.candidate.mean.unwrap() - 0.05).abs() < 1e-9);
    assert_eq!(
        primary.computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );
    assert_eq!(decided.preserved.len(), 1);
    assert_eq!(
        decided.preserved[0].computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );

    // Persisted: the stored record says accepted, with its evidence, and
    // reads back the same.
    let stored = serde_json::to_value(&cycle).unwrap();
    let persisted = &stored["decision"];
    assert_eq!(persisted["outcome"], "accepted");
    assert_eq!(persisted["policy_revision"], "policy-7");
    assert_eq!(persisted["comparability"]["comparable"], true);
    assert_eq!(persisted["primary"]["baseline"]["n"], 3);
    assert_eq!(
        persisted["primary"]["computed_outcome"],
        "validated_improvement"
    );
    assert_eq!(persisted["preserved"].as_array().map(Vec::len), Some(1));
    let read: CycleRecordV1 = serde_json::from_value(stored).unwrap();
    assert_eq!(read, cycle);

    // A repeated event does not start another comparison: the recorded
    // decision stands, whatever the new evidence says.
    let losing = vec![evidence("base", runs(3, 3, 0.10), "cand", runs(3, 0, 0.01))];
    let again =
        decision::decide_cycle(&mut cycle, &baseline, &candidate, &losing, NOW + 50).unwrap();
    assert_eq!(again, decided);
    assert_eq!(cycle.decision.as_ref(), Some(&decided));
    assert_eq!(cycle.updated_at, NOW);

    // A recorded decision is never replaced or dropped by a write.
    let rejected = decision::decide(&plan(), &baseline, &candidate, &losing, NOW + 50);
    assert_eq!(rejected.outcome, DecisionOutcomeV1::Rejected);
    for next in [
        CycleRecordV1 {
            decision: Some(rejected.clone()),
            ..cycle.clone()
        },
        CycleRecordV1 {
            decision: None,
            ..cycle.clone()
        },
    ] {
        assert!(is_conflict(
            &decision::check_write(Some(&cycle), &next).unwrap_err()
        ));
    }
    decision::check_write(Some(&cycle), &cycle).unwrap();

    // Rejected and inconclusive decisions persist the same way.
    let mut lost = record();
    decision::freeze(&mut lost, plan()).unwrap();
    decision::decide_cycle(&mut lost, &baseline, &candidate, &losing, NOW).unwrap();
    let stored = serde_json::to_value(&lost).unwrap();
    assert_eq!(stored["decision"]["outcome"], "rejected");
    assert_eq!(
        stored["decision"]["preserved"][0]["computed_outcome"],
        "regression"
    );
    let mut unknown = record();
    decision::freeze(&mut unknown, plan()).unwrap();
    decision::decide_cycle(&mut unknown, &baseline, &candidate, &[], NOW).unwrap();
    let stored = serde_json::to_value(&unknown).unwrap();
    assert_eq!(stored["decision"]["outcome"], "inconclusive");
    assert!(stored["decision"]["reason"]
        .as_str()
        .is_some_and(|reason| !reason.is_empty()));
    assert_eq!(
        serde_json::from_value::<CycleRecordV1>(stored).unwrap(),
        unknown
    );
}
