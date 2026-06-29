import { useCallback, useEffect, useMemo, useState } from 'react'
import { z } from 'zod'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { useWorkerLifecycle } from './use-worker-lifecycle'

/**
 * Generic presence probe for an OPTIONAL standalone engine worker. Answers a
 * single question — "is worker `workerName` connected right now?" — so callers
 * can gate worker-specific UI + RPC on it and never trigger "function not
 * found".
 *
 * Presence is fed by two signals, mirroring the harness probe:
 *   1. An initial `engine::workers::list` read on mount.
 *   2. A real-time `worker` add/remove lifecycle trigger, so the UI reacts the
 *      instant the worker is added or removed — whether from the CLI
 *      (`iii worker add <name>`) or another surface.
 *
 * Each consumer MUST pass a unique `watchFnId` so its browser-local handler for
 * the `worker` trigger does not collide with another presence probe's.
 */

export interface WorkerPresence {
  /** Whether the worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
}

const workerEventSchema = z.object({
  operation: z.string().optional(),
  stage: z.string().optional(),
  worker: z.string().nullable().optional(),
  source: z.unknown().optional(),
})
type WorkerEvent = z.infer<typeof workerEventSchema>

function parseWorkerEvent(payload: unknown): WorkerEvent | null {
  const parsed = workerEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

/** Loose match: is this lifecycle event about the named worker? */
function eventMatchesWorker(evt: WorkerEvent, workerName: string): boolean {
  const needle = workerName.toLowerCase()
  const w = typeof evt.worker === 'string' ? evt.worker.toLowerCase() : ''
  if (w === needle || w.includes(needle)) {
    return true
  }
  if (evt.source != null) {
    try {
      if (JSON.stringify(evt.source).toLowerCase().includes(needle)) {
        return true
      }
    } catch {
      // non-serialisable source; fall through
    }
  }
  return false
}

async function checkWorkerPresent(
  client: IiiClient,
  name: string,
): Promise<boolean> {
  const res = await client.trigger<{ workers?: Array<{ name?: unknown }> }>(
    'engine::workers::list',
    {},
  )
  const workers = Array.isArray(res?.workers) ? res.workers : []
  return workers.some((w) => w?.name === name)
}

export interface UseWorkerPresenceOptions {
  /** Engine worker name to probe, e.g. `shell` or `approval-gate`. */
  workerName: string
  /** Unique base id for this probe's browser-local `worker` trigger handler. */
  watchFnId: string
  /**
   * Only run against the real backend; pass `false` for the mock/Storybook
   * backend (treats the worker as present so its UI shows in isolation).
   */
  enabled: boolean
}

export function useWorkerPresence({
  workerName,
  watchFnId,
  enabled,
}: UseWorkerPresenceOptions): WorkerPresence {
  const [present, setPresent] = useState<boolean>(!enabled)
  const [loading, setLoading] = useState<boolean>(enabled)

  // Live add/remove so installing or dropping the worker mid-session flips the
  // UI without a reload. `workerName` is a stable literal per consumer, so this
  // callback's identity does not churn the lifecycle subscription.
  const handleEvent = useCallback(
    (payload: unknown) => {
      const evt = parseWorkerEvent(payload)
      if (!evt || !eventMatchesWorker(evt, workerName) || evt.stage !== 'done') {
        return
      }
      if (evt.operation === 'add') setPresent(true)
      else if (evt.operation === 'remove') setPresent(false)
    },
    [workerName],
  )

  useWorkerLifecycle({
    enabled,
    fnId: watchFnId,
    operations: ['add', 'remove'],
    onEvent: handleEvent,
  })

  // One-time presence probe on mount. Live changes arrive through the `worker`
  // trigger above; no polling.
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      setPresent(true)
      return
    }
    let cancelled = false
    void (async () => {
      const client = await getIiiClient()
      try {
        const found = await checkWorkerPresent(client, workerName)
        if (!cancelled) setPresent(found)
      } catch {
        if (!cancelled) setPresent(false)
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [enabled, workerName])

  return useMemo(() => ({ present, loading }), [present, loading])
}

/**
 * Whether the probed worker is connected and its functions safe to trigger.
 * False during the initial presence probe and while the worker is absent.
 */
export function isWorkerPresent(status: WorkerPresence): boolean {
  return status.present && !status.loading
}
