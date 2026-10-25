//! The cycle decision: a frozen plan with its frozen coverage, compatibility
//! (subject artifact, non-Harness stack, each side against the frozen tests)
//! and preserved correctness as gates of the verdict, the directional sign
//! test behind every accepted gain, and the native first pass. Adversarial:
//! both arms drifting together, a changed agent on the same model, one lucky
//! pair, a lost control, and the E2E's own failure domains. The captured
//! fixture is a real `execution-get` bundle (see its `_provenance`); every
//! other bundle here is built by hand in the native report's shape.

use eval::contract::*;
use eval::decision;
use eval::error::EvalError;
use eval::evidence;
use serde_json::{json, Value};

const NOW: i64 = 1_000;
const RUNS: u32 = 5;
const CAPTURED: &str = include_str!("fixtures/e2e/captured-tool-contract-recovery.json");
/// `contract_sha256` of tool_contract_recovery in the live `scenarios-list`.
const CATALOG_CONTRACT: &str =
    "sha256:d1c5d96f36b2539400efd583e799a44bca547c4d125c32f1b5336f33a757fa98";

fn criterion(
    metric: CriterionMetricV1,
    direction: DirectionV1,
    min_effect: f64,
    scenario_id: &str,
) -> CriterionV1 {
    CriterionV1 {
        metric,
        pattern: None,
        direction,
        min_effect,
        criterion_id: None,
        min_runs: RUNS,
        scenario_id: scenario_id.into(),
    }
}

fn selected(scenario_id: &str) -> SelectedTestV1 {
    SelectedTestV1 {
        scenario_id: scenario_id.into(),
        case_id: format!("{scenario_id}:seed-1"),
        seed: 1,
        behavior_sha256: format!("behavior-{scenario_id}"),
        contract_sha256: format!("contract-{scenario_id}"),
        inputs_sha256: format!("inputs-{scenario_id}"),
        criteria: vec![TestCriterionV1 {
            id: "done".into(),
            description: "the task is done".into(),
            gate: true,
        }],
        invariants: Vec::new(),
        matched_terms: Vec::new(),
        rationale: "test".into(),
        history_note: None,
    }
}

/// Cut the cost of `s` by at least 1 cent a run over 5 paired runs, without
/// losing the pass rate of `s` or of the control `control-a`.
fn plan() -> CyclePlanV1 {
    CyclePlanV1 {
        objective: "cut the cost of scenario s".into(),
        criterion: criterion(CriterionMetricV1::CostUsd, DirectionV1::Decrease, 0.01, "s"),
        preserved: vec![
            criterion(CriterionMetricV1::PassRate, DirectionV1::Increase, 0.0, "s"),
            criterion(
                CriterionMetricV1::PassRate,
                DirectionV1::Increase,
                0.0,
                "control-a",
            ),
        ],
        controls: vec!["control-a".into()],
        tests: vec![selected("s"), selected("control-a")],
        sampling: SamplingPlanV1 {
            runs_per_side: RUNS,
            seeds: vec!["s:seed-1".into(), "control-a:seed-1".into()],
        },
        parameters: ExecutionParametersV1 {
            provider: "provider-a".into(),
            model: "model-a".into(),
            runs: RUNS,
            scenarios: vec!["s".into(), "control-a".into()],
            technical_retries: 0,
            where_: WhereV1::Harness,
            stack: None,
            agent: None,
        },
        estimated_usd: 1.0,
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
        stage: CycleStageV1::Analysis,
        step: 1,
        created_at: 1,
        updated_at: 1,
        completed_at: None,
        history: Vec::new(),
        coverage: None,
        plan: None,
        executions: Vec::new(),
        analyses: Vec::new(),
        baseline: None,
        next: None,
        decision: None,
        outcome: None,
        failure: None,
        config: None,
        triage: None,
        proposal: None,
        candidate: None,
        artifact_decision: None,
        promotion: None,
        canary: None,
        recovery: None,
        activation_intent: None,
    }
}

/// The identity a report states for a scenario run exactly as frozen.
fn scenario_identity(test: &SelectedTestV1) -> ScenarioIdentityV1 {
    ScenarioIdentityV1 {
        scenario_id: test.scenario_id.clone(),
        case_id: Some(test.case_id.clone()),
        behavior_sha256: Some(test.behavior_sha256.clone()),
        inputs_sha256: Some(test.inputs_sha256.clone()),
        seed: Some(test.seed),
        contract_sha256: Some(test.contract_sha256.clone()),
    }
}

fn identity(harness_version: &str) -> EvidenceIdentityV1 {
    EvidenceIdentityV1 {
        scenarios: plan().tests.iter().map(scenario_identity).collect(),
        contract_digest: Some("contract-1".into()),
        artifacts_digest: Some("artifacts".into()),
        stack_digest: Some("stack".into()),
        subject_sha256: Some("subject-1".into()),
        fixed_stack_digest: Some("fixed-stack-1".into()),
        model: Some("model-a".into()),
        provider: Some("provider-a".into()),
        harness_version: Some(harness_version.into()),
        engine_version: Some("engine-1".into()),
        e2e_revision: Some("e2e-rev-1".into()),
        ..Default::default()
    }
}

/// A compatible pair: only the Harness version differs.
fn pair() -> (DecisionSideV1, DecisionSideV1) {
    let side = |execution_id: &str, harness: &str| DecisionSideV1 {
        execution_id: execution_id.into(),
        identity: identity(harness),
        seeds: vec!["control-a:seed-1".into(), "s:seed-1".into()],
        policy_revision: Some("policy-7".into()),
        expected_revision: None,
        expected_stack: None,
        observed_stack: None,
    };
    (side("base", "harness-1"), side("cand", "harness-2"))
}

fn run(scenario_id: &str, at: usize, status: Option<&str>, cost: Option<f64>) -> EvidenceRunV1 {
    EvidenceRunV1 {
        run_id: format!("{scenario_id}-r{at}"),
        case_id: Some(format!("{scenario_id}:seed-1")),
        completed: true,
        completion: Some("completed".into()),
        technical: Some("valid".into()),
        status: status.map(Into::into),
        cost_usd: cost,
        wall_time_ms: None,
        total_tokens: None,
        function_calls: None,
        criteria: Default::default(),
        signals: None,
    }
}

fn broken(scenario_id: &str, at: usize) -> EvidenceRunV1 {
    EvidenceRunV1 {
        completed: false,
        completion: Some("error".into()),
        technical: Some("technical_invalid".into()),
        ..run(scenario_id, at, Some("infrastructure_error"), None)
    }
}

