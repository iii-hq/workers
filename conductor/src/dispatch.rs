use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::join_all;
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::json;
use uuid::Uuid;

use crate::agents::run_local_agent;
use crate::gates::{all_passed, run_all_gates};
use crate::git::{create_worktree, current_branch, diff_against, remove_worktree};
use crate::state::write_run;
use crate::types::{
    now_ms, AgentKind, AgentRunState, AgentSpec, AgentStatus, DispatchInput, DispatchSummary,
    MergeResult, MergeWinner, RunState,
};

fn worktree_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".iii")
        .join("conductor")
        .join("worktrees")
}

async fn run_agent_in_worktree(
    iii: &III,
    index: usize,
    spec: &AgentSpec,
    run_id: &str,
    cwd: &Path,
    base_ref: &str,
    timeout_ms: Option<u64>,
) -> AgentRunState {
    let started_at = now_ms();
    let kind_label = format!("{:?}", spec.kind).to_lowercase();
    let branch = format!("conductor/{run_id}/{index}-{kind_label}");

    if spec.kind == AgentKind::Remote {
        let Some(function_id) = spec.function_id.clone() else {
            return AgentRunState {
                agent: spec.clone(),
                status: AgentStatus::Failed,
                started_at: Some(started_at),
                finished_at: Some(now_ms()),
                exit_code: None,
                output: None,
                error: Some("remote agent requires function_id".to_string()),
                diff: None,
                worktree_path: None,
                branch: None,
                gate_results: Default::default(),
            };
        };
        let prompt = spec.prompt.clone().unwrap_or_default();
        let result = iii
            .trigger(TriggerRequest {
                function_id,
                payload: json!({ "task": prompt, "cwd": cwd.to_string_lossy() }),
                action: None,
                timeout_ms: Some(timeout_ms.unwrap_or(600_000)),
            })
            .await;
        let finished_at = Some(now_ms());
        return match result {
            Ok(val) => AgentRunState {
                agent: spec.clone(),
                status: AgentStatus::Finished,
                started_at: Some(started_at),
                finished_at,
                exit_code: Some(0),
                output: Some(val.to_string()),
                error: None,
                diff: None,
                worktree_path: None,
                branch: None,
                gate_results: Default::default(),
            },
            Err(e) => AgentRunState {
                agent: spec.clone(),
                status: AgentStatus::Failed,
                started_at: Some(started_at),
                finished_at,
                exit_code: None,
                output: None,
                error: Some(e.to_string()),
                diff: None,
                worktree_path: None,
                branch: None,
                gate_results: Default::default(),
            },
        };
    }

    let wt_path = match create_worktree(cwd, &branch, &worktree_root()).await {
        Ok(p) => p,
        Err(reason) => {
            return AgentRunState {
                agent: spec.clone(),
                status: AgentStatus::Failed,
                started_at: Some(started_at),
                finished_at: Some(now_ms()),
                exit_code: None,
                output: None,
                error: Some(reason),
                diff: None,
                worktree_path: None,
                branch: Some(branch),
                gate_results: Default::default(),
            };
        }
    };

    let r = run_local_agent(spec, &wt_path, timeout_ms).await;
    let finished_at = Some(now_ms());
    let diff = diff_against(&wt_path, base_ref).await;

    AgentRunState {
        agent: spec.clone(),
        status: if r.ok {
            AgentStatus::Finished
        } else {
            AgentStatus::Failed
        },
        started_at: Some(started_at),
        finished_at,
        exit_code: r.code,
        output: Some(r.stdout),
        error: if r.ok {
            None
        } else {
            Some(r.stderr.trim().to_string())
        },
        diff: Some(diff),
        worktree_path: Some(wt_path.to_string_lossy().into_owned()),
        branch: Some(branch),
        gate_results: Default::default(),
    }
}

