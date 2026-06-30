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
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "run_input".into(),
                template: Some("List the docs to read for: {{topic}}".to_string()),
            },
            depends_on: vec![],
            fanout: None,
        },
    );

    nodes.insert(
        "read".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "fanout_item".into(),
                template: Some("Read and summarize: {{item}}".to_string()),
            },
            depends_on: vec!["plan".to_string()],
            fanout: Some(FanoutSpec {
                over: "node:plan.result.docs".to_string(),
            }),
        },
    );

    nodes.insert(
        "synthesize".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-opus-4-8".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "node:read".into(),
                template: Some("Synthesize from: {{results}}".to_string()),
            },
            depends_on: vec!["read".to_string()],
            fanout: None,
        },
    );

    WorkflowDef {
        version: 1,
        nodes,
        output: OutputRef {
            from: "node:synthesize".into(),
        },
        default_functions: None,
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
        reply_to: None,
        caller_session_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Run one tick step (expand fanouts + decide).
///
/// If the decision is `Fire(uids)`, mark each uid as Running with synthetic
/// session/turn ids so the record advances correctly.
fn drive_step(
    def: &WorkflowDef,
    record: &mut WorkflowRunRecord,
    results: &BTreeMap<String, Value>,
) -> TickDecision {
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
                    result_ref: None,
                    result_error: None,
                    pending_at: Some(1_000_000),
                    pending_timeout_ms: None,
                    retries: 0,
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
            result_error: None,
            pending_at: None,
            pending_timeout_ms: None,
            retries: 0,
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
            result_ref: None,
            result_error: None,
            pending_at: Some(1_000_000),
            pending_timeout_ms: None,
            retries: 0,
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
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "run_input".into(),
                template: None,
            },
            depends_on: vec![],
            fanout: None,
        },
    );

    nodes.insert(
        "b".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "node:a".into(),
                template: None,
            },
            depends_on: vec!["a".to_string()],
            fanout: None,
        },
    );

    nodes.insert(
        "c".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "node:a".into(),
                template: None,
            },
            depends_on: vec!["a".to_string()],
            fanout: None,
        },
    );

    nodes.insert(
        "d".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
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
            fanout: None,
        },
    );

    WorkflowDef {
        version: 1,
        nodes,
        output: OutputRef {
            from: "node:d".into(),
        },
        default_functions: None,
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
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "run_input".into(),
                template: Some("t".to_string()),
            },
            depends_on: vec![],
            fanout: None,
        },
    );

    nodes.insert(
        "b".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "fanout_item".into(),
                template: Some("t".to_string()),
            },
            depends_on: vec!["a".to_string()],
            fanout: Some(FanoutSpec {
                over: "node:a.result.items".to_string(),
            }),
        },
    );

    nodes.insert(
        "c".to_string(),
        NodeDef {
            agent: AgentSpec {
                model: "claude-haiku-4-5".to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output: Some(json!({"type": "json"})),
            },
            input: InputSpec {
                from: "node:b".into(),
                template: Some("t".to_string()),
            },
            depends_on: vec!["b".to_string()],
            fanout: None,
        },
    );

    WorkflowDef {
        version: 1,
        nodes,
        output: OutputRef {
            from: "node:c".into(),
        },
        default_functions: None,
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
    use workflow::timeout::{timeout_action, TimeoutAction};
    use workflow::types::*;
    // A Running node, pending_at far in the past, max_retries = 1.
    let mut cp = NodeCheckpoint {
        state: NodeState::Running,
        session_id: Some("wf_r_n@r0".into()),
        turn_id: Some("t0".into()),
        result_ref: None,
        result_error: None,
        pending_at: Some(0),
        pending_timeout_ms: Some(1_000),
        retries: 0,
    };
    let now = 10_000;
    // first sweep: under budget -> refire attempt 1
    match timeout_action(&cp, 30_000, 1, now) {
        TimeoutAction::Refire { attempt } => {
            cp.retries = attempt;
            cp.pending_at = Some(now);
        }
        other => panic!("expected Refire, got {other:?}"),
    }
    assert_eq!(cp.retries, 1);
    // second sweep (still stuck): at budget -> fail out
    assert!(matches!(
        timeout_action(&cp, 30_000, 1, now + 10_000),
        TimeoutAction::FailOut
    ));
}