/// `count` runs of `scenario_id`, the first `passed` passing, each costing `cost`.
fn runs_of(scenario_id: &str, count: usize, passed: usize, cost: f64) -> Vec<EvidenceRunV1> {
    (0..count)
        .map(|at| {
            run(
                scenario_id,
                at,
                Some(if at < passed { "passed" } else { "failed" }),
                Some(cost),
            )
        })
        .collect()
}

fn runs(count: usize, passed: usize, cost: f64) -> Vec<EvidenceRunV1> {
    runs_of("s", count, passed, cost)
}

fn evidence_for(
    scenario_id: &str,
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
        scenario_id: scenario_id.into(),
        computed_at: 0,
        baseline: side(baseline_id, baseline),
        candidate: side(candidate_id, candidate),
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    }
}

/// The control held on both sides.
fn control() -> EvidenceV1 {
    let all = || runs_of("control-a", RUNS as usize, RUNS as usize, 0.10);
    evidence_for("control-a", "base", all(), "cand", all())
}

/// `s`'s evidence plus the held control.
fn with_control(baseline: Vec<EvidenceRunV1>, candidate: Vec<EvidenceRunV1>) -> Vec<EvidenceV1> {
    vec![
        evidence_for("s", "base", baseline, "cand", candidate),
        control(),
    ]
}

/// Cost 0.10 → 0.05 on each of the 5 pairs, every run passing on both sides.
fn cheaper_and_correct() -> Vec<EvidenceV1> {
    with_control(runs(5, 5, 0.10), runs(5, 5, 0.05))
}

fn is_conflict(error: &EvalError) -> bool {
    matches!(error, EvalError::Conflict(_))
}

fn check<'a>(decided: &'a CycleDecisionV1, field: &str) -> &'a ComparabilityCheckV1 {
    decided
        .comparability
        .checks
        .iter()
        .find(|check| check.field == field)
        .unwrap_or_else(|| panic!("no check {field}: {:?}", decided.comparability))
}

fn imported(bundle: Value) -> ExecutionEvidenceV1 {
    let execution_id = bundle["manifest"]["executions"][0]["id"]
        .as_str()
        .unwrap_or("x")
        .to_string();
    ExecutionEvidenceV1 {
        admission_key: format!("admission-{execution_id}"),
        identity: evidence::identity(&bundle, &execution_id),
        execution_id,
        imported_at: 1,
        bundle_digest: "digest".into(),
        bundle,
    }
}

fn captured() -> Value {
    serde_json::from_str(CAPTURED).expect("the captured fixture is JSON")
}

#[test]
fn plan_is_frozen_before_results_with_its_coverage() {
    let (baseline, candidate) = pair();
    let mut cycle = record();
    let error = decision::decide_cycle(
        &mut cycle,
        None,
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, EvalError::InvalidRequest(_)), "{error}");
    assert_eq!(cycle.decision, None);

    decision::freeze(&mut cycle, plan()).unwrap();
    let frozen = cycle.plan.clone().expect("the plan is frozen");
    assert_eq!(frozen, plan());
    let stored = serde_json::to_value(&cycle).unwrap();
    assert_eq!(stored["plan"]["policy_revision"], "policy-7");
    assert_eq!(stored["plan"]["tests"][0]["behavior_sha256"], "behavior-s");
    assert_eq!(stored["plan"]["tests"][1]["scenario_id"], "control-a");
    decision::freeze(
        &mut cycle,
        CyclePlanV1 {
            frozen_at: 999,
            ..plan()
        },
    )
    .unwrap();
    assert_eq!(cycle.plan.as_ref(), Some(&frozen));

    type PlanChange = fn(&mut CyclePlanV1);
    let changes: Vec<(&str, PlanChange)> = vec![
        ("objective", |plan| plan.objective = "something else".into()),
        ("metric", |plan| {
            plan.criterion.metric = CriterionMetricV1::Tokens
        }),
        ("min_effect", |plan| plan.criterion.min_effect = 0.001),
        ("preserved", |plan| plan.preserved.truncate(1)),
        ("controls", |plan| plan.controls.clear()),
        ("sampling", |plan| plan.sampling.runs_per_side = 1),
        ("seeds", |plan| plan.sampling.seeds = vec!["seed-9".into()]),
        ("tests", |plan| {
            plan.tests[0].behavior_sha256 = "behavior-moved".into()
        }),
        ("policy_revision", |plan| {
            plan.policy_revision = "policy-8".into()
        }),
    ];
    for (field, change) in changes {
        let mut other = plan();
        change(&mut other);
        let error = decision::freeze(&mut cycle, other.clone()).unwrap_err();
        assert!(is_conflict(&error), "{field}: {error}");
        let rewritten = CycleRecordV1 {
            plan: Some(other),
            ..cycle.clone()
        };
        assert!(
            is_conflict(&decision::check_write(Some(&cycle), &rewritten).unwrap_err()),
            "{field}"
        );
    }
    let dropped = CycleRecordV1 {
        plan: None,
        ..cycle.clone()
    };
    assert!(is_conflict(
        &decision::check_write(Some(&cycle), &dropped).unwrap_err()
    ));
    decision::check_write(
        Some(&cycle),
        &CycleRecordV1 {
            step: 2,
            ..cycle.clone()
        },
    )
    .unwrap();

    // A plan that judges a scenario without its frozen test, or freezes one
    // twice, or has no policy revision, is refused.
    type Invalid = fn(&mut CyclePlanV1);
    let invalid: Vec<(&str, Invalid)> = vec![
        ("no tests", |plan| plan.tests.clear()),
        ("control not frozen", |plan| plan.tests.truncate(1)),
        ("frozen twice", |plan| plan.tests.push(selected("s"))),
        ("no policy", |plan| plan.policy_revision = " ".into()),
    ];
    for (name, change) in invalid {
        let mut fresh = record();
        let mut other = plan();
        change(&mut other);
        let error = decision::freeze(&mut fresh, other).unwrap_err();
        assert!(
            matches!(error, EvalError::InvalidRequest(_)),
            "{name}: {error}"
        );
        assert_eq!(fresh.plan, None, "{name}");
    }

    let decided = decision::decide(&plan(), &baseline, &candidate, &cheaper_and_correct(), NOW);
    let unplanned = CycleRecordV1 {
        decision: Some(decided),
        ..record()
    };
    assert!(is_conflict(
        &decision::check_write(None, &unplanned).unwrap_err()
    ));
    let mut late = unplanned.clone();
    assert!(is_conflict(
        &decision::freeze(&mut late, plan()).unwrap_err()
    ));
}

