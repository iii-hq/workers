import { useEffect, useId, useRef } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import {
  parseLandBlockedEvent,
  parseLandedEvent,
  WORKTREE_LAND_BLOCKED_TRIGGER,
  WORKTREE_LANDED_TRIGGER,
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
