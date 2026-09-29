import type { Host } from '@iii-dev/console-ui'
import { useEffect, useId, useRef } from 'react'

/**
 * Page-local binding to the computer worker's screencast stream (the session
 * lifecycle feed goes through the shared `useWorkerLive`, see page/index).
 * The binding is `host.iii.on(fnId)` plus `host.iii.registerTrigger`
 * targeting `<fnId>::<browserId>` (the SDK registers the handler under the
 * same namespaced id, so they match). The handler base id carries the `iii::`
 * prefix so per-frame invocations stay span-suppressed and out of the trace
 * feed; the per-mount `instanceId` keeps two hook instances from colliding.
 *
 * The binding is GC'd with the tab and unregistered on unmount, so the
 * injected UI's subscription dies and revives with the page script.
 */

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
    } catch (err) {
      // Stream not available; the seed read is the only paint. Say so — a
      // silent catch here looks identical to a desktop that never changes.
      console.warn('[computer-ui] stream binding failed', streamName, err)
    }

    return () => {
      for (const off of offs) off()
    }
  }, [host, enabled, streamName, groupId, fnId, instanceId])
}