#[test]
fn incompatible_pairs_never_accept() {
    let plan = plan();
    let evidence = cheaper_and_correct();
    let (baseline, candidate) = pair();
    let accepted = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
    assert_eq!(
        accepted.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        accepted.reason
    );
    for field in [
        "seeds",
        "policy_revision",
        "model",
        "provider",
        "subject_sha256",
        "fixed_stack_digest",
        "baseline_frozen_coverage",
        "candidate_frozen_coverage",
    ] {
        assert!(check(&accepted, field).matches, "{field}");
    }

    type Change = fn(&mut DecisionSideV1, &mut DecisionSideV1);
    let cases: Vec<(&str, Change)> = vec![
        ("inputs_sha256", |_, candidate| {
            candidate.identity.scenarios[0].inputs_sha256 = Some("inputs-2".into())
        }),
        ("behavior_sha256", |_, candidate| {
            candidate.identity.scenarios[0].behavior_sha256 = Some("behavior-2".into())
        }),
        ("contract_digest", |_, candidate| {
            candidate.identity.contract_digest = None
        }),
        ("subject_sha256", |_, candidate| {
            candidate.identity.subject_sha256 = Some("subject-2".into())
        }),
        ("subject_sha256", |baseline, candidate| {
            baseline.identity.subject_sha256 = None;
            candidate.identity.subject_sha256 = None;
        }),
        ("fixed_stack_digest", |_, candidate| {
            candidate.identity.fixed_stack_digest = Some("fixed-stack-2".into())
        }),
        // Source mode proves no non-Harness runtime identity: unknown on
        // both sides is not equal.
        ("fixed_stack_digest", |baseline, candidate| {
            baseline.identity.fixed_stack_digest = None;
            candidate.identity.fixed_stack_digest = None;
        }),
        ("model", |_, candidate| {
            candidate.identity.model = Some("model-b".into())
        }),
        ("provider", |_, candidate| {
            candidate.identity.provider = Some("provider-b".into())
        }),
        ("e2e_revision", |_, candidate| {
            candidate.identity.e2e_revision = Some("e2e-rev-2".into())
        }),
        ("engine_version", |_, candidate| {
            candidate.identity.engine_version = Some("engine-2".into())
        }),
        ("seeds", |_, candidate| {
            candidate.seeds = vec!["s:seed-1".into()]
        }),
        ("policy_revision", |_, candidate| {
            candidate.policy_revision = Some("policy-8".into())
        }),
        ("candidate_frozen_coverage", |_, candidate| {
            candidate.identity.scenarios[0].seed = Some(2)
        }),
        ("candidate_frozen_coverage", |_, candidate| {
            candidate.identity.scenarios[0].contract_sha256 = None
        }),
    ];
    for (field, change) in cases {
        let (mut baseline, mut candidate) = pair();
        change(&mut baseline, &mut candidate);
        let decided = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
        assert_eq!(
            decided.outcome,
            DecisionOutcomeV1::Inconclusive,
            "{field}: {}",
            decided.reason
        );
        assert!(!check(&decided, field).matches, "{field}");
        assert!(
            decided.reason.contains("not comparable") && decided.reason.contains(field),
            "{field}: {}",
            decided.reason
        );
        let mut cycle = record();
        decision::freeze(&mut cycle, plan.clone()).unwrap();
        let recorded =
            decision::decide_cycle(&mut cycle, None, &baseline, &candidate, &evidence, NOW)
                .unwrap();
        assert_ne!(recorded.outcome, DecisionOutcomeV1::Accepted, "{field}");
    }
}

#[test]
fn both_arms_drifting_together_from_the_frozen_coverage_are_inconclusive() {
    let plan = plan();
    let evidence = cheaper_and_correct();
    // The catalog moved after selection and both arms ran the new
    // definition: they agree with each other, not with what was frozen.
    type Drift = fn(&mut ScenarioIdentityV1);
    let drifts: Vec<(&str, Drift)> = vec![
        ("behavior", |scenario| {
            scenario.behavior_sha256 = Some("behavior-new".into())
        }),
        ("contract", |scenario| {
            scenario.contract_sha256 = Some("contract-new".into())
        }),
        ("inputs", |scenario| {
            scenario.inputs_sha256 = Some("inputs-new".into())
        }),
        ("seed", |scenario| scenario.seed = Some(7)),
        ("case", |scenario| {
            scenario.case_id = Some("s:seed-7".into())
        }),
    ];
    for (what, drift) in drifts {
        let (mut baseline, mut candidate) = pair();
        drift(&mut baseline.identity.scenarios[0]);
        drift(&mut candidate.identity.scenarios[0]);
        let decided = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
        assert_eq!(
            decided.outcome,
            DecisionOutcomeV1::Inconclusive,
            "{what}: {}",
            decided.reason
        );
        assert!(
            !check(&decided, "baseline_frozen_coverage").matches,
            "{what}"
        );
        assert!(
            !check(&decided, "candidate_frozen_coverage").matches,
            "{what}"
        );
        // The pairwise checks alone would have let it through.
        for field in ["behavior_sha256", "inputs_sha256"] {
            assert!(check(&decided, field).matches, "{what}: {field}");
        }
    }
    // Both arms skipped the control, or both ran an extra scenario.
    let (mut baseline, mut candidate) = pair();
    baseline.identity.scenarios.truncate(1);
    candidate.identity.scenarios.truncate(1);
    let decided = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
    assert_eq!(decided.outcome, DecisionOutcomeV1::Inconclusive);
    assert!(!check(&decided, "candidate_frozen_coverage").matches);
    let (mut baseline, mut candidate) = pair();
    let extra = scenario_identity(&selected("extra"));
    baseline.identity.scenarios.push(extra.clone());
    candidate.identity.scenarios.push(extra);
    let decided = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
    assert_eq!(decided.outcome, DecisionOutcomeV1::Inconclusive);
    assert!(!check(&decided, "baseline_frozen_coverage").matches);
}

