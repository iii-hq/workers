import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { activeTurnFromStatus, canActivateHarnessTurn } from './turn-status'

const TURN_STARTED_FN = 'iii::shell-ui::turn-started'
const TURN_COMPLETED_FN = 'iii::shell-ui::turn-completed'
const PREPARE_TURN_FN = 'iii::shell-ui::prepare-turn'

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

export interface HarnessPreTurnEvent {
  point: 'pre_turn'
  session_id: string
  turn_id: string
  step: number
  metadata?: Record<string, unknown>
}

export interface HarnessTurnState {
  turnId: string | null
  active: boolean
  completedAtMs: number | null
}

/**
 * Run an async browser-local preparation inside Harness's awaited pre-turn
 * chain. Unlike the fire-and-forget turn-started event, this is a real barrier:
 * the model cannot write files until every matching preparation has returned.
 *
 * The binding is global because Harness hook trigger types are global. The
 * browser handler scopes itself to the active session and fails open so a
 * disconnected review pane can never prevent a turn from running.
 */
export function useHarnessPreTurn(
  host: Host,
  conversationId: string | null | undefined,
  instanceId: string,
  onPrepare: (event: HarnessPreTurnEvent) => void | Promise<void>,
) {
  const prepareRef = useRef(onPrepare)
  prepareRef.current = onPrepare

  useEffect(() => {
    if (!conversationId) return
    const suffix = instanceId.replace(/[^a-zA-Z0-9_-]/g, '_') || 'page'
    const functionId = `${PREPARE_TURN_FN}:${suffix}`
    const offHandler = host.iii.on<HarnessPreTurnEvent>(functionId, async (event) => {
      if (event?.session_id !== conversationId || typeof event.turn_id !== 'string') return
      await prepareRef.current(event)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'harness::hook::pre-turn',
      function_id: `${functionId}::${host.iii.browserId}`,
      config: {
        sessions: [conversationId],
        priority: -100,
        timeout_ms: 30_000,
        on_error: 'fail_open',
      },
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host, conversationId, instanceId])
}

/** Exact Harness turn identity for the active chat. The status read closes
    the small bind race and restores an already-running turn after mount. */
export function useHarnessTurn(
  host: Host,
  conversationId: string | null | undefined,
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

    const offHandler = host.iii.on<TurnStartedEvent>(TURN_STARTED_FN, (event) => {
      if (event?.session_id !== conversationId || typeof event.turn_id !== 'string') return
      if (!canActivateHarnessTurn(event.turn_id, completedTurnIds)) return
      lifecycleGeneration += 1
      setState({ turnId: event.turn_id, active: true, completedAtMs: null })
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'harness::turn-started',
      function_id: `${TURN_STARTED_FN}::${host.iii.browserId}`,
      config: { session_id: conversationId },
    })
    const offCompletedHandler = host.iii.on<TurnCompletedEvent>(TURN_COMPLETED_FN, (event) => {
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
      function_id: `${TURN_COMPLETED_FN}::${host.iii.browserId}`,
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
  }, [host, conversationId])

  return state
}
