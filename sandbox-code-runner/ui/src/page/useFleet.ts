/**
 * React face of the fleet store, plus the live-event binding: one
 * tab-scoped Message-path trigger on `sandbox-code-runner::event`
 * (same pattern as memory's useMemoryLive / worktree's useWorktreesLive —
 * an `iii::`-prefixed handler id keeps the per-event invocations
 * span-suppressed; `host.iii.on` namespaces the handler `::<browserId>`,
 * which the trigger's function_id repeats to address it). Any event pokes
 * an immediate store refresh; pushed fleet snapshots apply directly and
 * the page honest when the binding is unavailable. Both the handler and
 * the trigger are unregistered on unmount.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { createFleetStore, type FleetState, type FleetStore } from './store'

const EVENTS_FN = 'iii::sandbox-code-runner-ui::events'
const EVENT_TRIGGER_TYPE = 'sandbox-code-runner::event'

export interface Fleet {
  state: FleetState
  /** Manual refresh — re-reads the catalog too. */
  refresh: () => void
  /** True while the live trigger binding is registered. */
  live: boolean
}

export function useFleet(host: Host): Fleet {
  const storeRef = useRef<FleetStore | null>(null)
  if (!storeRef.current) {
    storeRef.current = createFleetStore((functionId, payload) =>
      host.iii.trigger(functionId, payload),
    )
  }
  const store = storeRef.current

  const [state, setState] = useState<FleetState>(store.getState())
  const [live, setLive] = useState(false)

  useEffect(() => {
    const off = store.subscribe(() => setState(store.getState()))
    store.start()
    setState(store.getState())
    return () => {
      off()
      store.stop()
    }
  }, [store])

  useEffect(() => {
    let offHandler: (() => void) | null = null
    let offTrigger: (() => void) | null = null
    try {
      offHandler = host.iii.on(EVENTS_FN, (payload: unknown) => {
        // Fleet snapshots apply straight from the wire (zero read-back);
        // runtime-kind events cost one debounced list_runtimes read.
        if (!store.applyFleetEvent(payload)) store.refreshRuntimes()
      })
      offTrigger = host.iii.registerTrigger({
        type: EVENT_TRIGGER_TYPE,
        function_id: `${EVENTS_FN}::${host.iii.browserId}`,
        config: {},
      })
      setLive(true)
    } catch {
      // Worker absent or the trigger type unregistered. This page ships
      // WITH the worker, so in practice this never fires; the manual
      // refresh button remains the fallback.
    }
    return () => {
      setLive(false)
      offTrigger?.()
      offHandler?.()
    }
  }, [host, store])

  const refresh = useCallback(() => store.refresh(true), [store])

  return { state, refresh, live }
}