#[test]
fn a_lucky_single_pair_never_becomes_an_approval() {
    let (baseline, candidate) = pair();
    // n=1 per side under a plan that samples one run: the mean moved far
    // more than the minimum, but one pair has p = 0.5.
    let mut single = plan();
    single.sampling.runs_per_side = 1;
    single.criterion.min_runs = 1;
    for preserved in &mut single.preserved {
        preserved.min_runs = 1;
    }
    let lucky = vec![
        evidence_for("s", "base", runs(1, 1, 0.50), "cand", runs(1, 1, 0.01)),
        evidence_for(
            "control-a",
            "base",
            runs_of("control-a", 1, 1, 0.1),
            "cand",
            runs_of("control-a", 1, 1, 0.1),
        ),
    ];
    let decided = decision::decide(&single, &baseline, &candidate, &lucky, NOW);
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(
        decided.reason.contains("1 of 1") && decided.reason.contains("p=0.50000"),
        "{}",
        decided.reason
    );
    let mut cycle = record();
    decision::freeze(&mut cycle, single.clone()).unwrap();
    let recorded =
        decision::decide_cycle(&mut cycle, None, &baseline, &candidate, &lucky, NOW).unwrap();
    assert_ne!(recorded.outcome, DecisionOutcomeV1::Accepted);
    // So the artifact is never judged on it either.
    let error = decision::decide_artifact(&mut cycle, None, &baseline, &candidate, &lucky, NOW)
        .unwrap_err();
    assert!(is_conflict(&error), "{error}");

    // Five consistent pairs pass at p = 0.03125.
    let decided = decision::decide(&plan(), &baseline, &candidate, &cheaper_and_correct(), NOW);
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decided.reason
    );
    assert!(
        decided.reason.contains("5 of 5") && decided.reason.contains("p=0.03125"),
        "{}",
        decided.reason
    );

    // Four of five pairs carry the mean: p = 0.1875, not accepted.
    let mut cheaper = runs(5, 5, 0.04);
    cheaper[4].cost_usd = Some(0.10);
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.10), cheaper),
        NOW,
    );
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(decided.reason.contains("4 of 5"), "{}", decided.reason);

    // A raw gain under a primary with no positive minimum effect is not approved.
    let mut zero = plan();
    zero.criterion.min_effect = 0.0;
    let decided = decision::decide(&zero, &baseline, &candidate, &cheaper_and_correct(), NOW);
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(
        decided.reason.contains("no positive minimum effect"),
        "{}",
        decided.reason
    );
}

#[test]
fn the_sample_must_be_exactly_the_frozen_one() {
    let (baseline, candidate) = pair();
    // An extra candidate run (a repeat-until-win) is never silently accepted.
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.10), runs(6, 6, 0.05)),
        NOW,
    );
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(decided.reason.contains("exactly 5"), "{}", decided.reason);
    // A duplicated run is not two samples.
    let mut duplicated = runs(5, 5, 0.05);
    duplicated[4].run_id = "s-r0".into();
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.10), duplicated),
        NOW,
    );
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(
        decided.reason.contains("more than once"),
        "{}",
        decided.reason
    );
    // Unfinished cheaper runs are never an effort gain.
    let gave_up: Vec<_> = (0..5)
        .map(|at| EvidenceRunV1 {
            completed: false,
            completion: Some("task_incomplete".into()),
            ..run("s", at, Some("passed"), Some(0.01))
        })
        .collect();
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.10), gave_up),
        NOW,
    );
    assert_ne!(
        decided.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decided.reason
    );
    // Unchanged pass rate with a zero threshold is held, not a gain.
    let decided = decision::decide(&plan(), &baseline, &candidate, &cheaper_and_correct(), NOW);
    assert_eq!(
        decided.preserved[0].computed_outcome,
        ValidationOutcomeV1::NoImprovement,
        "{}",
        decided.preserved[0].reason
    );
}

#[test]
fn a_lost_control_rejects_whatever_the_metric_did() {
    let (baseline, candidate) = pair();
    for passed in [4, 0] {
        let evidence = vec![
            evidence_for("s", "base", runs(5, 5, 0.10), "cand", runs(5, 5, 0.05)),
            evidence_for(
                "control-a",
                "base",
                runs_of("control-a", 5, 5, 0.10),
                "cand",
                runs_of("control-a", 5, passed, 0.10),
            ),
        ];
        let decided = decision::decide(&plan(), &baseline, &candidate, &evidence, NOW);
        assert_eq!(
            decided.primary.computed_outcome,
            ValidationOutcomeV1::ValidatedImprovement,
            "{}",
            decided.primary.reason
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
        assert_eq!(decided.preserved[1].scenario_id, "control-a");
        assert_eq!(
            decided.preserved[1].computed_outcome,
            ValidationOutcomeV1::Regression
        );
    }
    // A control that was never run is unknown, never held.
    let unrun = vec![evidence_for(
        "s",
        "base",
        runs(5, 5, 0.10),
        "cand",
        runs(5, 5, 0.05),
    )];
    let decided = decision::decide(&plan(), &baseline, &candidate, &unrun, NOW);
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    // A control short of the frozen n is never held, whatever it shows.
    let evidence = vec![
        evidence_for("s", "base", runs(5, 5, 0.10), "cand", runs(5, 5, 0.05)),
        evidence_for(
            "control-a",
            "base",
            runs_of("control-a", 5, 5, 0.1),
            "cand",
            runs_of("control-a", 4, 0, 0.1),
        ),
    ];
    let decided = decision::decide(&plan(), &baseline, &candidate, &evidence, NOW);
    assert_ne!(
        decided.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decided.reason
    );
}

#[test]
fn cheaper_candidate_losing_correctness_is_rejected() {
    let (baseline, candidate) = pair();
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.10), runs(5, 4, 0.05)),
        NOW,
    );
    assert_eq!(
        decided.preserved[0].computed_outcome,
        ValidationOutcomeV1::Regression
    );
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Rejected,
        "{}",
        decided.reason
    );
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.05), runs(5, 5, 0.10)),
        NOW,
    );
    assert!(
        decided.reason.starts_with("regression"),
        "{}",
        decided.reason
    );
    let decided = decision::decide(
        &plan(),
        &baseline,
        &candidate,
        &with_control(runs(5, 5, 0.10), runs(5, 5, 0.10)),
        NOW,
    );
    assert!(
        decided.reason.starts_with("no improvement"),
        "{}",
        decided.reason
    );
}

