import { useEffect, useId, useRef } from 'react'
import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Observe `engine::on-workers-changed` with a browser-local handler so the
 * UI reacts the instant a worker connects or leaves — whether from an in-app
 * CTA or an `iii call compose::add` run in the operator's own terminal.
 *
 * The event names the worker by its connection id (`worker_id`), not by
 * name: consumers re-read `engine::workers::list` (or whatever else they
 * show) when it arrives, rather than reading the event.
 *
 * The registered function name is `<fnId>::<instanceId>::<browserId>`: the
 * per-tab `browserId` keeps two open tabs from colliding, and the per-mount
 * `instanceId` keeps two hook instances in the same tab from colliding too —
 * every UI registration ends up with a globally unique name.
 */

/** The engine's worker events; `worker_metadata_updated` is not a join or a leave. */
export type WorkersChangedKind =
  | 'worker_connected'
  | 'worker_disconnected'
  | 'worker_metadata_updated'

/** One call of `engine::on-workers-changed`. */
export interface WorkersChangedEvent {
  event: WorkersChangedKind
  worker_id: string
}

export const WORKERS_CHANGED_FN = 'engine::on-workers-changed'

const workersChangedSchema = z.object({
  event: z.enum([
    'worker_connected',
    'worker_disconnected',
    'worker_metadata_updated',
  ]),
  worker_id: z.string(),
})

export function parseWorkersChanged(
  payload: unknown,
): WorkersChangedEvent | null {
  const parsed = workersChangedSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

export interface UseWorkersChangedOptions {
  /** Only observe when enabled (e.g. the real backend). */
  enabled: boolean
  /**
   * Base id for the browser-local handler. The hook appends a unique
   * `<instanceId>::<browserId>` suffix so the final registered name never
   * collides across tabs or mounts.
   */
  fnId: string
  /** Which worker events to receive. Defaults to joins and leaves. */
  events?: readonly WorkersChangedKind[]
  /** Invoked with each matching event. */
  onChange: (event: WorkersChangedEvent) => void
}

const JOINS_AND_LEAVES: readonly WorkersChangedKind[] = [
  'worker_connected',
  'worker_disconnected',
]

export function useWorkersChanged(opts: UseWorkersChangedOptions): void {
  const { enabled, fnId } = opts

  // Keep the latest handler in a ref so a changing callback identity never
  // forces a new observer (the engine-side observer is the expensive part).
  const onChangeRef = useRef(opts.onChange)
  onChangeRef.current = opts.onChange

  // Per-mount token. Combined with the per-tab `browserId` (appended by
  // `client.on` and in the observer's target) this makes the registered
  // function name unique across tabs AND across multiple mounts of the hook.
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  // Serialize the filter so an inline literal does not re-run the effect on
  // every render; the effect rebuilds the array from this stable key.
  const eventsKey = (opts.events ?? JOINS_AND_LEAVES).join(',')

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let offHandler: (() => void) | undefined
    let offObserver: (() => void) | undefined

    void (async () => {
      try {
        const client = await getIiiClient()
        if (cancelled) return
        // Unique per (base id × hook instance); `client.on` then appends the
        // per-tab `::<browserId>`.
        const localFnId = `${fnId}::${instanceId}`
        offHandler = client.on(localFnId, (data: unknown) => {
          const event = parseWorkersChanged(data)
          if (event) onChangeRef.current(event)
        })
        offObserver = client.observe(
          {
            function_id: WORKERS_CHANGED_FN,
            filters: { 'input.event': { $in: eventsKey.split(',') } },
          },
          `${localFnId}::${client.browserId}`,
        )
      } catch {
        offObserver?.()
        offHandler?.()
        offObserver = undefined
        offHandler = undefined
      }
    })()

    return () => {
      cancelled = true
      offObserver?.()
      offHandler?.()
    }
  }, [enabled, fnId, instanceId, eventsKey])
}
