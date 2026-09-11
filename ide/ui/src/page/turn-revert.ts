/* Rolling a Harness turn back through the worker's own change history
   (`shell::turns::revert`): the worker restores pre-image bodies from its
   blob store, so the page only names the turn and, optionally, the files. */

import type { Host } from '@iii-dev/console-ui'

export interface RevertFileResult {
  path: string
  kind: string
  action: 'restored' | 'removed' | 'moved-back' | 'skipped' | 'unavailable' | string
  success: boolean
  error?: string | null
}

export interface RevertResult {
  session_id: string
  turn_id: string
  results: RevertFileResult[]
  reverted: number
  failed: number
}

export async function revertTurn(
  host: Host,
  sessionId: string,
  turnId: string,
  paths?: readonly string[],
): Promise<RevertResult> {
  return host.iii.trigger<RevertResult>('shell::turns::revert', {
    session_id: sessionId,
    turn_id: turnId,
    ...(paths !== undefined ? { paths: [...paths] } : {}),
  })
}

/** A one-line account of a revert for a status note. */
export function describeRevert(result: RevertResult): string {
  const restored = result.results.filter((r) => r.success && r.action !== 'skipped').length
  if (result.failed === 0) {
    return restored === 0
      ? 'nothing to revert'
      : `reverted ${restored} ${restored === 1 ? 'file' : 'files'}`
  }
  const first = result.results.find((r) => !r.success)
  const detail = first?.error ? `: ${first.error}` : ''
  return `reverted ${restored}, ${result.failed} could not be reverted${detail}`
}