#[test]
fn missing_measure_or_technical_failure_is_inconclusive() {
    let (baseline, candidate) = pair();
    let decide = |evidence: Vec<EvidenceV1>| {
        decision::decide(&plan(), &baseline, &candidate, &evidence, NOW)
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
    inconclusive("no evidence", decide(Vec::new()), "no run of s");
    inconclusive(
        "other execution",
        decide(vec![
            evidence_for("s", "base", runs(5, 5, 0.10), "other", runs(5, 5, 0.05)),
            control(),
        ]),
        "no run of s",
    );
    let all_broken: Vec<_> = (0..5).map(|at| broken("s", at)).collect();
    inconclusive(
        "technical failure",
        decide(with_control(runs(5, 5, 0.10), all_broken)),
        "failed technically",
    );
    let unpriced: Vec<_> = (0..5)
        .map(|at| run("s", at, Some("passed"), None))
        .collect();
    inconclusive(
        "no cost",
        decide(with_control(runs(5, 5, 0.10), unpriced)),
        "the candidate has 0 completed run(s) with cost (USD)",
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
        None,
        &baseline,
        &candidate,
        &cheaper_and_correct(),
        NOW,
    )
    .unwrap();
    decision::check_write(Some(&before), &cycle).unwrap();
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decided.reason
    );
    assert_eq!(
        (decided.primary.baseline.n, decided.primary.candidate.n),
        (5, 5)
    );
    let stored = serde_json::to_value(&cycle).unwrap();
    assert_eq!(stored["decision"]["outcome"], "accepted");
    let read: CycleRecordV1 = serde_json::from_value(stored).unwrap();
    assert_eq!(read, cycle);
    let losing = with_control(runs(5, 5, 0.10), runs(5, 0, 0.01));
    let again =
        decision::decide_cycle(&mut cycle, None, &baseline, &candidate, &losing, NOW + 50).unwrap();
    assert_eq!(again, decided);
    let rejected = decision::decide(&plan(), &baseline, &candidate, &losing, NOW + 50);
    assert_eq!(rejected.outcome, DecisionOutcomeV1::Rejected);
    let replaced = CycleRecordV1 {
        decision: Some(rejected),
        ..cycle.clone()
    };
    assert!(is_conflict(
        &decision::check_write(Some(&cycle), &replaced).unwrap_err()
    ));
}

/// A native report in the E2E's shape: one scenario with `runs`, a subject
/// and a Registry or Source stack.
fn native(execution_id: &str, subject: Value, stack: Value, runs: Vec<Value>) -> Value {
    json!({
        "manifest": {"executions": [{"id": execution_id, "status": "passed", "parameters": {}}]},
        "detail": {"reports": [{"scenario_id": "s", "available": true, "report": {
            "result_contract_sha256": "sha256:result",
            "subject": subject,
            "system_under_test": {
                "stack": stack, "engine_version": "0.25.0", "harness_version": "1.9.0",
                "e2e_repository": "iii-hq/harness-e2e", "e2e_revision": "e2e-rev",
                "contract_hashes": {"harness::send": "sha256:send"}
            },
            "scenarios": [{"scenario_id": "s", "case_id": "s:seed-1", "behavior_sha256": "sha256:b",
                "case": {"scenario_id": "s", "case_id": "s:seed-1", "behavior_sha256": "sha256:b", "seed": 1,
                    "inputs": {}, "inputs_sha256": "sha256:i", "required_capabilities": [], "deliverable_contract": {}},
                "execution_policy": {"max_turns": 4},
                "runs": runs}]
        }}]}
    })
}

fn subject(agent: Option<(&str, &str)>) -> Value {
    let mut subject = json!({"model": "model-a", "provider": "provider-a", "context_window": 1000,
        "max_output_tokens": 100, "supports_tools": true, "supports_vision": false});
    if let Some((id, configuration)) = agent {
        subject["agent"] = json!({"id": id, "configuration_sha256": configuration});
    }
    subject
}

fn registry(harness: &str, state: &str) -> Value {
    json!({"mode": "registry", "stack_versions": {"harness": harness, "state": state, "shell": "1.0.0"},
        "stack_lock_digest": "sha256:lock"})
}

fn native_run(id: &str, status: &str, technical: &str, failures: Value, criteria: Value) -> Value {
    json!({"run_id": id, "attempt_number": 1, "status": status, "completion": "completed",
        "technical": technical, "criteria": criteria, "failures": failures,
        "cost": {"total_usd": 0.01}, "transcript": {"messages": []}})
}

fn full() -> Value {
    json!([{"id": "done", "possible": 10, "awarded": 10, "gate": true, "reason": "ok"}])
}

#[test]
fn a_changed_agent_on_the_same_model_is_not_comparable() {
    let run = || vec![native_run("r", "passed", "valid", json!([]), full())];
    let base = imported(native(
        "b",
        subject(Some(("coder", "sha256:cfg-1"))),
        registry("1.9.0", "2.0.0"),
        run(),
    ));
    let same = imported(native(
        "c",
        subject(Some(("coder", "sha256:cfg-1"))),
        registry("1.9.1", "2.0.0"),
        run(),
    ));
    let changed = imported(native(
        "d",
        subject(Some(("coder", "sha256:cfg-2"))),
        registry("1.9.1", "2.0.0"),
        run(),
    ));
    let other_agent = imported(native(
        "e",
        subject(Some(("reviewer", "sha256:cfg-1"))),
        registry("1.9.1", "2.0.0"),
        run(),
    ));
    assert!(base.identity.subject_sha256.is_some());
    // Model and provider stay the same; only the agent profile differs.
    assert_eq!(base.identity.model, changed.identity.model);
    assert_eq!(base.identity.provider, changed.identity.provider);
    assert_eq!(base.identity.subject_sha256, same.identity.subject_sha256);
    assert_ne!(
        base.identity.subject_sha256,
        changed.identity.subject_sha256
    );
    assert_ne!(
        base.identity.subject_sha256,
        other_agent.identity.subject_sha256
    );
    // An incomplete subject or an agent without its configuration digest is unknown.
    let mut partial = subject(None);
    partial.as_object_mut().unwrap().remove("context_window");
    assert_eq!(
        imported(native("f", partial, registry("1.9.0", "2.0.0"), run()))
            .identity
            .subject_sha256,
        None
    );
    let mut agentless = subject(None);
    agentless["agent"] = json!({"id": "coder"});
    assert_eq!(
        imported(native("g", agentless, registry("1.9.0", "2.0.0"), run()))
            .identity
            .subject_sha256,
        None
    );

    // Through the decision: the same evidence, the same model, another agent.
    let (baseline, mut candidate) = pair();
    candidate.identity.subject_sha256 = Some("subject-other-agent".into());
    let decided = decision::decide(&plan(), &baseline, &candidate, &cheaper_and_correct(), NOW);
    assert_eq!(
        decided.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decided.reason
    );
    assert!(check(&decided, "model").matches && check(&decided, "provider").matches);
    assert!(!check(&decided, "subject_sha256").matches);
}

