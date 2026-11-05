import type { Host, ObserveSource, ObserverOptions } from '@iii-dev/console-ui'
import { useEffect, useId, useRef } from 'react'
import { BROWSER_ON_FRAME_EVENT_FUNCTION_ID } from './browser'

/**
 * Browser-local observers of the browser worker's per-tab event functions
 * (console/network/frame/handoff events) on the injected-UI `host` surface;
 * the session-list lifecycle feed goes through the shared `useWorkerLive`
 * (see page/useBrowserSessionsLive). Each observer is `host.iii.on(fnId)`
 * plus `host.iii.observe` with the target `<fnId>::<browserId>` (the host
 * registers the handler under the same namespaced id, so they match) and
 * `deliver: 'output'`, so the handler receives the event itself. The handler
 * base ids carry the `iii::` prefix so per-event invocations stay
 * span-suppressed and out of the trace feed; the per-mount `instanceId`
 * keeps two hook instances from colliding.
 *
 * Every observer is disposed with the tab and on unmount, so the injected
 * UI's observers die and revive with the page script.
 */

/** The event functions that take `{ session_id }` args (the worker encodes
 * nothing for a tab nobody watches); every other one filters on
 * `input.session_id`. */
const SESSION_ARGS_FUNCTIONS: ReadonlySet<string> = new Set([BROWSER_ON_FRAME_EVENT_FUNCTION_ID])

/** What to observe for `functionId`, narrowed to one tab when `sessionId` is set. */
export function browserEventSource(functionId: string, sessionId: string | null): ObserveSource {
  if (!sessionId) return { function_id: functionId, deliver: 'output' }
  if (SESSION_ARGS_FUNCTIONS.has(functionId)) {
    return { function_id: functionId, args: { session_id: sessionId }, deliver: 'output' }
  }
  return {
    function_id: functionId,
    filters: { 'input.session_id': { $eq: sessionId } },
    deliver: 'output',
  }
}

/** Frames are a stream of pictures: a viewer still painting one skips the
 * ones that arrive meanwhile instead of queueing them. */
export function browserEventObserverOptions(functionId: string): ObserverOptions | undefined {
  return functionId === BROWSER_ON_FRAME_EVENT_FUNCTION_ID ? { max_in_flight: 1 } : undefined
}

export interface UseBrowserEventOptions {
  host: Host
  enabled: boolean
  /** The event function to observe (e.g. `browser::on-session-started`). */
  functionId: string
  /** Base id for this observer's browser-local handler. */
  fnId: string
  /** Tab to narrow to; omit for every tab. */
  sessionId?: string | null
  onEvent: (payload: unknown) => void
}

/**
 * One observer of a browser event function, optionally narrowed to a tab.
 * Observes again when the tab changes and is disposed on unmount.
 */
export function useBrowserEvent(opts: UseBrowserEventOptions): void {
  const { host, enabled, functionId, fnId } = opts
  const sessionId = opts.sessionId ?? null
  const onEventRef = useRef(opts.onEvent)
  onEventRef.current = opts.onEvent

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled) return
    const offs: Array<() => void> = []
    const localFnId = `${fnId}::${instanceId}`
    try {
      offs.push(
        host.iii.on(localFnId, (payload: unknown) => {
          onEventRef.current(payload)
        }),
      )
      offs.push(
        host.iii.observe(
          browserEventSource(functionId, sessionId),
          `${localFnId}::${host.iii.browserId}`,
          browserEventObserverOptions(functionId),
        ),
      )
    } catch {
      // The client refused to observe; drop the observer.
    }

    return () => {
      for (const off of offs) off()
    }
  }, [host, enabled, functionId, sessionId, fnId, instanceId])
}

export interface UseBrowserSessionEventOptions extends UseBrowserEventOptions {
  /** The tab the observer is narrowed to. */
  sessionId: string | null
}

/**
 * One observer of a browser event function narrowed to a tab (console,
 * network, or picked events). Nothing is observed without a tab.
 */
export function useBrowserSessionEvent(
  opts: UseBrowserSessionEventOptions,
): void {
  useBrowserEvent({ ...opts, enabled: opts.enabled && !!opts.sessionId })
}
