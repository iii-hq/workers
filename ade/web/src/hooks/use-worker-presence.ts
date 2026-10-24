import { useCallback, useEffect, useMemo, useState } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { useWorkersChanged } from './use-workers-changed'

/**
 * Generic presence probe for an OPTIONAL standalone engine worker. Answers a
 * single question — "is worker `workerName` connected right now?" — so callers
 * can gate worker-specific UI + RPC on it and never trigger "function not
 * found".
 *
 * Presence is an `engine::workers::list` read on mount, read again on every
 * `engine::workers-available` event, so the UI reacts the instant the worker
 * is added or removed — whether from the CLI (`iii trigger compose::add
 * worker=<name>`) or another surface.
 *
 * Each consumer MUST pass a unique `watchFnId` for its browser-local
 * `engine::workers-available` handler.
 */

export interface WorkerPresence {
  /** Whether the worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
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
  /** Unique base id for this probe's browser-local trigger handler. */
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

  // A re-read after a worker change keeps the last answer when it fails.
  const recheck = useCallback(() => {
    void (async () => {
      try {
        const client = await getIiiClient()
        setPresent(await checkWorkerPresent(client, workerName))
      } catch {
        // keep the last known presence
      }
    })()
  }, [workerName])

  useWorkersChanged({ enabled, fnId: watchFnId, onChange: recheck })

  // Presence probe on mount; later changes arrive through the trigger above.
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      setPresent(true)
      return
    }
    let cancelled = false
    void (async () => {
      try {
        const client = await getIiiClient()
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
