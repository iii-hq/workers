/**
 * Push subscriptions, replacing the page's polling.
 *
 * The page used to run a three-second `setInterval` asking for git status and
 * re-reading every open tab. That was a mistake: the platform pushes. Two
 * bindings replace it, and both are garbage-collected with the tab.
 *
 * - The **`state`** trigger type fires on any key change in a scope. The
 *   workspace record already lives in `state` under scope `editor`, so every
 *   buffer open, close, save and folder toggle was already emitting an event
 *   the page was ignoring.
 * - **`editor::changed`** is the worker's own trigger type, fired when a file
 *   changes however it changed — including by an agent that never called this
 *   worker.
 *
 * The `iii::` handler prefix is deliberate: it keeps the per-event invocations
 * span-suppressed and out of the trace feed, which matters when an agent is
 * writing files in a loop.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef } from 'react'

const STATE_EVENTS_FN = 'iii::editor-ui::state'
const CHANGED_EVENTS_FN = 'iii::editor-ui::changed'

export interface StateEvent {
  type: 'state'
  event_type: 'state:created' | 'state:updated' | 'state:deleted'
  scope: string
  key: string
  old_value: unknown
  new_value: unknown
}

export interface ChangedEvent {
  path: string
  cause: string
  kind: 'created' | 'modified' | 'deleted' | 'moved' | string
  added: number
  removed: number
  patch: string
  truncated: boolean
  root: string
}

type Listener<T> = (event: T) => void
export type Subscribe<T> = (listener: Listener<T>) => () => void

/**
 * Bind both event sources for this component's lifetime and return
 * subscribe functions for descendants.
 *
 * One binding per tab, registered on mount and unregistered on unmount, so a
 * hot reload disposes them with the page.
 */
export function useWorkspaceEvents(host: Host): {
  onState: Subscribe<StateEvent>
  onChanged: Subscribe<ChangedEvent>
} {
  const stateListeners = useRef<Set<Listener<StateEvent>>>(new Set())
  const changedListeners = useRef<Set<Listener<ChangedEvent>>>(new Set())

  useEffect(() => {
    const offState = host.iii.on<StateEvent>(STATE_EVENTS_FN, (event) => {
      // Only this worker's scope: the state worker is shared, and another
      // worker's keys are none of the editor's business.
      if (event?.type !== 'state' || event.scope !== 'editor') return
      for (const listener of [...stateListeners.current]) listener(event)
    })
    const offChanged = host.iii.on<ChangedEvent>(CHANGED_EVENTS_FN, (event) => {
      if (typeof event?.path !== 'string') return
      for (const listener of [...changedListeners.current]) listener(event)
    })

    const unbindState = host.iii.registerTrigger({
      type: 'state',
      function_id: `${STATE_EVENTS_FN}::${host.iii.browserId}`,
      config: { scope: 'editor' },
    })
    const unbindChanged = host.iii.registerTrigger({
      type: 'editor::changed',
      function_id: `${CHANGED_EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })

    return () => {
      unbindState()
      unbindChanged()
      offState()
      offChanged()
    }
  }, [host])

  const onState = useCallback<Subscribe<StateEvent>>((listener) => {
    stateListeners.current.add(listener)
    return () => {
      stateListeners.current.delete(listener)
    }
  }, [])

  const onChanged = useCallback<Subscribe<ChangedEvent>>((listener) => {
    changedListeners.current.add(listener)
    return () => {
      changedListeners.current.delete(listener)
    }
  }, [])

  return { onState, onChanged }
}
