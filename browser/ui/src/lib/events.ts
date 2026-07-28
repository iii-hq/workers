import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useRef, useState } from 'react'
import { BROWSER_LIFECYCLE_TRIGGERS } from './browser'

/**
 * Browser-local bindings to the browser worker's custom trigger types and
 * screencast stream, ported to the injected-UI `host` surface. Each binding
 * is `host.iii.on(fnId)` plus `host.iii.registerTrigger` targeting
 * `<fnId>::<browserId>` (the SDK registers the handler under the same
 * namespaced id, so they match). The handler base ids carry the `iii::`
 * prefix so per-event invocations stay span-suppressed and out of the trace
 * feed; the per-mount `instanceId` keeps two hook instances from colliding.
 *
 * Every binding is GC'd with the tab and unregistered on unmount, so the
 * injected UI's subscriptions die and revive with the page script.
 */

const LIFECYCLE_FN = 'iii::browser-ui::lifecycle'

export interface UseBrowserLifecycleEventsOptions {
  host: Host
  /** Only subscribe while the page is live. */
  enabled: boolean
  onEvent: () => void
}

export interface BrowserLifecycleSubscription {
  /**
   * True once all three trigger bindings registered. While false (worker
   * absent, SDK failure) callers fall back to polling.
   */
  bound: boolean
}

/**
 * Page-scoped feed of the session lifecycle trigger types
 * (session-started / session-stopped / navigated), for surfaces that re-read
 * the session list on any change.
 */
export function useBrowserLifecycleEvents(
  opts: UseBrowserLifecycleEventsOptions,
): BrowserLifecycleSubscription {
  const { host, enabled } = opts
  const onEventRef = useRef(opts.onEvent)
  onEventRef.current = opts.onEvent

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  const [bound, setBound] = useState(false)

  useEffect(() => {
    if (!enabled) {
      setBound(false)
      return
    }
    const offs: Array<() => void> = []
    let registered = 0
    for (const triggerType of BROWSER_LIFECYCLE_TRIGGERS) {
      const suffix = triggerType.replace(/[^a-zA-Z0-9]/g, '-')
      const localFnId = `${LIFECYCLE_FN}::${suffix}::${instanceId}`
      try {
        offs.push(
          host.iii.on(localFnId, () => {
            onEventRef.current()
          }),
        )
        offs.push(
          host.iii.registerTrigger({
            type: triggerType,
            function_id: `${localFnId}::${host.iii.browserId}`,
            config: {},
          }),
        )
        registered += 1
      } catch {
        // Worker absent or trigger type unregistered; drop the binding.
      }
    }
    setBound(registered === BROWSER_LIFECYCLE_TRIGGERS.length)

    return () => {
      setBound(false)
      for (const off of offs) off()
    }
  }, [host, enabled, instanceId])

  return { bound }
}

export interface UseBrowserSessionEventOptions {
  host: Host
  enabled: boolean
  /** Trigger type to bind (e.g. `browser::console-event`). */
  triggerType: string
  /** Session the binding filters to (worker-side `session_id` filter). */
  sessionId: string | null
  /** Base id for this binding's browser-local handler. */
  fnId: string
  onEvent: (payload: unknown) => void
}

/**
 * One session-filtered binding to a browser trigger type (console-event,
 * network-event, or picked). Rebinds when the session changes and
 * unregisters on unmount.
 */
export function useBrowserSessionEvent(
  opts: UseBrowserSessionEventOptions,
): void {
  const { host, enabled, triggerType, sessionId, fnId } = opts
  const onEventRef = useRef(opts.onEvent)
  onEventRef.current = opts.onEvent

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled || !sessionId) return
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
          config: { session_id: sessionId },
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

export interface UseBrowserStreamOptions {
  host: Host
  enabled: boolean
  /** iii stream name to subscribe to. */
  streamName: string
  /** Stream group (the session id for per-session streams). */
  groupId: string | null
  /** Base id for this binding's browser-local handler. */
  fnId: string
  onFrame: (payload: unknown) => void
}

/**
 * Subscribe to an iii stream (`type:'stream'`) for a session, the same
 * engine-pushes / client-appends pattern the Traces view uses. Rebinds when
 * the stream group (session) changes and unregisters on unmount.
 */
export function useBrowserStream(opts: UseBrowserStreamOptions): void {
  const { host, enabled, streamName, groupId, fnId } = opts
  const onFrameRef = useRef(opts.onFrame)
  onFrameRef.current = opts.onFrame

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled || !groupId) return
    const offs: Array<() => void> = []
    const localFnId = `${fnId}::${instanceId}`
    try {
      offs.push(
        host.iii.on(localFnId, (payload: unknown) => {
          onFrameRef.current(payload)
        }),
      )
      offs.push(
        host.iii.registerTrigger({
          type: 'stream',
          function_id: `${localFnId}::${host.iii.browserId}`,
          config: { stream_name: streamName, group_id: groupId },
        }),
      )
    } catch {
      // Stream not available; the seed read is the fallback.
    }

    return () => {
      for (const off of offs) off()
    }
  }, [host, enabled, streamName, groupId, fnId, instanceId])
}
