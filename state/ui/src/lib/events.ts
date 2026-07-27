/**
 * Live state events over the worker's own `state` trigger type.
 *
 * `useStateEventHub` registers ONE tab-scoped binding (empty config = every
 * scope/key) whose function_id is a per-tab handler, and fans the events out
 * to whichever consumers attach via the returned `Subscribe`. The `iii::`
 * handler prefix keeps the per-event invocations span-suppressed, and the
 * binding is GC'd with the tab like any Message-path trigger.
 */

import { useCallback, useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
export const EVENTS_FN = 'iii::state-ui::events'

/** The message a `state` function trigger delivers per write. */
export interface StateEvent {
  type: 'state'
  event_type: 'state:created' | 'state:updated' | 'state:deleted'
  scope: string
  key: string
  old_value: unknown
  new_value: unknown
}

export type Subscribe = (listener: (e: StateEvent) => void) => () => void

/**
 * Register the binding for this component's lifetime and return a
 * `Subscribe` for descendants. Registered on mount, unregistered on
 * unmount — hot reload disposes it with the page.
 */
export function useStateEventHub(host: Host): Subscribe {
  const listenersRef = useRef<Set<(e: StateEvent) => void>>(new Set())
  useEffect(() => {
    const offHandler = host.iii.on<StateEvent>(EVENTS_FN, (event) => {
      if (!event || event.type !== 'state') return
      for (const listener of [...listenersRef.current]) listener(event)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'state',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host])
  return useCallback((listener) => {
    listenersRef.current.add(listener)
    return () => {
      listenersRef.current.delete(listener)
    }
  }, [])
}

/** Latest-handler subscription helper — views attach for their lifetime. */
export function useStateEvents(
  subscribe: Subscribe,
  handler: (e: StateEvent) => void,
) {
  const ref = useRef(handler)
  ref.current = handler
  useEffect(() => subscribe((e) => ref.current(e)), [subscribe])
}
