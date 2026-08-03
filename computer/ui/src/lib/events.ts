import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useRef, useState } from 'react'
import { LIFECYCLE_TRIGGERS } from './computer'

/**
 * Page-local bindings to the computer worker's custom trigger types and its
 * screencast stream. Each binding is `host.iii.on(fnId)` plus
 * `host.iii.registerTrigger` targeting `<fnId>::<browserId>` (the SDK
 * registers the handler under the same namespaced id, so they match). The
 * handler base ids carry the `iii::` prefix so per-event invocations stay
 * span-suppressed and out of the trace feed; the per-mount `instanceId` keeps
 * two hook instances from colliding.
 *
 * Every binding is GC'd with the tab and unregistered on unmount, so the
 * injected UI's subscriptions die and revive with the page script.
 */

const LIFECYCLE_FN = 'iii::computer-ui::lifecycle'

export interface UseLifecycleEventsOptions {
  host: Host
  /** Only subscribe while the page is live. */
  enabled: boolean
  onEvent: () => void
}

export interface LifecycleSubscription {
  /**
   * True once both trigger bindings registered. While false (worker absent,
   * SDK failure) callers fall back to polling.
   */
  bound: boolean
}

/**
 * Feed of the session lifecycle trigger types, for surfaces that re-read the
 * session list on any change.
 */
export function useComputerLifecycleEvents(
  opts: UseLifecycleEventsOptions,
): LifecycleSubscription {
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
    for (const triggerType of LIFECYCLE_TRIGGERS) {
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
    setBound(registered === LIFECYCLE_TRIGGERS.length)

    return () => {
      setBound(false)
      for (const off of offs) off()
    }
  }, [host, enabled, instanceId])

  return { bound }
}

export interface UseComputerStreamOptions {
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
 * Subscribe to an iii stream (`type:'stream'`) for a session: the engine
 * pushes, the client appends. Rebinds when the group (session) changes and
 * unregisters on unmount.
 */
export function useComputerStream(opts: UseComputerStreamOptions): void {
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
