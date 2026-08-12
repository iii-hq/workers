/* Live fleet for the target picker: one `sandbox::list` read on mount,
   then pushed `{ kind: "fleet" }` snapshots over a tab-scoped
   `sandbox-code-runner::event` trigger (the `iii::` handler-id prefix
   keeps per-event invocations span-suppressed; `host.iii.on` namespaces
   the handler `::<browserId>`, which the trigger's function_id repeats
   to address it). The code-runner worker may be absent — the mount read
   plus the manual refresh affordance stand alone then. A `sandbox::list`
   that reads as function-not-found means no sandbox daemon at all:
   status `absent`, and the picker hides entirely. */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import {
  createFleetSequencer,
  type FleetState,
  isFunctionNotFound,
  parseFleetEvent,
  parseSandboxList,
} from './sandbox'

const FLEET_HANDLER_FN = 'iii::shell-ui::fleet'
const FLEET_EVENT_TRIGGER = 'sandbox-code-runner::event'

export function useSandboxFleet(host: Host): {
  fleet: FleetState
  refreshFleet: () => void
} {
  const [fleet, setFleet] = useState<FleetState>({
    status: 'loading',
    sandboxes: [],
  })
  const aliveRef = useRef(true)
  useEffect(() => {
    aliveRef.current = true
    return () => {
      aliveRef.current = false
    }
  }, [])

  // Overlapping reads resolve in any order — only the latest request may
  // apply, and a pushed snapshot outranks every read still in flight.
  const seqRef = useRef(createFleetSequencer())

  const refreshFleet = useCallback(() => {
    const seq = seqRef.current.begin()
    host.iii
      .trigger('sandbox::list', {})
      .then((out) => {
        if (!aliveRef.current || !seqRef.current.isCurrent(seq)) return
        setFleet({ status: 'ready', sandboxes: parseSandboxList(out) })
      })
      .catch((err: unknown) => {
        if (!aliveRef.current || !seqRef.current.isCurrent(seq)) return
        setFleet((prev) => ({
          status: isFunctionNotFound(errorMessage(err)) ? 'absent' : 'error',
          // A transient failure keeps the last-known fleet — the picker
          // must not dump a live selection over one bad read.
          sandboxes: prev.sandboxes,
        }))
      })
  }, [host])

  useEffect(() => {
    refreshFleet()
  }, [refreshFleet])

  useEffect(() => {
    let offHandler: (() => void) | null = null
    let offTrigger: (() => void) | null = null
    try {
      offHandler = host.iii.on(FLEET_HANDLER_FN, (payload: unknown) => {
        const snapshot = parseFleetEvent(payload)
        if (!snapshot || !aliveRef.current) return
        // The snapshot is fresher than any older list read in flight.
        seqRef.current.invalidate()
        setFleet({
          status: snapshot.daemonAbsent ? 'absent' : 'ready',
          sandboxes: snapshot.sandboxes,
        })
      })
      offTrigger = host.iii.registerTrigger({
        type: FLEET_EVENT_TRIGGER,
        function_id: `${FLEET_HANDLER_FN}::${host.iii.browserId}`,
        config: {},
      })
    } catch {
      // Code-runner absent or the trigger type unregistered — the mount
      // read and the refresh affordance remain.
    }
    return () => {
      offTrigger?.()
      offHandler?.()
    }
  }, [host])

  return { fleet, refreshFleet }
}
