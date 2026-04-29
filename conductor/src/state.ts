import type { ISdk } from 'iii-sdk'
import type { RunState } from './types.js'

const SCOPE = 'conductor'
const RUN_PREFIX = 'runs::'

export async function writeRun(sdk: ISdk, run: RunState): Promise<void> {
  await sdk.trigger<{ scope: string; key: string; value: RunState }, RunState>({
    function_id: 'state::set',
    payload: { scope: SCOPE, key: `${RUN_PREFIX}${run.id}`, value: run },
  })
}

export async function readRun(sdk: ISdk, runId: string): Promise<RunState | null> {
  return sdk.trigger<{ scope: string; key: string }, RunState | null>({
    function_id: 'state::get',
    payload: { scope: SCOPE, key: `${RUN_PREFIX}${runId}` },
  })
}

export async function listRuns(sdk: ISdk): Promise<RunState[]> {
  const all = await sdk.trigger<{ scope: string }, RunState[]>({
    function_id: 'state::list',
    payload: { scope: SCOPE },
  })
  return all ?? []
}