pub async fn dispatch(iii: Arc<III>, input: DispatchInput) -> Result<(DispatchSummary, RunState)> {
    if input.task.trim().is_empty() {
        return Err(anyhow!("task required"));
    }
    if input.agents.is_empty() {
        return Err(anyhow!("at least one agent required"));
    }
    if input.cwd.trim().is_empty() {
        return Err(anyhow!("cwd required"));
    }

    let run_id = Uuid::new_v4().to_string();
    let cwd = PathBuf::from(&input.cwd);
    let base_ref = current_branch(&cwd).await;

    let agents: Vec<AgentSpec> = input
        .agents
        .iter()
        .cloned()
        .map(|mut a| {
            if a.prompt.is_none() {
                a.prompt = Some(input.task.clone());
            }
            a
        })
        .collect();

    let mut run = RunState {
        id: run_id.clone(),
        task: input.task.clone(),
        cwd: input.cwd.clone(),
        started_at: now_ms(),
        finished_at: None,
        agents: agents
            .iter()
            .map(|a| AgentRunState {
                agent: a.clone(),
                status: AgentStatus::Pending,
                started_at: None,
                finished_at: None,
                exit_code: None,
                output: None,
                error: None,
                diff: None,
                worktree_path: None,
                branch: None,
                gate_results: Default::default(),
            })
            .collect(),
        winner_index: None,
    };

    write_run(&iii, &run)
        .await
        .map_err(|e: IIIError| anyhow!("state::set seed: {e}"))?;

    let futures = agents.iter().enumerate().map(|(i, spec)| {
        let iii = iii.clone();
        let run_id = run_id.clone();
        let cwd = cwd.clone();
        let base_ref = base_ref.clone();
        let spec = spec.clone();
        async move {
            run_agent_in_worktree(
                iii.as_ref(),
                i,
                &spec,
                &run_id,
                &cwd,
                &base_ref,
                input.timeout_ms,
            )
            .await
        }
    });
    let mut settled: Vec<AgentRunState> = join_all(futures).await;

    if !input.gates.is_empty() {
        for state in settled.iter_mut() {
            if state.status == AgentStatus::Finished {
                if let Some(path) = state.worktree_path.as_ref().map(PathBuf::from) {
                    state.gate_results = run_all_gates(iii.as_ref(), &input.gates, &path).await;
                }
            }
        }
    }

    run.agents = settled;
    run.finished_at = Some(now_ms());
    write_run(&iii, &run)
        .await
        .map_err(|e: IIIError| anyhow!("state::set finalize: {e}"))?;

    let summary = DispatchSummary {
        ok: true,
        run_id: run.id.clone(),
        agents: run.agents.len(),
        gates: input.gates.len(),
    };
    Ok((summary, run))
}

pub async fn merge_run(iii: Arc<III>, mut run: RunState) -> MergeResult {
    let mut losers: Vec<usize> = Vec::new();
    let mut winner: Option<MergeWinner> = None;

    for (i, a) in run.agents.iter().enumerate() {
        if a.status != AgentStatus::Finished {
            losers.push(i);
            continue;
        }
        let gates_ok = a.gate_results.is_empty() || all_passed(&a.gate_results);
        let diff_non_empty = a
            .diff
            .as_deref()
            .map(|d| !d.trim().is_empty())
            .unwrap_or(false);
        if !gates_ok || !diff_non_empty {
            losers.push(i);
            continue;
        }
        if winner.is_none() {
            winner = Some(MergeWinner {
                index: i,
                agent: a.agent.clone(),
                diff: a.diff.clone().unwrap_or_default(),
                branch: a.branch.clone(),
            });
        } else {
            losers.push(i);
        }
    }

    let cwd = PathBuf::from(&run.cwd);
    for i in losers.iter() {
        if let Some(path) = run.agents[*i].worktree_path.as_ref().map(PathBuf::from) {
            let _ = remove_worktree(&cwd, &path).await;
        }
    }

    run.winner_index = winner.as_ref().map(|w| w.index);
    let _ = write_run(&iii, &run).await;

    if let Some(w) = winner {
        MergeResult {
            ok: true,
            reason: None,
            run_id: run.id,
            winner: Some(w),
            losers,
        }
    } else {
        MergeResult {
            ok: false,
            reason: Some("no agent passed all gates with a non-empty diff".to_string()),
            run_id: run.id,
            winner: None,
            losers,
        }
    }
}
