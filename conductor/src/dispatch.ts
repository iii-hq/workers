import { randomUUID } from 'node:crypto'
import { homedir } from 'node:os'
import { resolve } from 'node:path'
import type { ISdk } from 'iii-sdk'
import { runLocalAgent } from './agents.js'
import { allPassed, runAllGates } from './gates.js'
import { createWorktree, currentBranch, diffAgainst, removeWorktree } from './git.js'
import { writeRun } from './state.js'
import type { AgentRunState, AgentSpec, DispatchInput, DispatchResult, MergeResult, RunState } from './types.js'

function worktreeRoot(): string {
  return resolve(homedir(), '.iii', 'conductor', 'worktrees')
}

async function runAgentInWorktree(
  sdk: ISdk,
  index: number,
  spec: AgentSpec,
  runId: string,
  cwd: string,
  baseRef: string,
  timeoutMs: number | undefined,
): Promise<AgentRunState> {
  const startedAt = Date.now()
  const branch = `conductor/${runId}/${index}-${spec.kind}`

  if (spec.kind === 'remote') {
    if (!spec.function_id) {
      return {
        agent: spec,
        status: 'failed',
        startedAt,
        finishedAt: Date.now(),
        error: 'remote agent requires function_id',
      }
    }
    try {
      const out = await sdk.trigger<{ task: string; cwd: string }, unknown>({
        function_id: spec.function_id,
        payload: { task: spec.prompt ?? '', cwd },
        timeoutMs: timeoutMs ?? 600_000,
      })
      return {
        agent: spec,
        status: 'finished',
        startedAt,
        finishedAt: Date.now(),
        exitCode: 0,
        output: typeof out === 'string' ? out : JSON.stringify(out),
      }
    } catch (err) {
      return { agent: spec, status: 'failed', startedAt, finishedAt: Date.now(), error: String(err) }
    }
  }

  const wt = await createWorktree(cwd, branch, worktreeRoot())
  if (!wt.ok || !wt.path) {
    return {
      agent: spec,
      status: 'failed',
      startedAt,
      finishedAt: Date.now(),
      error: wt.reason ?? 'worktree create failed',
    }
  }

  const r = await runLocalAgent(spec, wt.path, timeoutMs)
  const finishedAt = Date.now()
  const diff = await diffAgainst(wt.path, baseRef)

  return {
    agent: spec,
    status: r.ok ? 'finished' : 'failed',
    startedAt,
    finishedAt,
    exitCode: r.exitCode ?? undefined,
    output: r.stdout,
    error: r.ok ? undefined : r.reason,
    diff,
    worktreePath: wt.path,
    branch,
  }
}

export async function dispatch(sdk: ISdk, input: DispatchInput): Promise<DispatchResult & { run: RunState }> {
  if (!input.task || input.task.trim() === '') {
    throw new Error('task required')
  }
  if (!input.agents || input.agents.length === 0) {
    throw new Error('at least one agent required')
  }
  if (!input.cwd) throw new Error('cwd required')

  const runId = randomUUID()
  const baseRef = await currentBranch(input.cwd)

  const run: RunState = {
    id: runId,
    task: input.task,
    cwd: input.cwd,
    startedAt: Date.now(),
    agents: input.agents.map((a) => ({ agent: { ...a, prompt: a.prompt ?? input.task }, status: 'pending' })),
  }
  await writeRun(sdk, run)

  const settled = await Promise.all(
    run.agents.map((a, i) => runAgentInWorktree(sdk, i, a.agent, runId, input.cwd, baseRef, input.timeoutMs)),
  )

  for (let i = 0; i < settled.length; i++) {
    const state = settled[i]
    if (state.status === 'finished' && input.gates && input.gates.length > 0 && state.worktreePath) {
      state.gateResults = await runAllGates(sdk, input.gates, state.worktreePath)
    }
    run.agents[i] = state
  }

  run.finishedAt = Date.now()
  await writeRun(sdk, run)

  return { ok: true, run_id: runId, agents: run.agents.length, gates: input.gates?.length ?? 0, run }
}

export async function mergeRun(sdk: ISdk, run: RunState): Promise<MergeResult> {
  const losers: number[] = []
  let winner: { index: number; agent: AgentSpec; diff: string; branch?: string } | undefined

  for (let i = 0; i < run.agents.length; i++) {
    const a = run.agents[i]
    if (a.status !== 'finished') {
      losers.push(i)
      continue
    }
    const gatesOk = a.gateResults ? allPassed(a.gateResults) : true
    if (!gatesOk || !a.diff || a.diff.trim() === '') {
      losers.push(i)
      continue
    }
    if (!winner) {
      winner = { index: i, agent: a.agent, diff: a.diff, branch: a.branch }
    } else {
      losers.push(i)
    }
  }

  for (const i of losers) {
    const a = run.agents[i]
    if (a.worktreePath) {
      await removeWorktree(run.cwd, a.worktreePath).catch(() => undefined)
    }
  }

  run.winnerIndex = winner?.index
  await writeRun(sdk, run)

  if (!winner) {
    return { ok: false, reason: 'no agent passed all gates with a non-empty diff', run_id: run.id, losers }
  }
  return { ok: true, run_id: run.id, winner, losers }
}
