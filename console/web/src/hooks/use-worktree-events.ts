import { useEffect, useId, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import {
  parseLandBlockedEvent,
  parseLandedEvent,
  WORKTREE_LAND_BLOCKED_TRIGGER,
  WORKTREE_LANDED_TRIGGER,
  WORKTREE_LIFECYCLE_TRIGGERS,
  type WorktreeLandBlockedEvent,
  type WorktreeLandedEvent,
} from '@/lib/worktrees'

/**
 * Subscribe browser-local handlers to the worktree worker's `worktree::landed`
 * and `worktree::land-blocked` trigger types, following the worker-lifecycle
 * binding pattern: `client.on(fnId)` plus `client.registerTrigger` targeting
 * `<fnId>::<browserId>`. The per-mount `instanceId` keeps two hook instances
 * (route chat + dock) from colliding on the registered function name.
 *
 * Registration is wrapped in try/catch: with the worker absent (its trigger
 * types unregistered) the binding drops silently — callers already gate
 * `enabled` on worktree presence, this is defense-in-depth for races.
 */

const LANDED_FN = 'console::worktree-landed'
const LAND_BLOCKED_FN = 'console::worktree-land-blocked'

export interface UseWorktreeEventsOptions {
  /** Only subscribe on the real backend with the worktree worker present. */
  enabled: boolean
  onLanded: (evt: WorktreeLandedEvent) => void
  onLandBlocked: (evt: WorktreeLandBlockedEvent) => void
}

export function useWorktreeEvents(opts: UseWorktreeEventsOptions): void {
  const { enabled } = opts

  const onLandedRef = useRef(opts.onLanded)
  onLandedRef.current = opts.onLanded
  const onLandBlockedRef = useRef(opts.onLandBlocked)
  onLandBlockedRef.current = opts.onLandBlocked

  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    const offs: Array<() => void> = []

    void (async () => {
      const client = await getIiiClient()
      if (cancelled) return
      const bind = (
        baseFnId: string,
        triggerType: string,
        onPayload: (payload: unknown) => void,
      ) => {
        const localFnId = `${baseFnId}::${instanceId}`
        try {
          offs.push(client.on(localFnId, onPayload))
          offs.push(
            client.registerTrigger({
              type: triggerType,
              function_id: `${localFnId}::${client.browserId}`,
              config: {},
            }),
          )
        } catch {
          // Worker absent or trigger type unregistered; drop the binding.
        }
      }
      bind(LANDED_FN, WORKTREE_LANDED_TRIGGER, (payload) => {
        const evt = parseLandedEvent(payload)
        if (evt) onLandedRef.current(evt)
      })
      bind(LAND_BLOCKED_FN, WORKTREE_LAND_BLOCKED_TRIGGER, (payload) => {
        const evt = parseLandBlockedEvent(payload)
        if (evt) onLandBlockedRef.current(evt)
      })
    })()

    return () => {
      cancelled = true
      for (const off of offs) off()
    }
  }, [enabled, instanceId])
}

const LIFECYCLE_FN = 'console::worktree-lifecycle'

export interface UseWorktreeLifecycleEventsOptions {
  /** Only subscribe on the real backend with the worktree worker present. */
  enabled: boolean
  /** Fired for EVERY worktree lifecycle event, with its trigger type. */
  onEvent: (triggerType: string, payload: unknown) => void
}

export interface WorktreeLifecycleSubscription {
  /**
   * True once all six trigger bindings registered. While false (probe in
   * flight, worker absent, SDK failure) callers fall back to polling.
   */
  bound: boolean
}

/**
 * Page-scoped feed of ALL six worktree lifecycle trigger types
 * (created / claimed / released / removed / landed / land-blocked), for
 * surfaces that re-read the registry on any change rather than reacting to
 * one event shape. Same browser-local binding pattern as
 * [`useWorktreeEvents`].
 */
export function useWorktreeLifecycleEvents(
  opts: UseWorktreeLifecycleEventsOptions,
): WorktreeLifecycleSubscription {
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
      const client = await getIiiClient()
      if (cancelled) return
      let registered = 0
      for (const triggerType of WORKTREE_LIFECYCLE_TRIGGERS) {
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
        setBound(registered === WORKTREE_LIFECYCLE_TRIGGERS.length)
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
