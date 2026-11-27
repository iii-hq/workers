import { useEffect, useState } from 'react'
import { type AutocompactMark, snapshotMark } from '@/lib/autocompact'
import type { ChatBackend } from '@/lib/backend/types'

interface TurnState {
  sessionId: string
  running: boolean
  /** What the meter read when the turn started (the harness's step 0). */
  frozen: AutocompactMark | null
  /** The turn's own snapshot, once a step wrote one. */
  snapshot: AutocompactMark | null
}

/**
 * The auto-compact the meter's tick, its label and the header chip show
 * (chat-projects §3.3.3):
 * - **between turns**, `between`: the shared pure function over the
 *   setting chain and the last `usable` (D164), so a changed setting shows
 *   at once;
 * - **while a turn runs**, the value the turn froze: its snapshot once a
 *   step writes one (`observe`), and until then what `between` read when
 *   the turn started. A setting changed during the turn shows when it ends,
 *   as the harness applies it then (finalize resolves again), and the
 *   meter agrees with the injected harness chip meanwhile.
 */
export function useTurnAutocompact(input: {
  sessionId: string
  running: boolean
  between: AutocompactMark | null
  observe?: ChatBackend['onTurnAutocompact']
}): AutocompactMark | null {
  const { sessionId, running, between, observe } = input
  const [turn, setTurn] = useState<TurnState>(() => ({
    sessionId,
    running,
    frozen: running ? between : null,
    snapshot: null,
  }))
  const current = turn.sessionId === sessionId && turn.running === running
  // A turn starts, ends, or the chat changes: freeze (or drop) the value
  // during render, so no frame shows the live one.
  if (!current) {
    setTurn({
      sessionId,
      running,
      frozen: running ? between : null,
      snapshot: null,
    })
  }

  useEffect(() => {
    if (!running || !observe) return
    return observe(sessionId, (value) => {
      setTurn((state) =>
        state.sessionId === sessionId && state.running
          ? { ...state, snapshot: snapshotMark(value) }
          : state,
      )
    })
  }, [running, observe, sessionId])

  if (!running || !current) return between
  return turn.snapshot ?? turn.frozen
}
