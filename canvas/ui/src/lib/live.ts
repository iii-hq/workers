/**
 * Live canvas events: the page observes the state worker's
 * `state::on-changed`, filtered to scope "canvas".
 *
 * Canvas records live in state scope "canvas" (`record/<id>` plus the
 * `index` side key), so an agent creating or editing a canvas from chat
 * emits state events this page can stream — one list on mount, pushes
 * after, never polling. The observer's target is a tab-scoped handler (the
 * `iii::` handler prefix keeps per-event invocations span-suppressed) and
 * the observer is disposed with the page, same shape as the state and
 * editor pages.
 */

import { useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'

/** The observed function: one call per committed state change. */
const ON_CHANGED_FN = 'state::on-changed'

const EVENTS_FN = 'iii::canvas-ui::events'

export interface CanvasStateEvent {
  type: 'state'
  event_type: 'state:created' | 'state:updated' | 'state:deleted'
  scope: string
  key: string
}

/** Fires `onEvent` for every state write in scope "canvas". */
export function useCanvasStateEvents(
  host: Host,
  onEvent: (e: CanvasStateEvent) => void,
) {
  const handlerRef = useRef(onEvent)
  handlerRef.current = onEvent
  useEffect(() => {
    const offHandler = host.iii.on<CanvasStateEvent>(EVENTS_FN, (event) => {
      if (!event || event.type !== 'state' || event.scope !== 'canvas') return
      handlerRef.current(event)
    })
    const dispose = host.iii.observe(
      { function_id: ON_CHANGED_FN, filters: { 'input.scope': { $eq: 'canvas' } }, deliver: 'output' },
      `${EVENTS_FN}::${host.iii.browserId}`,
    )
    return () => {
      dispose()
      offHandler()
    }
  }, [host])
}