#[test]
fn the_fixed_stack_is_every_registry_component_but_the_harness() {
    let run = || vec![native_run("r", "passed", "valid", json!([]), full())];
    let fixed = |stack: Value| {
        imported(native("x", subject(None), stack, run()))
            .identity
            .fixed_stack_digest
    };
    // Only the Harness differs: the fixed stack is the same.
    let base = fixed(registry("1.9.0", "2.0.0"));
    assert!(base.is_some());
    assert_eq!(base, fixed(registry("@abc1234", "2.0.0")));
    // Any other component differing is another stack.
    assert_ne!(base, fixed(registry("1.9.0", "2.0.1")));
    // A Registry identity without the Harness entry proves nothing about what
    // was removed; a Source identity proves no other runtime identity.
    assert_eq!(
        fixed(
            json!({"mode": "registry", "stack_versions": {"state": "2.0.0"}, "stack_lock_digest": "sha256:l"})
        ),
        None
    );
    assert_eq!(
        fixed(
            json!({"mode": "source", "workers_repository": "iii-hq/workers", "workers_revision": "a".repeat(40)})
        ),
        None
    );
}

#[test]
fn the_first_pass_follows_the_native_failure_domain_first() {
    use FindingClassV1::*;
    let failure = |domain: &str, status: &str, code: &str| json!([{"status": status, "domain": domain, "phase": "execute", "code": code, "message": "m"}]);
    // (case, status, technical, failures, criteria, the run_outcome class)
    let table = vec![
        (
            "subject error on a technically invalid run stays the subject's",
            "subject_error",
            "technical_invalid",
            failure("subject", "subject_error", "tool_loop"),
            json!([]),
            Some(Defect),
        ),
        (
            "subject domain recorded on a hard-gate run",
            "hard_gate_failed",
            "valid",
            failure("subject", "subject_error", "bad_answer"),
            full(),
            Some(Defect),
        ),
        (
            "explicit E2E infrastructure is the infrastructure's even if valid",
            "infrastructure_error",
            "valid",
            failure("e2e_infrastructure", "infrastructure_error", "engine_down"),
            full(),
            Some(InfrastructureOrEvaluatorFailure),
        ),
        (
            "resource exhaustion is a resource finding, never a defect",
            "resource_limit",
            "valid",
            failure("resource", "resource_limit", "max_total_tokens"),
            full(),
            Some(InsufficientEvidence),
        ),
        (
            "subject budget exhausted on an invalid run is still resource",
            "resource_limit",
            "technical_invalid",
            failure("resource", "resource_limit", "max_turns"),
            json!([]),
            Some(InsufficientEvidence),
        ),
        (
            "no failure record: technical_invalid is infrastructure",
            "hard_gate_failed",
            "technical_invalid",
            json!([]),
            json!([]),
            Some(InfrastructureOrEvaluatorFailure),
        ),
        (
            "no failure record: a valid hard-gate failure is a defect",
            "hard_gate_failed",
            "valid",
            json!([]),
            full(),
            Some(Defect),
        ),
        (
            "no failure record: subject_error status names the domain",
            "subject_error",
            "technical_invalid",
            json!([]),
            json!([]),
            Some(Defect),
        ),
        (
            "passed and fully awarded holds",
            "passed",
            "valid",
            json!([]),
            full(),
            None,
        ),
    ];
    for (case, status, technical, failures, criteria, wanted) in table {
        let evidence = imported(native(
            "x",
            subject(None),
            registry("1.9.0", "2.0.0"),
            vec![native_run("r", status, technical, failures, criteria)],
        ));
        let (findings, _) = evidence::first_pass(&evidence);
        let outcome = findings
            .iter()
            .find(|finding| finding.signal == "run_outcome")
            .map(|finding| finding.class);
        assert_eq!(outcome, wanted, "{case}: {findings:?}");
        if wanted == Some(InsufficientEvidence) {
            assert!(
                findings.iter().all(|finding| finding.class != Defect),
                "{case}: {findings:?}"
            );
        }
    }
    // A criterion the evaluator did not evaluate on a valid run is an
    // evaluation gap, never a Harness defect; a score loss is no runtime error.
    let unevaluated = json!([{"id": "done", "possible": 10, "awarded": null, "gate": true, "reason": "judge unavailable"}]);
    let evidence = imported(native(
        "x",
        subject(None),
        registry("1.9.0", "2.0.0"),
        vec![native_run("r", "passed", "valid", json!([]), unevaluated)],
    ));
    let (findings, _) = evidence::first_pass(&evidence);
    assert!(
        findings
            .iter()
            .any(|finding| finding.class == EvaluationGap
                && finding.signal == "criterion_not_evaluated"),
        "{findings:?}"
    );
    assert!(
        findings.iter().all(|finding| finding.class != Defect),
        "{findings:?}"
    );
    let lost =
        json!([{"id": "done", "possible": 10, "awarded": 4, "gate": false, "reason": "partial"}]);
    let evidence = imported(native(
        "x",
        subject(None),
        registry("1.9.0", "2.0.0"),
        vec![native_run("r", "passed", "valid", json!([]), lost)],
    ));
    assert!(evidence::sentinel_observations(&evidence, "baseline", "rev", 1).is_empty());
}

