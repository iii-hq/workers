import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { activeTurnFromStatus, canActivateHarnessTurn } from './turn-status'

const TURN_STARTED_FN = 'iii::shell-ui::turn-started'
const TURN_COMPLETED_FN = 'iii::shell-ui::turn-completed'
const TURNS_CHANGED_FN = 'iii::shell-ui::turns-changed'
/** `shell::turns::changed` already folds a burst into one event per 200 ms;
    this folds the few left into one list read. */
export const TURNS_CHANGED_DEBOUNCE_MS = 150

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

/** Calls `onChange` when the ide worker stores a new record of the chat's
    change history (`shell::turns::changed`): a turn opened or closed, a
    file change recorded. Debounced; nothing is read on a timer. `scope`
    keeps two panes beside one chat on separate functions. */
export function useTurnsChanged(
  host: Host,
  conversationId: string | null | undefined,
  scope: string,
  onChange: () => void,
): void {
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange
  useEffect(() => {
    if (!conversationId) return
    let timer: ReturnType<typeof setTimeout> | null = null
    const functionId = `${TURNS_CHANGED_FN}::${scope}`
    const offHandler = host.iii.on<{ session_id?: string }>(functionId, (event) => {
      if (event?.session_id !== conversationId || timer !== null) return
      timer = setTimeout(() => {
        timer = null
        onChangeRef.current()
      }, TURNS_CHANGED_DEBOUNCE_MS)
    })
    let offTrigger: () => void = () => {}
    try {
      offTrigger = host.iii.registerTrigger({
        type: 'shell::turns::changed',
        function_id: `${functionId}::${host.iii.browserId}`,
        config: { session_id: conversationId },
      })
    } catch {
      // No ide worker: the list still follows turn boundaries.
    }
    return () => {
      if (timer !== null) clearTimeout(timer)
      try {
        offTrigger()
      } finally {
        offHandler()
      }
    }
  }, [host, conversationId, scope])
}
