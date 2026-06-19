import { useEffect, useId, useRef } from 'react'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Subscribe to engine `worker` lifecycle events bound to a browser-local
 * handler so the UI reacts the instant a worker is added/removed — whether
 * from an in-app CTA or a `iii worker add/remove <name>` run in the operator's
 * own terminal.
 *
 * IMPORTANT: the engine `worker` trigger filters on BOTH `operations` and
 * `stages`. The only working subscriber in the tree (the Rust `directory`
 * worker) always sets both; omitting `stages` matches no events, so this hook
 * always sends a stage filter (defaulting to the full lifecycle).
 *
 * The registered function name is `<fnId>::<instanceId>::<browserId>`: the
 * per-tab `browserId` keeps two open tabs from colliding, and the per-mount
 * `instanceId` keeps two hook instances in the same tab from colliding too —
 * every UI registration ends up with a globally unique name.
 *
 * Registration is wrapped in try/catch: if the engine doesn't publish the
 * `worker` trigger type the binding is dropped silently and callers fall back
 * to whatever presence path they own.
 */

/** Full worker-manager lifecycle, in order. */
export const WORKER_LIFECYCLE_STAGES = [
  'started',
  'downloading',
  'downloaded',
  'done',
  'failed',
] as const

export interface UseWorkerLifecycleOptions {
  /** Only subscribe when enabled (e.g. the real backend). */
  enabled: boolean
  /**
   * Base id for the browser-local handler. The hook appends a unique
   * `<instanceId>::<browserId>` suffix so the final registered name never
   * collides across tabs or mounts.
   */
  fnId: string
  /** Worker operations to watch (e.g. `['add']` or `['add', 'remove']`). */
  operations: readonly string[]
  /** Lifecycle stages to watch. Defaults to every stage. */
  stages?: readonly string[]
  /** Invoked with the raw payload for each matching lifecycle event. */
  onEvent: (payload: unknown) => void
}

export function useWorkerLifecycle(opts: UseWorkerLifecycleOptions): void {
  const { enabled, fnId, onEvent } = opts

  // Keep the latest handler in a ref so a changing callback identity never
  // forces a re-subscription (the engine binding is the expensive part).
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent

  // Per-mount token. Combined with the per-tab `browserId` (appended by
  // `client.on` and in the trigger id) this makes the registered function name
  // unique across tabs AND across multiple mounts of the same hook.
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  // Serialize the filter arrays so inline literals don't re-run the effect on
  // every render; the effect rebuilds the arrays from these stable keys.
  const opsKey = opts.operations.join(',')
  const stagesKey = (opts.stages ?? WORKER_LIFECYCLE_STAGES).join(',')

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined

    void (async () => {
      const client = await getIiiClient()
      if (cancelled) return
      // Unique per (base id × hook instance); `client.on` then appends the
      // per-tab `::<browserId>`.
      const localFnId = `${fnId}::${instanceId}`
      try {
        offHandler = client.on(localFnId, (data: unknown) => {
          onEventRef.current(data)
        })
        offTrigger = client.registerTrigger({
          type: 'worker',
          function_id: `${localFnId}::${client.browserId}`,
          config: {
            operations: opsKey ? opsKey.split(',') : [],
            stages: stagesKey ? stagesKey.split(',') : [],
          },
        })
      } catch {
        offTrigger?.()
        offHandler?.()
        offTrigger = undefined
        offHandler = undefined
      }
    })()

    return () => {
      cancelled = true
      offTrigger?.()
      offHandler?.()
    }
  }, [enabled, fnId, instanceId, opsKey, stagesKey])
}