#[test]
fn the_captured_bundle_binds_to_the_catalog_identity() {
    let evidence = imported(captured());
    let identity = &evidence.identity;
    assert_eq!(identity.scenarios.len(), 1);
    let scenario = &identity.scenarios[0];
    assert_eq!(scenario.scenario_id, "tool_contract_recovery");
    assert_eq!(
        scenario.case_id.as_deref(),
        Some("tool_contract_recovery:seed-746f6f6c00000001")
    );
    assert_eq!(scenario.seed, Some(8390047140461674497));
    // The E2E's own contract digest, recomputed from the native case and
    // policy, is the live catalog's contract_sha256 for this definition.
    assert_eq!(scenario.contract_sha256.as_deref(), Some(CATALOG_CONTRACT));
    // The run was a Source-mode stack: no non-Harness identity is proven.
    assert_eq!(identity.fixed_stack_digest, None);
    assert!(identity.subject_sha256.is_some());
    let (findings, unknowns) = evidence::first_pass(&evidence);
    assert!(
        unknowns
            .iter()
            .any(|unknown| unknown.contains("fixed stack digest")),
        "{unknowns:?}"
    );
    assert!(
        findings.iter().all(|finding| !finding.class.is_pertinent()),
        "{findings:?}"
    );

    // A selection frozen from that catalog row matches this side; a moved
    // definition (the catalog changed after selection) does not.
    let frozen = SelectedTestV1 {
        scenario_id: "tool_contract_recovery".into(),
        case_id: scenario.case_id.clone().unwrap(),
        seed: 8390047140461674497,
        behavior_sha256: scenario.behavior_sha256.clone().unwrap(),
        contract_sha256: CATALOG_CONTRACT.into(),
        inputs_sha256: scenario.inputs_sha256.clone().unwrap(),
        ..selected("tool_contract_recovery")
    };
    let mut plan = plan();
    plan.criterion.scenario_id = frozen.scenario_id.clone();
    plan.preserved.clear();
    plan.controls.clear();
    plan.tests = vec![frozen.clone()];
    let side = |identity: EvidenceIdentityV1| DecisionSideV1 {
        execution_id: evidence.execution_id.clone(),
        identity,
        seeds: Vec::new(),
        policy_revision: None,
        expected_revision: None,
        expected_stack: None,
        observed_stack: None,
    };
    let decided = decision::decide(
        &plan,
        &side(identity.clone()),
        &side(identity.clone()),
        &[],
        NOW,
    );
    assert!(
        check(&decided, "baseline_frozen_coverage").matches,
        "{:?}",
        decided.comparability
    );
    // Source mode keeps the pair incomparable however else it matches.
    assert!(!check(&decided, "fixed_stack_digest").matches);
    plan.tests[0].contract_sha256 = "sha256:moved".into();
    let decided = decision::decide(
        &plan,
        &side(identity.clone()),
        &side(identity.clone()),
        &[],
        NOW,
    );
    assert!(!check(&decided, "baseline_frozen_coverage").matches);
    assert!(!check(&decided, "candidate_frozen_coverage").matches);
    // An edited case no longer reproduces the catalog digest.
    let mut edited = captured();
    edited["detail"]["reports"][0]["report"]["scenarios"][0]["case"]["deliverable_contract"] =
        json!({"edited": true});
    let edited = imported(edited);
    assert_ne!(
        edited.identity.scenarios[0].contract_sha256.as_deref(),
        Some(CATALOG_CONTRACT)
    );
}

#[test]
fn source_identity_needs_the_full_commit_bound_to_the_observed_bytes() {
    let mut bundle: Value = serde_json::from_str(CAPTURED).unwrap();
    let source = json!({
        "repository": "iii-hq/workers", "path": "harness",
        "revision": "a".repeat(40), "binary_sha256": format!("sha256:{}", "b".repeat(64))
    });
    let sut = &mut bundle["detail"]["reports"][0]["report"]["system_under_test"];
    sut["harness_source"] = source.clone();
    sut["harness_binary_sha256"] = source["binary_sha256"].clone();
    let revision = |bundle: &Value| evidence::identity(bundle, "captured").workers_revision;
    assert_eq!(revision(&bundle), Some("a".repeat(40)));
    let sut = "/detail/reports/0/report/system_under_test";
    let mut same_short = bundle.clone();
    same_short.pointer_mut(sut).unwrap()["harness_source"]["revision"] =
        json!(format!("{}{}", "a".repeat(7), "c".repeat(33)));
    assert_ne!(revision(&bundle), revision(&same_short));
    for bad in [json!("abcdefg"), json!(null)] {
        let mut edited = bundle.clone();
        edited.pointer_mut(sut).unwrap()["harness_source"]["revision"] = bad;
        assert_eq!(revision(&edited), None);
    }
    for bad in [
        json!("sha256:unknown"),
        json!(format!("sha256:{}", "F".repeat(64))),
        json!(format!("sha256:{}", "d".repeat(64))),
    ] {
        let mut edited = bundle.clone();
        edited.pointer_mut(sut).unwrap()["harness_source"]["binary_sha256"] = bad;
        assert_eq!(revision(&edited), None);
    }
    bundle
        .pointer_mut(sut)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("harness_source");
    assert_eq!(
        revision(&bundle),
        None,
        "legacy requested stack SHA is not proof"
    );
}

#[test]
fn a_candidate_cannot_qualify_by_changing_its_own_meter() {
    let (mut baseline, mut candidate) = pair();
    baseline.expected_revision = Some("a".repeat(40));
    baseline.identity.workers_revision = baseline.expected_revision.clone();
    candidate.expected_revision = Some("b".repeat(40));
    candidate.identity.workers_revision = candidate.expected_revision.clone();
    let native_side = |changed: bool| {
        native("meter", subject(None), registry("1.0.0", "2.0.0"), (0..RUNS).map(|at| {
            let mut run = native_run(&format!("s-r{at}"), "passed", "valid", json!([]), full());
            run["metrics"] = json!({"by_session": [{"input_tokens":1000, "output_tokens":100, "cache_read_tokens":12000, "cache_write_tokens":2000, "context":{"total":13000}}]});
            run["efficiency"] = json!({"total_tokens":if changed {0} else {1100}});
            run
        }).collect())
    };
    let before = native_side(false);
    let after = native_side(true);
    assert_eq!(
        before["detail"]["reports"][0]["report"]["scenarios"][0]["runs"][0]["metrics"],
        after["detail"]["reports"][0]["report"]["scenarios"][0]["runs"][0]["metrics"],
        "only the reported meter changed"
    );
    let mut plan = plan();
    plan.criterion.metric = CriterionMetricV1::Tokens;
    plan.criterion.min_effect = 100.0;
    let evidence = with_control(
        evidence::runs_of(&before, "s", &[]),
        evidence::runs_of(&after, "s", &[]),
    );
    let result = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
    assert_eq!(
        result.primary.computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement,
        "the old math alone would accept: {}",
        result.primary.reason
    );
    assert_eq!(result.outcome, DecisionOutcomeV1::Inconclusive);
    assert!(
        result.reason.contains("independent meter"),
        "{}",
        result.reason
    );
    for metric in [CriterionMetricV1::CostUsd, CriterionMetricV1::FunctionCalls] {
        plan.criterion.metric = metric;
        plan.criterion.min_effect = 0.001;
        let mut evidence = cheaper_and_correct();
        if metric == CriterionMetricV1::FunctionCalls {
            for run in &mut evidence[0].baseline.runs {
                run.function_calls = Some(10.0);
            }
            for run in &mut evidence[0].candidate.runs {
                run.function_calls = Some(0.0);
            }
        }
        let result = decision::decide(&plan, &baseline, &candidate, &evidence, NOW);
        assert_eq!(
            result.outcome,
            DecisionOutcomeV1::Inconclusive,
            "{}",
            result.reason
        );
    }
}

