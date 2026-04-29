import { dispatch, mergeRun } from './dispatch.js'
import { iii } from './iii.js'
import { listRuns, readRun } from './state.js'
import type { DispatchInput, RunState } from './types.js'

const PUBLIC = { metadata: { public: true } }

iii.registerFunction(
  'conductor::dispatch',
  async (input: DispatchInput) => {
    const { run, ...summary } = await dispatch(iii, input)
    return { ...summary, agents: run.agents.length }
  },
  {
    description:
      'Fan out a task across N agents in parallel. Each gets its own worktree, runs verifier gates, and is recorded under a run_id.',
    ...PUBLIC,
  },
)

iii.registerFunction(
  'conductor::status',
  async (input: { run_id: string }): Promise<RunState | null> => readRun(iii, input.run_id),
  { description: 'Read the current state of a dispatch run.', ...PUBLIC },
)

iii.registerFunction('conductor::list', async (): Promise<RunState[]> => listRuns(iii), {
  description: 'List all dispatch runs.',
  ...PUBLIC,
})

iii.registerFunction(
  'conductor::merge',
  async (input: { run_id: string }) => {
    const run = await readRun(iii, input.run_id)
    if (!run) return { ok: false, reason: 'run not found', run_id: input.run_id, losers: [] }
    return mergeRun(iii, run)
  },
  {
    description:
      'Pick the winning agent for a run (first to pass all gates with a non-empty diff). Cleans up loser worktrees.',
    ...PUBLIC,
  },
)

console.info('Conductor functions registered: dispatch, status, list, merge')
