import type { TimelineMessageListRow } from '@/components/observer-activity/grouping'
import type { ThoughtMessage } from '@/types/chat'

const isThoughtRow = (
  row: TimelineMessageListRow,
): row is Extract<TimelineMessageListRow, { kind: 'message' }> & {
  message: ThoughtMessage
} => row.kind === 'message' && row.message.role === 'thought'

/**
 * Fold a run of adjacent thought rows into one. Thoughts stay beside the
 * activity group they bracket while it absorbs later calls, so a long turn of
 * think → call → think → call leaves every thought stacked under the group,
 * each a "Thought" line of its own. One fold reads the run in order instead.
 *
 * The run keeps its first thought's id, so the row's key, measured height and
 * open state survive as later thoughts join it. It is live while its newest
 * thought streams, and its duration is the sum of its parts.
 */
export function mergeThoughtRuns(
  rows: readonly TimelineMessageListRow[],
): TimelineMessageListRow[] {
  const output: TimelineMessageListRow[] = []
  let run: ThoughtMessage[] = []

  const flush = () => {
    if (run.length === 1) {
      output.push({ kind: 'message', message: run[0] })
    } else if (run.length > 1) {
      const last = run[run.length - 1]
      const merged: ThoughtMessage = {
        ...run[0],
        content: run
          .map((thought) => thought.content.trim())
          .filter(Boolean)
          .join('\n\n'),
        durationMs: run.reduce((sum, thought) => sum + thought.durationMs, 0),
      }
      if (last.streaming) merged.streaming = true
      else delete merged.streaming
      output.push({ kind: 'message', message: merged })
    }
    run = []
  }

  for (const row of rows) {
    if (isThoughtRow(row)) {
      run.push(row.message)
      continue
    }
    flush()
    output.push(row)
  }
  flush()
  return output
}
