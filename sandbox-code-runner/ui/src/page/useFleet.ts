/**
 * React face of the fleet store, plus the live events: one tab-scoped
 * observer of `sandbox-code-runner::on-event` (same pattern as memory's
 * useMemoryLive / worktree's page — an `iii::`-prefixed handler id keeps the
 * per-event invocations span-suppressed; `host.iii.on` namespaces the
 * handler `::<browserId>`, which the observer's target repeats to address
 * it). The observer also keeps the worker's fleet watcher running while the
 * page is open. Pushed fleet snapshots apply directly; any other event kind
 * costs one debounced `list_runtimes` re-read. The live/static pill keeps
 * the page honest when the observer is unavailable. The handler is
 * unregistered and the observer disposed on unmount.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { createFleetStore, type FleetState, type FleetStore } from './store'

const EVENTS_FN = 'iii::sandbox-code-runner-ui::events'
const ON_EVENT_FN = 'sandbox-code-runner::on-event'

export interface Fleet {
  state: FleetState
  /** Manual refresh — re-reads the catalog too. */
  refresh: () => void
  /** True while the live observer is in place. */
  live: boolean
}

export function useFleet(host: Host): Fleet {
  // The store is created once, so its call closure reads the host off
  // a ref — a re-rendered host prop must not strand it on the first one.
  const hostRef = useRef(host)
  useEffect(() => {
    hostRef.current = host
  }, [host])
  const storeRef = useRef<FleetStore | null>(null)
  if (!storeRef.current) {
    storeRef.current = createFleetStore((functionId, payload) =>
      hostRef.current.iii.call(functionId, payload),
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
    let offObserver: (() => void) | null = null
    try {
      offHandler = host.iii.on(EVENTS_FN, (payload: unknown) => {
        // Fleet snapshots apply straight from the wire; a membership
        // change (and any runtime-kind event) costs one debounced
        // list_runtimes read.
        if (!store.applyFleetEvent(payload)) store.refreshRuntimes()
      })
      offObserver = host.iii.observe(ON_EVENT_FN, `${EVENTS_FN}::${host.iii.browserId}`)
      setLive(true)
    } catch {
      // The client refused to observe. This page ships WITH the worker, so
      // in practice this never fires; the manual refresh button remains the
      // fallback.
    }
    return () => {
      setLive(false)
      offObserver?.()
      offHandler?.()
    }
  }, [host, store])

  const refresh = useCallback(() => store.refresh(true), [store])

  return { state, refresh, live }
}
