/**
 * The worker's `github::on-called` function — the one source both the
 * activity feed (index.tsx) and the live git graph (GitGraph.tsx) observe.
 * Each caller passes its own per-tab handler id so the two observers never
 * collide; the `iii::` handler prefix keeps the per-event invocations
 * span-suppressed and out of the trace feed. See github/src/events.rs for the
 * worker side.
 */

import type { Host } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'

/** The function the worker calls after each github function; see github/src/events.rs. */
export const ON_CALLED_FN = 'github::on-called'

/** The `github::on-called` event: what its handler returns (github/src/events.rs). */
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
 * Observe `github::on-called` with ONE tab-scoped handler under `handlerId`
 * for the caller's lifetime and deliver each event to `onEvent`. The latest
 * `onEvent` is held in a ref so a changing callback identity never observes
 * again (mirrors state/ui's `useStateEvents`). The observer is disposed on
 * unmount / host change — hot reload disposes it with the page.
 */
export function useGithubCalled(host: Host, handlerId: string, onEvent: (event: CalledEvent) => void): void {
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  useEffect(() => {
    const offHandler = host.iii.on<CalledEvent>(handlerId, (event) => onEventRef.current(event))
    const offObserver = host.iii.observe(
      { function_id: ON_CALLED_FN, deliver: 'output' },
      `${handlerId}::${host.iii.browserId}`,
    )
    return () => {
      offObserver()
      offHandler()
    }
  }, [host, handlerId])
}
