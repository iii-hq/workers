/// Logical end-to-end test over the pure orchestration seams.
///
/// No live engine. No III mock. Drives `dag::*`, `tick::decide`, and
/// `reconcile::classify_terminal` directly over an in-memory
/// `WorkflowRunRecord` + `BTreeMap<String, Value>` results map, simulating
/// the tick loop.
use std::collections::BTreeMap;

use serde_json::{json, Value};
use workflow::{
    dag,
    functions::tick::{decide, TickDecision},
    reconcile::{classify_terminal, NodeOutcome},
    types::{
        AgentSpec, FanoutSpec, InputSpec, NodeCheckpoint, NodeDef, NodeState, OutputRef, RunStatus,
        WorkflowDef, WorkflowRunRecord,
    },
};

// ---------------------------------------------------------------------------
// 3-node definition used across all tests
// ---------------------------------------------------------------------------

fn three_node_def() -> WorkflowDef {
    let mut nodes = BTreeMap::new();

    nodes.insert(
        "plan".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-opus-4-8".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "run_input".into(),
                template: Some("List the docs to read for: {{topic}}".to_string()),
            },
            ..Default::default()
        },
    );

    nodes.insert(
        "read".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "fanout_item".into(),
                template: Some("Read and summarize: {{item}}".to_string()),
            },
            depends_on: vec!["plan".to_string()],
            fanout: Some(FanoutSpec {
                over: "node:plan.result.docs".to_string(),
                items: None,
            }),
            ..Default::default()
        },
    );

    nodes.insert(
        "synthesize".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-opus-4-8".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "node:read".into(),
                template: Some("Synthesize from: {{results}}".to_string()),
            },
            depends_on: vec!["read".to_string()],
            ..Default::default()
        },
    );

    WorkflowDef {
        version: 1,
        nodes,
        output: OutputRef {
            from: "node:synthesize".into(),
        },
        default_functions: None,
        max_concurrency: None,
    }
}

// ---------------------------------------------------------------------------
// In-memory driver helpers
// ---------------------------------------------------------------------------

