import { useEffect, useId, useRef } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import {
  parseLandBlockedEvent,
  parseLandedEvent,
  WORKTREE_ON_LAND_BLOCKED_FN,
  WORKTREE_ON_LANDED_FN,
  type WorktreeLandBlockedEvent,
  type WorktreeLandedEvent,
} from '@/lib/worktrees'

/**
 * Observe the worktree worker's `worktree::on-landed` and
 * `worktree::on-land-blocked` with browser-local handlers: `client.on(fnId)`
 * plus `client.observe` targeting `<fnId>::<browserId>`. The per-mount
 * `instanceId` keeps two hook instances (route chat + dock) from colliding
 * on the registered function name.
 *
 * With the worker absent the observers stay pending until it registers;
 * callers already gate `enabled` on worktree presence. The try/catch drops
 * a subscription the client refuses.
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
      let client: Awaited<ReturnType<typeof getIiiClient>>
      try {
        client = await getIiiClient()
      } catch {
        // Client startup failed; leave the observers out (callers already
        // gate `enabled` on worktree presence).
        return
      }
      if (cancelled) return
      const bind = (
        baseFnId: string,
        functionId: string,
        onPayload: (payload: unknown) => void,
      ) => {
        const localFnId = `${baseFnId}::${instanceId}`
        try {
          offs.push(client.on(localFnId, onPayload))
          offs.push(
            client.observe(functionId, `${localFnId}::${client.browserId}`),
          )
        } catch {
          // The client refused the observer; drop the subscription.
        }
      }
      bind(LANDED_FN, WORKTREE_ON_LANDED_FN, (payload) => {
        const evt = parseLandedEvent(payload)
        if (evt) onLandedRef.current(evt)
      })
      bind(LAND_BLOCKED_FN, WORKTREE_ON_LAND_BLOCKED_FN, (payload) => {
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