#[test]
fn a_current_native_context_test_is_coverage_before_its_first_run() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/e2e/catalog-context-pressure.json")).unwrap();
    let coverage = eval::coverage::select(
        "reduce context consumption",
        fixture["rows"].as_array().unwrap(),
        fixture["revision"].as_str().unwrap(),
        &fixture["catalog"],
        1,
        &[],
        NOW,
    );
    assert!(coverage.covered, "{:?}", coverage.gaps);
    assert_eq!(coverage.selected[0].scenario_id, "context_pressure");
    assert_eq!(
        coverage.selected[0].contract_sha256,
        fixture["catalog"]["scenarios"][0]["contract_sha256"]
    );
}

#[test]
fn configured_controls_count_toward_the_scenario_limit() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/e2e/catalog-context-pressure.json")).unwrap();
    for controls in [vec!["minimal_path".into()], vec!["form_flow_build".into()]] {
        let coverage = eval::coverage::select(
            "reduce context consumption",
            fixture["rows"].as_array().unwrap(),
            fixture["revision"].as_str().unwrap(),
            &fixture["catalog"],
            2,
            &controls,
            NOW,
        );
        assert!(coverage.covered, "{:?}", coverage.gaps);
        assert_eq!(coverage.selected[0].scenario_id, "context_pressure");
        assert_eq!(coverage.selected.len() + coverage.controls.len(), 2);
        assert_eq!(coverage.controls[0].scenario_id, controls[0]);
    }
    let coverage = eval::coverage::select(
        "reduce context consumption",
        fixture["rows"].as_array().unwrap(),
        fixture["revision"].as_str().unwrap(),
        &fixture["catalog"],
        1,
        &["minimal_path".into()],
        NOW,
    );
    assert!(!coverage.covered);
    assert!(coverage.selected.is_empty());
    assert!(coverage.gaps.iter().any(|gap| gap.contains("no primary")));
}

/// The canary's side as it runs on the activated target: the proved
/// executable, on the target's own Source-mode stack, E2E and engine.
fn on_target(binary: &str) -> DecisionSideV1 {
    let (_, mut side) = pair();
    side.identity.fixed_stack_digest = None;
    side.identity.e2e_revision = Some("e2e-on-target".into());
    side.identity.engine_version = Some("engine-on-target".into());
    side.identity.harness_binary_sha256 = Some(format!("sha256:{binary}"));
    side
}

fn canary_analysis() -> CycleAnalysisV1 {
    CycleAnalysisV1 {
        execution_id: "cand".into(),
        admission_key: "admission-cand".into(),
        role: ExecutionRoleV1::Canary,
        identity: Default::default(),
        findings: Vec::new(),
        unknowns: Vec::new(),
        child_transcripts_complete: false,
        sentinel: None,
        scenarios: Vec::new(),
        analyzed_at: NOW,
    }
}

#[test]
fn a_canary_on_the_targets_own_stack_holds_and_any_loss_violates() {
    let binary = "a".repeat(64);
    let (base, _) = pair();
    let judge = |evidence: &[EvidenceV1]| {
        eval::release::canary(
            &plan(),
            &base,
            &on_target(&binary),
            evidence,
            &canary_analysis(),
            Some(&binary),
            NOW,
        )
    };
    let held = judge(&cheaper_and_correct());
    assert_eq!(held.outcome, CanaryOutcomeV1::Held, "{}", held.reason);
    // Half a cent dearer: under the 1 cent minimum effect, and still a loss.
    let dearer = judge(&with_control(runs(5, 5, 0.10), runs(5, 5, 0.105)));
    assert_eq!(
        dearer.outcome,
        CanaryOutcomeV1::Violated,
        "{}",
        dearer.reason
    );
}

#[test]
fn an_unfinished_majority_is_a_loss_not_an_unmeasured_side() {
    let unfinished = |at| EvidenceRunV1 {
        completed: false,
        completion: Some("task_incomplete".into()),
        ..run("s", at, Some("passed"), Some(0.10))
    };
    let mut gave_up = runs(2, 2, 0.10);
    gave_up.extend((2..5).map(unfinished));
    let mut evidence = evidence_for("s", "base", runs(5, 5, 0.10), "cand", gave_up);
    let pass_rate = criterion(CriterionMetricV1::PassRate, DirectionV1::Increase, 0.0, "s");
    decision::judge(&mut evidence, Some(&pass_rate));
    assert_eq!(
        evidence.computed_outcome,
        ValidationOutcomeV1::Regression,
        "{}",
        evidence.reason
    );
}

#[test]
fn a_retried_slot_did_not_complete_and_its_attempts_failures_reach_sentinel() {
    let mut run = native_run("r", "passed", "valid", json!([]), full());
    run["attempt_number"] = json!(2);
    run["retry_attempts"] = json!([{"attempt_number": 1, "session_id": "session-1",
        "status": "subject_error", "failures": [{"domain": "subject", "phase": "turn",
        "code": "connection_closed", "message": "connection closed"}]}]);
    let bundle = native("x", subject(None), registry("1.9.0", "1.0.0"), vec![run]);
    assert!(!evidence::runs_of(&bundle, "s", &[])[0].completed);
    let observations = evidence::sentinel_observations(&imported(bundle), "baseline", "rev", NOW);
    assert_eq!(observations.len(), 1, "{observations:?}");
    assert_eq!(observations[0]["attempt"], 1);
    assert_eq!(observations[0]["session_id"], "session-1");
    assert!(observations[0]["artifact"]
        .as_str()
        .unwrap()
        .ends_with("/runs/r/retry_attempts/0/failures"));
}

#[test]
fn need_words_only_in_a_prompts_code_or_paths_are_no_coverage() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/e2e/catalog-context-pressure.json")).unwrap();
    // form_flow_build's prompt holds `context: { canvasId }` and "In-memory
    // state" in code: not a context-load test.
    let coverage = eval::coverage::select(
        "reduce context consumption",
        fixture["rows"].as_array().unwrap(),
        fixture["revision"].as_str().unwrap(),
        &fixture["catalog"],
        2,
        &[],
        NOW,
    );
    let selected: Vec<&str> = coverage
        .selected
        .iter()
        .map(|test| test.scenario_id.as_str())
        .collect();
    assert_eq!(selected, ["context_pressure"]);
}
