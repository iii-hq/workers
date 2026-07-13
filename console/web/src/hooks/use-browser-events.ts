import { useEffect, useId, useRef, useState } from 'react'
import { BROWSER_LIFECYCLE_TRIGGERS } from '@/lib/browser'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Browser-local bindings to the browser worker's custom trigger types,
 * following the worktree-events pattern: `client.on(fnId)` plus
 * `client.registerTrigger` targeting `<fnId>::<browserId>`. The per-mount
 * `instanceId` keeps two hook instances from colliding on the registered
 * function name.
 *
 * Registration is wrapped in try/catch: with the worker absent (its trigger
 * types unregistered) the binding drops silently; callers already gate
 * `enabled` on browser presence, this is defense-in-depth for races.
 */

const LIFECYCLE_FN = 'console::browser-lifecycle'

export interface UseBrowserLifecycleEventsOptions {
  /** Only subscribe on the real backend with the browser worker present. */
  enabled: boolean
  /** Fired for every session lifecycle event, with its trigger type. */
  onEvent: (triggerType: string, payload: unknown) => void
}

export interface BrowserLifecycleSubscription {
  /**
   * True once all three trigger bindings registered. While false (probe in
   * flight, worker absent, SDK failure) callers fall back to polling.
   */
  bound: boolean
}

/**
 * Page-scoped feed of the session lifecycle trigger types
 * (session-started / session-stopped / navigated), for surfaces that
 * re-read the session list on any change.
 */
export function useBrowserLifecycleEvents(
  opts: UseBrowserLifecycleEventsOptions,
): BrowserLifecycleSubscription {
  const { enabled } = opts
  const onEventRef = useRef(opts.onEvent)
  onEventRef.current = opts.onEvent

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  const [bound, setBound] = useState(false)

  useEffect(() => {
    if (!enabled) {
      setBound(false)
      return
    }
    let cancelled = false
    const offs: Array<() => void> = []

    void (async () => {
      let client: Awaited<ReturnType<typeof getIiiClient>>
      try {
        client = await getIiiClient()
      } catch {
        // Client startup failed; stay unbound so callers fall back to polling.
        return
      }
      if (cancelled) return
      let registered = 0
      for (const triggerType of BROWSER_LIFECYCLE_TRIGGERS) {
        const suffix = triggerType.replace(/[^a-zA-Z0-9]/g, '-')
        const localFnId = `${LIFECYCLE_FN}::${suffix}::${instanceId}`
        try {
          offs.push(
            client.on(localFnId, (payload: unknown) => {
              onEventRef.current(triggerType, payload)
            }),
          )
          offs.push(
            client.registerTrigger({
              type: triggerType,
              function_id: `${localFnId}::${client.browserId}`,
              config: {},
            }),
          )
          registered += 1
        } catch {
          // Worker absent or trigger type unregistered; drop the binding.
        }
      }
      if (!cancelled) {
        setBound(registered === BROWSER_LIFECYCLE_TRIGGERS.length)
      }
    })()

    return () => {
      cancelled = true
      setBound(false)
      for (const off of offs) off()
    }
  }, [enabled, instanceId])

  return { bound }
}

export interface UseBrowserSessionEventOptions {
  /** Only subscribe on the real backend with the browser worker present. */
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
 * One session-filtered binding to a browser trigger type (console-event or
 * picked). Rebinds when the session changes and unregisters on unmount.
 */
export function useBrowserSessionEvent(
  opts: UseBrowserSessionEventOptions,
): void {
  const { enabled, triggerType, sessionId, fnId } = opts
  const onEventRef = useRef(opts.onEvent)
  onEventRef.current = opts.onEvent

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled || !sessionId) return
    let cancelled = false
    const offs: Array<() => void> = []

    void (async () => {
      let client: Awaited<ReturnType<typeof getIiiClient>>
      try {
        client = await getIiiClient()
      } catch {
        return
      }
      if (cancelled) return
      const localFnId = `${fnId}::${instanceId}`
      try {
        offs.push(
          client.on(localFnId, (payload: unknown) => {
            onEventRef.current(payload)
          }),
        )
        offs.push(
          client.registerTrigger({
            type: triggerType,
            function_id: `${localFnId}::${client.browserId}`,
            config: { session_id: sessionId },
          }),
        )
      } catch {
        // Worker absent or trigger type unregistered; drop the binding.
      }
    })()

    return () => {
      cancelled = true
      for (const off of offs) off()
    }
  }, [enabled, triggerType, sessionId, fnId, instanceId])
}

export interface UseBrowserStreamOptions {
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
  const { enabled, streamName, groupId, fnId } = opts
  const onFrameRef = useRef(opts.onFrame)
  onFrameRef.current = opts.onFrame

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled || !groupId) return
    let cancelled = false
    const offs: Array<() => void> = []

    void (async () => {
      let client: Awaited<ReturnType<typeof getIiiClient>>
      try {
        client = await getIiiClient()
      } catch {
        return
      }
      if (cancelled) return
      const localFnId = `${fnId}::${instanceId}`
      try {
        offs.push(
          client.on(localFnId, (payload: unknown) => {
            onFrameRef.current(payload)
          }),
        )
        offs.push(
          client.registerTrigger({
            type: 'stream',
            function_id: `${localFnId}::${client.browserId}`,
            config: { stream_name: streamName, group_id: groupId },
          }),
        )
      } catch {
        // Stream not available; the seed read is the fallback.
      }
    })()

    return () => {
      cancelled = true
      for (const off of offs) off()
    }
  }, [enabled, streamName, groupId, fnId, instanceId])
}
