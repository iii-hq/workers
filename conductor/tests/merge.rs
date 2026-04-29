use iii_conductor::gates::all_passed;
use iii_conductor::types::{
    AgentKind, AgentRunState, AgentSpec, AgentStatus, GateRunResult, RunState,
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

fn finished_at(
    kind: AgentKind,
    diff: &str,
    gates: Vec<GateRunResult>,
    finished_at: u64,
) -> AgentRunState {
    AgentRunState {
        agent: agent(kind),
        status: AgentStatus::Finished,
        started_at: Some(0),
        finished_at: Some(finished_at),
        exit_code: Some(0),
        output: None,
        error: None,
        diff: Some(diff.to_string()),
        worktree_path: None,
        branch: None,
        gate_results: gates,
    }
}

fn pass() -> Vec<GateRunResult> {
    vec![GateRunResult {
        function_id: "tests".to_string(),
        description: None,
        ok: true,
        reason: None,
    }]
}

fn fail() -> Vec<GateRunResult> {
    vec![GateRunResult {
        function_id: "tests".to_string(),
        description: None,
        ok: false,
        reason: Some("unit failed".to_string()),
    }]
}

fn pick_winner(agents: &[AgentRunState]) -> Option<usize> {
    agents
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            a.status == AgentStatus::Finished
                && a.diff
                    .as_deref()
                    .map(|d| !d.trim().is_empty())
                    .unwrap_or(false)
                && (a.gate_results.is_empty() || all_passed(&a.gate_results))
        })
        .min_by_key(|(_, a)| a.finished_at.unwrap_or(u64::MAX))
        .map(|(i, _)| i)
}

#[test]
fn all_passed_empty_is_true() {
    let v: Vec<GateRunResult> = Vec::new();
    assert!(all_passed(&v));
}

#[test]
fn all_passed_mixed_is_false() {
    assert!(!all_passed(&fail()));
}

#[test]
fn all_passed_all_ok_is_true() {
    assert!(all_passed(&pass()));
}

#[test]
fn duplicate_function_ids_are_preserved_as_separate_entries() {
    let gates = vec![
        GateRunResult {
            function_id: "verify::tests".to_string(),
            description: Some("unit".to_string()),
            ok: false,
            reason: Some("first run failed".to_string()),
        },
        GateRunResult {
            function_id: "verify::tests".to_string(),
            description: Some("integration".to_string()),
            ok: true,
            reason: None,
        },
    ];
    assert_eq!(gates.len(), 2);
    assert!(!all_passed(&gates));
}

#[test]
fn merge_skips_input_order_when_other_agent_finished_earlier() {
    let agents = [
        finished_at(AgentKind::Claude, "diff --git a b\n", pass(), 200),
        finished_at(AgentKind::Codex, "diff --git c d\n", pass(), 100),
        finished_at(AgentKind::Gemini, "diff --git e f\n", pass(), 300),
    ];
    assert_eq!(pick_winner(&agents), Some(1));
}

#[test]
fn merge_skips_failed_gates_even_if_finished_first() {
    let agents = [
        finished_at(AgentKind::Claude, "diff --git a b\n", fail(), 50),
        finished_at(AgentKind::Codex, "diff --git e f\n", pass(), 200),
    ];
    assert_eq!(pick_winner(&agents), Some(1));
}

#[test]
fn merge_returns_none_when_every_agent_failed() {
    let agents = [
        AgentRunState {
            agent: agent(AgentKind::Claude),
            status: AgentStatus::Failed,
            started_at: Some(0),
            finished_at: Some(50),
            exit_code: None,
            output: None,
            error: Some("crashed".to_string()),
            diff: None,
            worktree_path: None,
            branch: None,
            gate_results: Vec::new(),
        },
        AgentRunState {
            agent: agent(AgentKind::Codex),
            status: AgentStatus::Failed,
            started_at: Some(0),
            finished_at: Some(60),
            exit_code: None,
            output: None,
            error: Some("crashed".to_string()),
            diff: None,
            worktree_path: None,
            branch: None,
            gate_results: Vec::new(),
        },
    ];
    assert_eq!(pick_winner(&agents), None);
}

#[test]
fn merge_skips_empty_diff() {
    let agents = [
        finished_at(AgentKind::Claude, "", pass(), 50),
        finished_at(AgentKind::Codex, "diff --git a b\n", pass(), 100),
    ];
    assert_eq!(pick_winner(&agents), Some(1));
}

#[test]
fn run_state_round_trip_with_vec_gate_results() {
    let run = RunState {
        id: "r-vec".to_string(),
        task: "t".to_string(),
        cwd: "/tmp".to_string(),
        started_at: 0,
        finished_at: Some(100),
        agents: vec![finished_at(
            AgentKind::Claude,
            "diff --git a b\n",
            pass(),
            42,
        )],
        winner_index: None,
    };
    let json = serde_json::to_value(&run).unwrap();
    let back: RunState = serde_json::from_value(json).unwrap();
    assert_eq!(back.agents[0].gate_results.len(), 1);
    assert_eq!(back.agents[0].gate_results[0].function_id, "tests");
}
