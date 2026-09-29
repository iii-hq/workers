import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useRef } from 'react'

/**
 * Browser-local bindings to the browser worker's per-tab trigger types
 * (console/network/frame/handoff events) on the injected-UI `host` surface;
 * the session-list lifecycle feed goes through the shared `useWorkerLive`
 * (see page/useBrowserSessionsLive). Each binding
 * is `host.iii.on(fnId)` plus `host.iii.registerTrigger` targeting
 * `<fnId>::<browserId>` (the SDK registers the handler under the same
 * namespaced id, so they match). The handler base ids carry the `iii::`
 * prefix so per-event invocations stay span-suppressed and out of the trace
 * feed; the per-mount `instanceId` keeps two hook instances from colliding.
 *
 * Every binding is GC'd with the tab and unregistered on unmount, so the
 * injected UI's subscriptions die and revive with the page script.
 */

export interface UseBrowserEventOptions {
  host: Host
  enabled: boolean
  /** Trigger type to bind (e.g. `browser::session-started`). */
  triggerType: string
  /** Base id for this binding's browser-local handler. */
  fnId: string
  /** Session to filter to (worker-side `session_id` filter); omit for every session. */
  sessionId?: string | null
  onEvent: (payload: unknown) => void
}

/**
 * One binding to a browser trigger type, optionally filtered to a session.
 * Rebinds when the session changes and unregisters on unmount.
 */
export function useBrowserEvent(opts: UseBrowserEventOptions): void {
  const { host, enabled, triggerType, fnId } = opts
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
        host.iii.registerTrigger({
          type: triggerType,
          function_id: `${localFnId}::${host.iii.browserId}`,
          config: sessionId ? { session_id: sessionId } : {},
        }),
      )
    } catch {
      // Worker absent or trigger type unregistered; drop the binding.
    }

    return () => {
      for (const off of offs) off()
    }
  }, [host, enabled, triggerType, sessionId, fnId, instanceId])
}

export interface UseBrowserSessionEventOptions extends UseBrowserEventOptions {
  /** Session the binding filters to (worker-side `session_id` filter). */
  sessionId: string | null
}

/**
 * One session-filtered binding to a browser trigger type (console-event,
 * network-event, or picked). Nothing is bound without a session.
 */
export function useBrowserSessionEvent(
  opts: UseBrowserSessionEventOptions,
): void {
  useBrowserEvent({ ...opts, enabled: opts.enabled && !!opts.sessionId })
}
