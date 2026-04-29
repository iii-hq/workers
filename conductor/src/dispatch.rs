use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::stream::{FuturesUnordered, StreamExt};
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
                gate_results: Vec::new(),
            };
        }
    };

    let (ok, code, output, error) = if spec.kind == AgentKind::Remote {
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
                worktree_path: Some(wt_path.to_string_lossy().into_owned()),
                branch: Some(branch),
                gate_results: Vec::new(),
            };
        };
        let prompt = spec.prompt.clone().unwrap_or_default();
        let payload = json!({ "task": prompt, "cwd": wt_path.to_string_lossy() });
        let result = iii
            .trigger(TriggerRequest {
                function_id,
                payload,
                action: None,
                timeout_ms: Some(timeout_ms.unwrap_or(600_000)),
            })
            .await;
        match result {
            Ok(val) => (true, Some(0), Some(val.to_string()), None),
            Err(e) => (false, None, None, Some(e.to_string())),
        }
    } else {
        let r = run_local_agent(spec, &wt_path, timeout_ms).await;
        let err = if r.ok {
            None
        } else {
            Some(r.stderr.trim().to_string())
        };
        (r.ok, r.code, Some(r.stdout), err)
    };

    let diff = diff_against(&wt_path, base_ref).await;
    let finished_at = Some(now_ms());

    AgentRunState {
        agent: spec.clone(),
        status: if ok {
            AgentStatus::Finished
        } else {
            AgentStatus::Failed
        },
        started_at: Some(started_at),
        finished_at,
        exit_code: code,
        output,
        error,
        diff: Some(diff),
        worktree_path: Some(wt_path.to_string_lossy().into_owned()),
        branch: Some(branch),
        gate_results: Vec::new(),
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
                gate_results: Vec::new(),
            })
            .collect(),
        winner_index: None,
    };

    write_run(&iii, &run)
        .await
        .map_err(|e: IIIError| anyhow!("state::set seed: {e}"))?;

    let mut futures = FuturesUnordered::new();
    for (i, spec) in agents.iter().enumerate() {
        let iii = iii.clone();
        let spec = spec.clone();
        let cwd = cwd.clone();
        let base_ref = base_ref.clone();
        let run_id = run_id.clone();
        futures.push(async move {
            let state = run_agent_in_worktree(
                iii.as_ref(),
                i,
                &spec,
                &run_id,
                &cwd,
                &base_ref,
                input.timeout_ms,
            )
            .await;
            (i, state)
        });
    }

    while let Some((i, mut state)) = futures.next().await {
        if state.status == AgentStatus::Finished && !input.gates.is_empty() {
            if let Some(path) = state.worktree_path.as_ref().map(PathBuf::from) {
                state.gate_results = run_all_gates(iii.as_ref(), &input.gates, &path).await;
            }
        }
        run.agents[i] = state;
        if let Err(e) = write_run(&iii, &run).await {
            tracing::warn!(error = %e, run_id = %run.id, "mid-run state::set failed");
        }
    }

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

fn agent_eligible(a: &AgentRunState) -> bool {
    if a.status != AgentStatus::Finished {
        return false;
    }
    let gates_ok = a.gate_results.is_empty() || all_passed(&a.gate_results);
    if !gates_ok {
        return false;
    }
    a.diff
        .as_deref()
        .map(|d| !d.trim().is_empty())
        .unwrap_or(false)
}

pub async fn merge_run(iii: Arc<III>, mut run: RunState) -> MergeResult {
    let winner_index: Option<usize> = run
        .agents
        .iter()
        .enumerate()
        .filter(|(_, a)| agent_eligible(a))
        .min_by_key(|(_, a)| a.finished_at.unwrap_or(u64::MAX))
        .map(|(i, _)| i);

    let mut losers: Vec<usize> = Vec::new();
    for (i, _) in run.agents.iter().enumerate() {
        if Some(i) != winner_index {
            losers.push(i);
        }
    }

    let cwd = PathBuf::from(&run.cwd);
    for i in losers.iter() {
        if let Some(path) = run.agents[*i].worktree_path.as_ref().map(PathBuf::from) {
            let _ = remove_worktree(&cwd, &path).await;
        }
    }

    run.winner_index = winner_index;
    let _ = write_run(&iii, &run).await;

    if let Some(idx) = winner_index {
        let a = &run.agents[idx];
        MergeResult {
            ok: true,
            reason: None,
            run_id: run.id,
            winner: Some(MergeWinner {
                index: idx,
                agent: a.agent.clone(),
                diff: a.diff.clone().unwrap_or_default(),
                branch: a.branch.clone(),
            }),
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
