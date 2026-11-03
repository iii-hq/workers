/**
 * Live state events: the page observes `state::on-changed`.
 *
 * `useStateEventHub` registers ONE tab-scoped observer (no filters = every
 * scope/key) whose target is a per-tab handler, and fans the events out to
 * whichever consumers attach via the returned `Subscribe`. The `iii::`
 * handler prefix keeps the per-event invocations span-suppressed, and the
 * observer is disposed with the page.
 */

import { useCallback, useEffect, useRef } from 'react'
import type { Host } from '@iii-dev/console-ui'

/** The observed function: one call per committed state change. */
export const ON_CHANGED_FN = 'state::on-changed'

/** Per-tab handler id (host.iii.on namespaces it `::<browserId>`). */
export const EVENTS_FN = 'iii::state-ui::events'

/** The event `state::on-changed` returns per write. */
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
 * Observe `state::on-changed` for this component's lifetime and return a
 * `Subscribe` for descendants. Observed on mount, disposed on unmount — hot
 * reload disposes it with the page.
 */
export function useStateEventHub(host: Host): Subscribe {
  const listenersRef = useRef<Set<(e: StateEvent) => void>>(new Set())
  useEffect(() => {
    const offHandler = host.iii.on<StateEvent>(EVENTS_FN, (event) => {
      if (!event || event.type !== 'state') return
      for (const listener of [...listenersRef.current]) listener(event)
    })
    const dispose = host.iii.observe(
      { function_id: ON_CHANGED_FN, deliver: 'output' },
      `${EVENTS_FN}::${host.iii.browserId}`,
    )
    return () => {
      dispose()
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
