/* The chat footer's review of the newest turn. Its rows fill in one file at
   a time; kept in their own component, each fill re-renders this, not the
   whole page. */

import type { Host } from '@iii-dev/console-ui'
import { useShellReviewSummaryBridge } from './review-summary-store'
import type { SessionTurn, SessionTurnSummary } from './turns'
import { useTurnSummary } from './use-turn-summary'

export function TurnReviewBridge({
  host,
  root,
  turn,
  turnCache,
  epoch,
  sessionId,
  sourceId,
  onSelectFile,
}: {
  host: Host
  root: string | null
  turn: SessionTurnSummary | null
  turnCache: { get(turnId: string): Promise<SessionTurn | null>; forget(turnId: string): void }
  /** Bumps when the disk changed; the totals follow. */
  epoch: number
  sessionId: string | null | undefined
  sourceId: string
  onSelectFile(path: string): void
}) {
  const files = useTurnSummary(host, root, turn, turnCache, epoch)
  useShellReviewSummaryBridge({ sessionId, sourceId, turnId: turn?.turn_id ?? null, files, onSelectFile })
  return null
}