/// Create a fresh `WorkflowRunRecord` with no nodes/fanout_src.
fn new_record(def_input: Value) -> WorkflowRunRecord {
    WorkflowRunRecord {
        run_id: "run_test".to_string(),
        step: 0,
        status: RunStatus::Running,
        abort: false,
        def_ref: "run_test".to_string(),
        input: def_input,
        nodes: BTreeMap::new(),
        fanout_src: BTreeMap::new(),
        result: None,
        result_error: None,
        notify: None,
        caller_session_id: None,
        caller_fs_root: None,
        idem_key: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Run one tick step (expand loops and fanouts + decide), in the tick's order.
///
/// If the decision is `Fire(uids)`, mark each uid as Running with synthetic
/// session/turn ids so the record advances correctly.
fn drive_step(
    def: &WorkflowDef,
    record: &mut WorkflowRunRecord,
    results: &BTreeMap<String, Value>,
) -> TickDecision {
    dag::expand_ready_loops(def, record, results);
    dag::expand_ready_fanouts(def, record, results);
    let decision = decide(def, record);
    if let TickDecision::Fire(ref uids) = decision {
        for uid in uids {
            record.nodes.insert(
                uid.clone(),
                NodeCheckpoint {
                    state: NodeState::Running,
                    session_id: Some(format!("wf_run_test_{}", uid)),
                    turn_id: Some(format!("turn_{}", uid)),
                    pending_at: Some(1_000_000),
                    ..Default::default()
                },
            );
        }
    }
    decision
}

/// Mark a node checkpoint as Done and store its result in the results map.
/// Simulates a successful node completion (as reconcile would do).
fn complete(
    record: &mut WorkflowRunRecord,
    results: &mut BTreeMap<String, Value>,
    node_uid_str: &str,
    result_value: Value,
) {
    if let Some(cp) = record.nodes.get_mut(node_uid_str) {
        cp.state = NodeState::Done;
        cp.result_ref = Some(format!("run_test/{}", node_uid_str));
    }
    results.insert(node_uid_str.to_string(), result_value);
}

/// Mark a node checkpoint as Failed using `classify_terminal` with a result_error.
///
/// Asserts that `classify_terminal` returns `NodeOutcome::Failed`, then applies
/// the failure to the checkpoint.
fn complete_with_error(record: &mut WorkflowRunRecord, node_uid_str: &str, error: &str) {
    // classify_terminal("completed", garbage_value, error) → must be Failed
    let outcome = classify_terminal(
        "completed",
        Some(json!({"unexpected": "garbage"})),
        Some(error.to_string()),
    );
    assert!(
        matches!(outcome, NodeOutcome::Failed(_)),
        "classify_terminal should return Failed when result_error is set"
    );

    if let Some(cp) = record.nodes.get_mut(node_uid_str) {
        cp.state = NodeState::Failed;
        cp.result_error = Some(error.to_string());
    }
}

// ---------------------------------------------------------------------------
// Test 1: fanout_barrier_synthesize_completes_in_order
// ---------------------------------------------------------------------------

#[test]
fn fanout_barrier_synthesize_completes_in_order() {
    let def = three_node_def();
    let mut record = new_record(json!({"topic": "rust"}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    // Step 1: fresh record → should Fire(["plan"])
    let step1 = drive_step(&def, &mut record, &results);
    match &step1 {
        TickDecision::Fire(uids) => {
            assert_eq!(uids, &vec!["plan".to_string()], "step 1 must fire plan");
        }
        other => panic!("expected Fire([plan]) at step 1, got {:?}", other),
    }

    // Complete plan with 2 docs.
    complete(
        &mut record,
        &mut results,
        "plan",
        json!({"docs": ["a", "b"]}),
    );

    // Step 2: expand fanout + fire read#0, read#1
    let step2 = drive_step(&def, &mut record, &results);
    let fired_uids = match &step2 {
        TickDecision::Fire(uids) => uids.clone(),
        other => panic!("expected Fire([read#0, read#1]) at step 2, got {:?}", other),
    };
    assert!(
        fired_uids.contains(&"read#0".to_string()),
        "read#0 should be fired"
    );
    assert!(
        fired_uids.contains(&"read#1".to_string()),
        "read#1 should be fired"
    );
    assert_eq!(fired_uids.len(), 2, "exactly 2 read items should fire");

    // Complete out of order: read#1 first, then read#0.
    complete(&mut record, &mut results, "read#1", json!({"summary": "B"}));
    complete(&mut record, &mut results, "read#0", json!({"summary": "A"}));

    // Assert gather_input returns results in NUMERIC order (read#0 first, then read#1),
    // NOT in completion order (which was read#1, read#0).
    let gathered = dag::gather_input(&def, &record, "synthesize", &results);
    let arr = gathered
        .as_array()
        .expect("gather_input must return an array");
    assert_eq!(arr.len(), 2, "gather_input must have 2 elements");
    assert_eq!(
        arr[0],
        json!({"summary": "A"}),
        "element 0 must be read#0's result (summary A), not B (completion order)"
    );
    assert_eq!(
        arr[1],
        json!({"summary": "B"}),
        "element 1 must be read#1's result (summary B)"
    );

    // Step 3: all read#i done → expand synthesize, fire it
    let step3 = drive_step(&def, &mut record, &results);
    match &step3 {
        TickDecision::Fire(uids) => {
            assert_eq!(
                uids,
                &vec!["synthesize".to_string()],
                "step 3 must fire synthesize"
            );
        }
        other => panic!("expected Fire([synthesize]) at step 3, got {:?}", other),
    }

    // Complete synthesize.
    complete(
        &mut record,
        &mut results,
        "synthesize",
        json!({"report": "done"}),
    );

    // Final quiescence: must be Completed.
    let q = dag::quiescence(&def, &record);
    assert_eq!(
        q,
        RunStatus::Completed,
        "run must be Completed after synthesize done"
    );
}

// ---------------------------------------------------------------------------
// Test 2: redelivered_drive_is_stable
// ---------------------------------------------------------------------------

#[test]
fn redelivered_drive_is_stable() {
    let def = three_node_def();
    let mut record = new_record(json!({"topic": "rust"}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    // Complete plan so read fanout can be expanded.
    record.nodes.insert(
        "plan".to_string(),
        NodeCheckpoint {
            state: NodeState::Done,
            session_id: Some("wf_run_test_plan".to_string()),
            turn_id: Some("turn_plan".to_string()),
            result_ref: Some("run_test/plan".to_string()),
            ..Default::default()
        },
    );
    results.insert("plan".to_string(), json!({"docs": ["a", "b"]}));

    // First expand: should expand "read" and insert read#0, read#1.
    let first_expanded = dag::expand_ready_fanouts(&def, &mut record, &results);
    assert_eq!(
        first_expanded,
        vec!["read".to_string()],
        "first expand must expand read"
    );
    let uid_count_after_first = dag::fanned_uids(&record, "read").len();
    assert_eq!(
        uid_count_after_first, 2,
        "must have 2 fanned uids after first expand"
    );

    // Second call to expand_ready_fanouts: must be idempotent — returns empty, frozen.
    let second_expanded = dag::expand_ready_fanouts(&def, &mut record, &results);
    assert!(
        second_expanded.is_empty(),
        "second expand must return empty (already expanded)"
    );

    let uid_count_after_second = dag::fanned_uids(&record, "read").len();
    assert_eq!(
        uid_count_after_second, uid_count_after_first,
        "fanned_uids count must be unchanged after re-expand"
    );

    // The fanout_src snapshot must be frozen (same items).
    assert_eq!(
        record.fanout_src["read"],
        vec![json!("a"), json!("b")],
        "fanout_src must be frozen after first expansion"
    );
}

// ---------------------------------------------------------------------------
// Test 3: completed_with_result_error_marks_node_failed
// ---------------------------------------------------------------------------

#[test]
fn completed_with_result_error_marks_node_failed() {
    let def = three_node_def();
    let mut record = new_record(json!({"topic": "rust"}));

    // Set up record with plan Running.
    record.nodes.insert(
        "plan".to_string(),
        NodeCheckpoint {
            state: NodeState::Running,
            session_id: Some("wf_run_test_plan".to_string()),
            turn_id: Some("turn_plan".to_string()),
            pending_at: Some(1_000_000),
            ..Default::default()
        },
    );

    // Apply result_error to plan using complete_with_error.
    complete_with_error(&mut record, "plan", "schema validation failed");

    // Checkpoint must be Failed.
    let plan_cp = record
        .nodes
        .get("plan")
        .expect("plan checkpoint must exist");
    assert_eq!(
        plan_cp.state,
        NodeState::Failed,
        "plan checkpoint state must be Failed"
    );
    assert_eq!(
        plan_cp.result_error.as_deref(),
        Some("schema validation failed"),
        "plan checkpoint must carry the error message"
    );

    // quiescence with a failed node required for output → must be Failed.
    let q = dag::quiescence(&def, &record);
    assert_eq!(
        q,
        RunStatus::Failed,
        "run must be Failed when a node has failed"
    );

    // decide must also Finalize(Failed).
    let decision = decide(&def, &record);
    match decision {
        TickDecision::Finalize(RunStatus::Failed) => {}
        other => panic!(
            "expected Finalize(Failed) when node failed, got {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Diamond definition: a → {b, c} → d
// ---------------------------------------------------------------------------

fn diamond_def() -> WorkflowDef {
    let mut nodes = BTreeMap::new();

    nodes.insert(
        "a".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "run_input".into(),
                template: None,
            },
            ..Default::default()
        },
    );

    nodes.insert(
        "b".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "node:a".into(),
                template: None,
            },
            depends_on: vec!["a".to_string()],
            ..Default::default()
        },
    );

    nodes.insert(
        "c".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "node:a".into(),
                template: None,
            },
            depends_on: vec!["a".to_string()],
            ..Default::default()
        },
    );

    nodes.insert(
        "d".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                // Join node: read BOTH branches. depends_on lists b and c, so the
                // input must consume both (a single `from` would drop one).
                from: workflow::types::InputFrom::Many(vec![
                    "node:b".to_string(),
                    "node:c".to_string(),
                ]),
                template: None,
            },
            depends_on: vec!["b".to_string(), "c".to_string()],
            ..Default::default()
        },
    );

    WorkflowDef {
        version: 1,
        nodes,
        output: OutputRef {
            from: "node:d".into(),
        },
        default_functions: None,
        max_concurrency: None,
    }
}

// ---------------------------------------------------------------------------
// Test 4: diamond_advances_both_branches_then_joins
// ---------------------------------------------------------------------------

#[test]
fn diamond_advances_both_branches_then_joins() {
    let def = diamond_def();
    let mut record = new_record(json!({}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    // Step 1: fresh record → should Fire(["a"])
    let step1 = drive_step(&def, &mut record, &results);
    match &step1 {
        TickDecision::Fire(uids) => {
            assert_eq!(uids, &vec!["a".to_string()], "step 1 must fire only a");
        }
        other => panic!("expected Fire([a]) at step 1, got {:?}", other),
    }

    // Complete a.
    complete(&mut record, &mut results, "a", json!({"x": 1}));

    // Step 2: both b and c become ready in the SAME frontier — both must fire.
    let step2 = drive_step(&def, &mut record, &results);
    let fired2 = match &step2 {
        TickDecision::Fire(uids) => uids.clone(),
        other => panic!("expected Fire([b, c]) at step 2, got {:?}", other),
    };
    assert!(
        fired2.contains(&"b".to_string()) && fired2.contains(&"c".to_string()),
        "step 2 must fire both b and c concurrently; got {:?}",
        fired2
    );
    assert_eq!(fired2.len(), 2, "exactly 2 nodes should fire at step 2");

    // Complete c out of order (before b).
    complete(&mut record, &mut results, "c", json!({"x": 3}));

    // d still waits on b — the join/barrier must block it.
    let frontier_after_c = dag::ready_frontier(&def, &record);
    assert!(
        frontier_after_c.is_empty(),
        "frontier must be empty after only c is done (d still waits on b); got {:?}",
        frontier_after_c
    );

    // Now complete b as well.
    complete(&mut record, &mut results, "b", json!({"x": 2}));

    // Step 3: both b and c done → d's join fires.
    let step3 = drive_step(&def, &mut record, &results);
    match &step3 {
        TickDecision::Fire(uids) => {
            assert_eq!(
                uids,
                &vec!["d".to_string()],
                "step 3 must fire d after join"
            );
        }
        other => panic!("expected Fire([d]) at step 3, got {:?}", other),
    }

    // Complete d.
    complete(&mut record, &mut results, "d", json!({"x": 4}));

    // Final quiescence: the run must be Completed.
    let q = dag::quiescence(&def, &record);
    assert_eq!(
        q,
        RunStatus::Completed,
        "run must be Completed after d finishes"
    );
}

// ---------------------------------------------------------------------------
// Test 5: abort_while_node_running_finalizes_cancelled
// ---------------------------------------------------------------------------

/// Logical abort e2e: set abort=true on a run with a Running node and assert
/// that decide returns Finalize(Cancelled). This validates that the abort flag
/// wins over a non-empty ready frontier and over any Running node state.
#[test]
fn abort_while_node_running_finalizes_cancelled() {
    let def = three_node_def();
    let mut record = new_record(json!({"topic": "rust"}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    // Drive one step so "plan" is Running.
    let step1 = drive_step(&def, &mut record, &results);
    match &step1 {
        TickDecision::Fire(uids) => {
            assert_eq!(uids, &vec!["plan".to_string()], "step 1 must fire plan");
        }
        other => panic!("expected Fire([plan]) at step 1, got {:?}", other),
    }

    // Sanity: plan is now Running in the record.
    let plan_cp = record
        .nodes
        .get("plan")
        .expect("plan checkpoint must exist");
    assert_eq!(
        plan_cp.state,
        NodeState::Running,
        "plan must be Running after drive_step"
    );

    // Set abort flag — simulates workflow::stop marking the run for cancellation.
    record.abort = true;

    // decide must return Finalize(Cancelled) regardless of the Running node or
    // any ready frontier.
    let decision = decide(&def, &record);
    match decision {
        TickDecision::Finalize(RunStatus::Cancelled) => {}
        other => panic!(
            "expected Finalize(Cancelled) when abort=true, got {:?}",
            other
        ),
    }

    // quiescence also short-circuits on abort and returns Cancelled.
    let q = dag::quiescence(&def, &record);
    assert_eq!(
        q,
        RunStatus::Cancelled,
        "quiescence must return Cancelled when abort=true"
    );

    // Simulate completing plan with a result (abort still set) — decide must
    // still return Finalize(Cancelled), not Fire or Park.
    complete(&mut record, &mut results, "plan", json!({"docs": ["x"]}));
    let decision2 = decide(&def, &record);
    match decision2 {
        TickDecision::Finalize(RunStatus::Cancelled) => {}
        other => panic!(
            "expected Finalize(Cancelled) even after plan done (abort=true), got {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Test 6 (empty-fanout acceptance): empty_fanout_completes_run
// ---------------------------------------------------------------------------

fn fanout_empty_def() -> WorkflowDef {
    let mut nodes = BTreeMap::new();

    nodes.insert(
        "a".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "run_input".into(),
                template: Some("t".to_string()),
            },
            ..Default::default()
        },
    );

    nodes.insert(
        "b".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "fanout_item".into(),
                template: Some("t".to_string()),
            },
            depends_on: vec!["a".to_string()],
            fanout: Some(FanoutSpec {
                over: "node:a.result.items".to_string(),
                items: None,
            }),
            ..Default::default()
        },
    );

    nodes.insert(
        "c".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                output: Some(json!({"type": "json"})),
                ..Default::default()
            },
            input: InputSpec {
                from: "node:b".into(),
                template: Some("t".to_string()),
            },
            depends_on: vec!["b".to_string()],
            ..Default::default()
        },
    );

    WorkflowDef {
        version: 1,
        nodes,
        output: OutputRef {
            from: "node:c".into(),
        },
        default_functions: None,
        max_concurrency: None,
    }
}

#[test]
fn empty_fanout_completes_run() {
    let def = fanout_empty_def();
    let mut record = new_record(json!({}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    // Step 1: fresh record → Fire(["a"])
    let step1 = drive_step(&def, &mut record, &results);
    match &step1 {
        TickDecision::Fire(uids) => {
            assert_eq!(uids, &vec!["a".to_string()], "step 1 must fire a");
        }
        other => panic!("expected Fire([a]) at step 1, got {:?}", other),
    }

    // Complete a with zero items.
    complete(&mut record, &mut results, "a", json!({"items": []}));

    // Step 2: expand b to empty (zero b#i), then c becomes ready (b vacuously Done).
    let step2 = drive_step(&def, &mut record, &results);
    match &step2 {
        TickDecision::Fire(uids) => {
            assert_eq!(
                uids,
                &vec!["c".to_string()],
                "step 2 must fire c after empty fanout expansion"
            );
        }
        other => panic!("expected Fire([c]) at step 2, got {:?}", other),
    }

    // Complete c.
    complete(&mut record, &mut results, "c", json!({"done": true}));

    // Final quiescence: must be Completed.
    let q = dag::quiescence(&def, &record);
    assert_eq!(
        q,
        RunStatus::Completed,
        "run must be Completed after empty fanout + c done"
    );
}

// ---------------------------------------------------------------------------
// Test 7: sweep_refires_then_fails_after_budget
// ---------------------------------------------------------------------------

#[test]
fn sweep_refires_then_fails_after_budget() {
    use workflow::timeout::{node_limits, timeout_action, TimeoutAction};
    // The node's own limits win over the worker defaults (30 s, no retries).
    let mut def = three_node_def();
    let plan = def.nodes.get_mut("plan").unwrap();
    plan.timeout_ms = Some(1_000);
    plan.retries = Some(1);
    let (timeout_ms, max_retries) = node_limits(&def, "plan", 30_000, 0);

    // A Running node, pending_at far in the past.
    let mut cp = NodeCheckpoint {
        state: NodeState::Running,
        session_id: Some("wf_r_plan@r0".into()),
        turn_id: Some("t0".into()),
        pending_at: Some(0),
        ..Default::default()
    };
    let now = 10_000;
    // first sweep: under budget -> refire attempt 1
    match timeout_action(&cp, timeout_ms, max_retries, now) {
        TimeoutAction::Refire { attempt } => {
            cp.retries = attempt;
            cp.pending_at = Some(now);
        }
        other => panic!("expected Refire, got {other:?}"),
    }
    assert_eq!(cp.retries, 1);
    // second sweep (still stuck): at budget -> fail out
    assert!(matches!(
        timeout_action(&cp, timeout_ms, max_retries, now + 10_000),
        TimeoutAction::FailOut
    ));
}

// ---------------------------------------------------------------------------
// Test 7b: max_concurrency_drains_fanout_in_waves
// ---------------------------------------------------------------------------

#[test]
fn max_concurrency_drains_fanout_in_waves() {
    let mut def = three_node_def();
    def.max_concurrency = Some(2);
    let mut record = new_record(json!({"topic": "rust"}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    assert!(matches!(
        drive_step(&def, &mut record, &results),
        TickDecision::Fire(_)
    ));
    complete(
        &mut record,
        &mut results,
        "plan",
        json!({"docs": ["a", "b", "c", "d", "e"]}),
    );

    let mut waves = Vec::new();
    loop {
        match drive_step(&def, &mut record, &results) {
            TickDecision::Fire(uids) if uids == ["synthesize"] => break,
            TickDecision::Fire(uids) => {
                assert!(
                    matches!(drive_step(&def, &mut record, &results), TickDecision::Park),
                    "the cap must hold while a wave runs"
                );
                for uid in &uids {
                    complete(&mut record, &mut results, uid, json!({"summary": uid}));
                }
                waves.push(uids);
            }
            other => panic!("expected Fire, got {other:?}"),
        }
    }
    assert_eq!(
        waves,
        vec![
            vec!["read#0", "read#1"],
            vec!["read#2", "read#3"],
            vec!["read#4"],
        ]
    );
}

// ---------------------------------------------------------------------------
// Test 8: sweep_refire_goes_through_decide
// ---------------------------------------------------------------------------

/// Prevents: the sweep re-firing a timed-out node directly while a required
/// sibling had already failed, keeping a dead run spending tokens.
#[test]
fn sweep_refire_goes_through_decide() {
    use workflow::timeout::{mark_refire, timeout_action, TimeoutAction};

    let def = diamond_def();
    let mut record = new_record(json!({}));
    record.nodes.insert(
        "a".into(),
        NodeCheckpoint {
            state: NodeState::Done,
            ..Default::default()
        },
    );
    let running = |uid: &str| NodeCheckpoint {
        state: NodeState::Running,
        session_id: Some(format!("wf_run_test_{uid}@r0")),
        pending_at: Some(0),
        ..Default::default()
    };
    record.nodes.insert("b".into(), running("b"));
    record.nodes.insert("c".into(), running("c"));

    let cp = record.nodes.get_mut("b").unwrap();
    let TimeoutAction::Refire { attempt } = timeout_action(cp, 1_000, 1, 10_000) else {
        panic!("expected Refire");
    };
    mark_refire(cp, attempt);
    assert_eq!(cp.state, NodeState::Pending, "a refired node is Pending");
    assert_eq!(cp.retries, 1);
    assert_eq!(
        cp.session_id.as_deref(),
        Some("wf_run_test_b@r0"),
        "the old session stays for the tick to stop"
    );

    // The next tick re-fires it through decide.
    match decide(&def, &record) {
        TickDecision::Fire(uids) => assert_eq!(uids, vec!["b".to_string()]),
        other => panic!("expected Fire([b]), got {other:?}"),
    }

    // A required sibling failed: decide finalizes instead of re-firing.
    complete_with_error(&mut record, "c", "boom");
    assert!(matches!(
        decide(&def, &record),
        TickDecision::Finalize(RunStatus::Failed)
    ));
}

// ---------------------------------------------------------------------------
// Test 9: continue_on_error_feeds_null_and_run_completes
// ---------------------------------------------------------------------------

/// Three independent text nodes, all required through an array `output.from`;
/// `y` tolerates failure.
fn independent_def(y_on_error: &str) -> WorkflowDef {
    serde_json::from_value(json!({
        "output": {"from": ["node:x", "node:y", "node:z"]},
        "nodes": {
            "x": {"agent": {"model": "m"}, "input": {"from": "run_input"}},
            "y": {"agent": {"model": "m"}, "input": {"from": "run_input"}, "on_error": y_on_error},
            "z": {"agent": {"model": "m"}, "input": {"from": "run_input"}}
        }
    }))
    .expect("independent def")
}

#[test]
fn continue_on_error_feeds_null_and_run_completes() {
    let def = independent_def("continue");
    let mut record = new_record(json!({}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    match drive_step(&def, &mut record, &results) {
        TickDecision::Fire(uids) => assert_eq!(uids, vec!["x", "y", "z"]),
        other => panic!("expected Fire([x, y, z]), got {other:?}"),
    }
    complete_with_error(&mut record, "y", "boom");
    assert!(
        matches!(decide(&def, &record), TickDecision::Park),
        "a tolerated failure does not finalize while siblings run"
    );
    complete(&mut record, &mut results, "x", json!("text x"));
    assert!(matches!(decide(&def, &record), TickDecision::Park));
    complete(&mut record, &mut results, "z", json!("text z"));
    assert!(matches!(
        decide(&def, &record),
        TickDecision::Finalize(RunStatus::Completed)
    ));
    assert_eq!(
        dag::gather_output(&def, &record, &results),
        json!({"x": "text x", "y": null, "z": "text z"})
    );

    // The same failure without `continue` fails the run at once.
    let strict = independent_def("fail");
    let mut record = new_record(json!({}));
    drive_step(&strict, &mut record, &results);
    complete_with_error(&mut record, "y", "boom");
    assert!(matches!(
        decide(&strict, &record),
        TickDecision::Finalize(RunStatus::Failed)
    ));
}

// ---------------------------------------------------------------------------
// Loops: one work/verify group per ticket, then one integration node
// ---------------------------------------------------------------------------

fn ticket_nodes(ticket: &str, max_repeats: u32) -> Value {
    json!({
        format!("{ticket}-work"): {
            "agent": {"profile": "deepwork-impl"},
            "input": {"from": "run_input"},
            "on_error": "continue"
        },
        format!("{ticket}-verify"): {
            "agent": {"profile": "deepwork-adversary", "output": {"type": "json"}},
            "input": {"from": format!("node:{ticket}-work")},
            "depends_on": [format!("{ticket}-work")],
            "loop": {"body": [format!("{ticket}-work")],
                     "while": {"field": "verdict", "equals": "reject"},
                     "max_repeats": max_repeats},
            "on_error": "continue"
        }
    })
}

/// `kan-12` and `kan-13` tickets, each a work → verify loop, read by `integrate`.
fn wave_def(max_repeats: u32) -> WorkflowDef {
    let mut nodes = ticket_nodes("kan-12", max_repeats);
    nodes.as_object_mut().unwrap().extend(
        ticket_nodes("kan-13", max_repeats)
            .as_object()
            .unwrap()
            .clone(),
    );
    nodes["integrate"] = json!({
        "agent": {"model": "m"},
        "input": {"from": ["node:kan-12-verify", "node:kan-13-verify"]},
        "depends_on": ["kan-12-verify", "kan-13-verify"]
    });
    serde_json::from_value(json!({"nodes": nodes, "output": {"from": "node:integrate"}}))
        .expect("wave def")
}

/// One ticket's work → verify loop; the run returns the verdict.
fn ticket_def(max_repeats: u32) -> WorkflowDef {
    serde_json::from_value(json!({
        "nodes": ticket_nodes("kan-12", max_repeats),
        "output": {"from": "node:kan-12-verify"}
    }))
    .expect("ticket def")
}

fn fired(decision: TickDecision) -> Vec<String> {
    match decision {
        TickDecision::Fire(uids) => uids,
        other => panic!("expected Fire, got {other:?}"),
    }
}

#[test]
fn loop_repeats_only_the_rejected_ticket() {
    let def = wave_def(2);
    let mut record = new_record(json!({"wave": "w1"}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["kan-12-work", "kan-13-work"]
    );
    complete(&mut record, &mut results, "kan-12-work", json!("12 done"));
    complete(&mut record, &mut results, "kan-13-work", json!("13 done"));
    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["kan-12-verify", "kan-13-verify"]
    );
    complete(
        &mut record,
        &mut results,
        "kan-12-verify",
        json!({"verdict": "pass"}),
    );
    complete(
        &mut record,
        &mut results,
        "kan-13-verify",
        json!({"verdict": "reject"}),
    );

    // Only the rejected ticket repeats; integrate waits for its last pass.
    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["kan-13-work#it1"]
    );
    complete(
        &mut record,
        &mut results,
        "kan-13-work#it1",
        json!("13 fixed"),
    );
    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["kan-13-verify#it1"]
    );
    complete(
        &mut record,
        &mut results,
        "kan-13-verify#it1",
        json!({"verdict": "pass"}),
    );
    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["integrate"]
    );
    assert_eq!(
        dag::gather_input(&def, &record, "integrate", &results),
        json!({"kan-12-verify": {"verdict": "pass"}, "kan-13-verify": {"verdict": "pass"}})
    );
    complete(&mut record, &mut results, "integrate", json!("merged"));
    assert!(matches!(
        drive_step(&def, &mut record, &results),
        TickDecision::Finalize(RunStatus::Completed)
    ));
    assert_eq!(
        record.nodes.keys().collect::<Vec<_>>(),
        vec![
            "integrate",
            "kan-12-verify",
            "kan-12-work",
            "kan-13-verify",
            "kan-13-verify#it1",
            "kan-13-work",
            "kan-13-work#it1"
        ]
    );
}

#[test]
fn loop_exhausted_completes_with_last_verdict() {
    let def = ticket_def(1);
    let mut record = new_record(json!({}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();

    for (k, uid) in ["", "#it1"].into_iter().enumerate() {
        let work = format!("kan-12-work{uid}");
        let verify = format!("kan-12-verify{uid}");
        assert_eq!(
            fired(drive_step(&def, &mut record, &results)),
            vec![work.clone()]
        );
        complete(&mut record, &mut results, &work, json!(format!("try {k}")));
        assert_eq!(
            fired(drive_step(&def, &mut record, &results)),
            vec![verify.clone()]
        );
        let verdict = json!({"verdict": "reject", "findings": [format!("finding {k}")]});
        complete(&mut record, &mut results, &verify, verdict);
    }

    assert!(matches!(
        drive_step(&def, &mut record, &results),
        TickDecision::Finalize(RunStatus::Completed)
    ));
    assert!(!record.nodes.contains_key("kan-12-verify#it2"));
    assert_eq!(
        dag::gather_output(&def, &record, &results),
        json!({"verdict": "reject", "findings": ["finding 1"]})
    );
}

#[test]
fn loop_resumes_from_serialized_record() {
    let def = ticket_def(2);
    let mut record = new_record(json!({}));
    let mut results: BTreeMap<String, Value> = BTreeMap::new();
    drive_step(&def, &mut record, &results);
    complete(&mut record, &mut results, "kan-12-work", json!("try 0"));
    drive_step(&def, &mut record, &results);
    complete(
        &mut record,
        &mut results,
        "kan-12-verify",
        json!({"verdict": "reject"}),
    );
    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["kan-12-work#it1"]
    );

    // A restart reloads the record and results from state; a duplicate tick adds nothing.
    let mut record: WorkflowRunRecord =
        serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
    let def: WorkflowDef = serde_json::from_value(serde_json::to_value(&def).unwrap()).unwrap();
    let before = record.clone();
    assert!(dag::expand_ready_loops(&def, &mut record, &results).is_empty());
    assert_eq!(record, before);

    complete(&mut record, &mut results, "kan-12-work#it1", json!("try 1"));
    assert_eq!(
        fired(drive_step(&def, &mut record, &results)),
        vec!["kan-12-verify#it1"]
    );
    complete(
        &mut record,
        &mut results,
        "kan-12-verify#it1",
        json!({"verdict": "pass"}),
    );
    assert!(matches!(
        drive_step(&def, &mut record, &results),
        TickDecision::Finalize(RunStatus::Completed)
    ));
    assert_eq!(
        dag::gather_output(&def, &record, &results),
        json!({"verdict": "pass"})
    );
}
