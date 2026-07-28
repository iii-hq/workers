/**
 * The worker's `github::called` trigger — the one subscription both the
 * activity feed (index.tsx) and the live git graph (GitGraph.tsx) bind. Each
 * caller passes its own per-tab handler id so the two bindings never collide;
 * the `iii::` handler prefix keeps the per-event invocations span-suppressed
 * and out of the trace feed. See github/src/events.rs for the emitter side.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'

/** The worker's trigger type; see github/src/events.rs. */
export const CALLED_TYPE = 'github::called'

/** The `github::called` payload the worker emits (github/src/events.rs). */
export interface CalledEvent {
  function_id: string
  args_summary: string
  repo: string | null
  ok: boolean
  duration_ms: number
  result_summary: string
  /** Renderer discriminator for `result_preview`: list/object/text/diff/outcome. */
  kind: string
  /** The budgeted, projected result the worker carried in the event; `null`
   *  when there was nothing useful. Rendered by `kind` (see result-views.tsx). */
  result_preview: unknown
  timestamp: string
}

/**
 * Bind ONE tab-scoped subscription to `github::called` under `handlerId` for
 * the caller's lifetime and deliver each event to `onEvent`. The latest
 * `onEvent` is held in a ref so a changing callback identity never re-registers
 * the trigger (mirrors state/ui's `useStateEvents`). The binding is torn down
 * on unmount / host change — hot reload disposes it with the page.
 */
export function useGithubCalled(host: Host, handlerId: string, onEvent: (event: CalledEvent) => void): void {
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  useEffect(() => {
    const offHandler = host.iii.on<CalledEvent>(handlerId, (event) => onEventRef.current(event))
    const offTrigger = host.iii.registerTrigger({
      type: CALLED_TYPE,
      function_id: `${handlerId}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host, handlerId])
}
