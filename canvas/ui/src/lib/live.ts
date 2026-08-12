/**
 * Live canvas events over the state worker's `state` trigger type.
 *
 * Canvas records live in state scope "canvas" (`record/<id>` plus the
 * `index` side key), so an agent creating or editing a canvas from chat
 * emits state events this page can stream — one list on mount, pushes
 * after, never polling. The binding is tab-scoped Message-path (the
 * `iii::` handler prefix keeps per-event invocations span-suppressed)
 * and is GC'd with the tab, same shape as the state and editor pages.
 */

import { useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'

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
    const offTrigger = host.iii.registerTrigger({
      type: 'state',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: { scope: 'canvas' },
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host])
}
