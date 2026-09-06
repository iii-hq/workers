import type { Host } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { activeTurnFromStatus, canActivateHarnessTurn } from './turn-status'

const TURN_STARTED_FN = 'iii::shell-ui::turn-started'
const TURN_COMPLETED_FN = 'iii::shell-ui::turn-completed'

interface TurnStartedEvent {
  session_id: string
  turn_id: string
}

interface TurnCompletedEvent extends TurnStartedEvent {
  terminal?: boolean
}

interface HarnessStatus {
  session_id?: string
  turn_id?: string
  status?: string
}

export interface HarnessTurnState {
  turnId: string | null
  active: boolean
  completedAtMs: number | null
}

/** Exact Harness turn identity for the active chat. The status read closes
    the small bind race and restores an already-running turn after mount.
    `scope` (the pane's function-id segment) keeps two panes beside the
    same chat on separate functions, so neither hears the other's binding. */
export function useHarnessTurn(
  host: Host,
  conversationId: string | null | undefined,
  scope: string,
): HarnessTurnState {
  const [state, setState] = useState<HarnessTurnState>({
    turnId: null,
    active: false,
    completedAtMs: null,
  })

  useEffect(() => {
    if (!conversationId) {
      setState({ turnId: null, active: false, completedAtMs: null })
      return
    }
    let cancelled = false
    let lifecycleGeneration = 0
    const completedTurnIds = new Set<string>()
    setState({ turnId: null, active: false, completedAtMs: null })

    const startedFn = `${TURN_STARTED_FN}::${scope}`
    const completedFn = `${TURN_COMPLETED_FN}::${scope}`
    const offHandler = host.iii.on<TurnStartedEvent>(startedFn, (event) => {
      if (event?.session_id !== conversationId || typeof event.turn_id !== 'string') return
      if (!canActivateHarnessTurn(event.turn_id, completedTurnIds)) return
      lifecycleGeneration += 1
      setState({ turnId: event.turn_id, active: true, completedAtMs: null })
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'harness::turn-started',
      function_id: `${startedFn}::${host.iii.browserId}`,
      config: { session_id: conversationId },
    })
    const offCompletedHandler = host.iii.on<TurnCompletedEvent>(completedFn, (event) => {
      if (
        event?.session_id !== conversationId ||
        typeof event.turn_id !== 'string' ||
        event.terminal === false
      ) {
        return
      }
      completedTurnIds.add(event.turn_id)
      lifecycleGeneration += 1
      const completedAtMs = Date.now()
      setState((previous) =>
        previous.turnId !== null && previous.turnId !== event.turn_id
          ? previous
          : { turnId: event.turn_id, active: false, completedAtMs },
      )
    })
    const offCompletedTrigger = host.iii.registerTrigger({
      type: 'harness::turn-completed',
      function_id: `${completedFn}::${host.iii.browserId}`,
      config: { session_id: conversationId },
    })

    const statusGeneration = lifecycleGeneration
    void host.iii
      .trigger<HarnessStatus>('harness::status', { session_id: conversationId })
      .then((status) => {
        if (cancelled) return
        const turnId = activeTurnFromStatus(
          status,
          conversationId,
          statusGeneration,
          lifecycleGeneration,
          completedTurnIds,
        )
        if (turnId !== null) setState({ turnId, active: true, completedAtMs: null })
      })
      .catch(() => {
        // Harness may be restarting; the live trigger remains authoritative.
      })

    return () => {
      cancelled = true
      try {
        offTrigger()
      } finally {
        offHandler()
      }
      try {
        offCompletedTrigger()
      } finally {
        offCompletedHandler()
      }
    }
  }, [host, conversationId, scope])

  return state
}
