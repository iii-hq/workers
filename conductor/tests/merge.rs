use std::collections::HashMap;

use iii_conductor::gates::all_passed;
use iii_conductor::types::{
    AgentKind, AgentRunState, AgentSpec, AgentStatus, GateOutcome, RunState,
};

fn agent(kind: AgentKind) -> AgentSpec {
    AgentSpec {
        kind,
        bin: None,
        args: None,
        function_id: None,
        prompt: None,
        worktree: false,
    }
}

fn finished(kind: AgentKind, diff: &str, gates: HashMap<String, GateOutcome>) -> AgentRunState {
    AgentRunState {
        agent: agent(kind),
        status: AgentStatus::Finished,
        started_at: Some(0),
        finished_at: Some(1),
        exit_code: Some(0),
        output: None,
        error: None,
        diff: Some(diff.to_string()),
        worktree_path: None,
        branch: None,
        gate_results: gates,
    }
}

fn pass() -> HashMap<String, GateOutcome> {
    let mut m = HashMap::new();
    m.insert(
        "tests".to_string(),
        GateOutcome {
            ok: true,
            reason: None,
        },
    );
    m
}

fn fail() -> HashMap<String, GateOutcome> {
    let mut m = HashMap::new();
    m.insert(
        "tests".to_string(),
        GateOutcome {
            ok: false,
            reason: Some("unit failed".to_string()),
        },
    );
    m
}

#[test]
fn all_passed_empty_is_true() {
    let m: HashMap<String, GateOutcome> = HashMap::new();
    assert!(all_passed(&m));
}

#[test]
fn all_passed_mixed_is_false() {
    let m = fail();
    assert!(!all_passed(&m));
}

#[test]
fn all_passed_all_ok_is_true() {
    let m = pass();
    assert!(all_passed(&m));
}

#[test]
fn merge_picks_first_finished_with_diff_and_passing_gates() {
    let run = RunState {
        id: "r1".to_string(),
        task: "t".to_string(),
        cwd: "/tmp".to_string(),
        started_at: 0,
        finished_at: None,
        agents: vec![
            finished(AgentKind::Claude, "", pass()),
            finished(AgentKind::Codex, "diff --git a b\n", pass()),
            finished(AgentKind::Gemini, "diff --git c d\n", pass()),
        ],
        winner_index: None,
    };

    let winner_idx = run
        .agents
        .iter()
        .enumerate()
        .find(|(_, a)| {
            a.status == AgentStatus::Finished
                && a.diff
                    .as_deref()
                    .map(|d| !d.trim().is_empty())
                    .unwrap_or(false)
                && (a.gate_results.is_empty() || all_passed(&a.gate_results))
        })
        .map(|(i, _)| i);
    assert_eq!(winner_idx, Some(1));
}

#[test]
fn merge_skips_failed_gates() {
    let agents = [
        finished(AgentKind::Claude, "diff --git a b\n", fail()),
        finished(AgentKind::Codex, "diff --git e f\n", pass()),
    ];

    let winner_idx = agents
        .iter()
        .enumerate()
        .find(|(_, a)| {
            a.status == AgentStatus::Finished
                && a.diff
                    .as_deref()
                    .map(|d| !d.trim().is_empty())
                    .unwrap_or(false)
                && (a.gate_results.is_empty() || all_passed(&a.gate_results))
        })
        .map(|(i, _)| i);
    assert_eq!(winner_idx, Some(1));
}
