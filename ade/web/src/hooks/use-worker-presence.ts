import { useCallback, useEffect, useMemo, useState } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { useWorkersChanged } from './use-workers-changed'

/**
 * Generic presence probe for an OPTIONAL standalone engine worker. Answers a
 * single question — "is worker `workerName` connected right now?" — so callers
 * can gate worker-specific UI + RPC on it and never call a function that is
 * not registered.
 *
 * Presence is read from `engine::workers::list`: once on mount, and again on
 * every join or leave `engine::on-workers-changed` announces, so the UI
 * reacts the instant the worker is added or removed — whether from the CLI
 * (`iii call compose::add`) or another surface.
 *
 * Each consumer MUST pass a unique `watchFnId` so its browser-local handler
 * does not collide with another presence probe's.
 */

export interface WorkerPresence {
  /** Whether the worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
}

/** Whether an `engine::workers::list` answer lists a worker named `name`. */
export function workerListed(response: unknown, name: string): boolean {
  const workers = (response as { workers?: unknown } | null)?.workers
  return (
    Array.isArray(workers) &&
    workers.some(
      (worker) => (worker as { name?: unknown } | null)?.name === name,
    )
  )
}

async function checkWorkerPresent(
  client: IiiClient,
  name: string,
): Promise<boolean> {
  return workerListed(await client.call('engine::workers::list', {}), name)
}

export interface UseWorkerPresenceOptions {
  /** Engine worker name to probe, e.g. `shell` or `approval-gate`. */
  workerName: string
  /** Unique base id for this probe's browser-local workers-changed handler. */
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

  // Live joins and leaves, so installing or dropping the worker mid-session
  // flips the UI without a reload: each one re-reads the list. `workerName`
  // is a stable literal per consumer, so this callback's identity does not
  // churn the observer.
  const handleChange = useCallback(() => {
    void (async () => {
      try {
        const client = await getIiiClient()
        setPresent(await checkWorkerPresent(client, workerName))
      } catch {
        // Keep the last answer; the next event reads again.
      }
    })()
  }, [workerName])

  useWorkersChanged({
    enabled,
    fnId: watchFnId,
    onChange: handleChange,
  })

  // One-time presence probe on mount. Live changes arrive through the
  // workers-changed observer above; no polling.
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
 * Whether the probed worker is connected and its functions safe to call.
 * False during the initial presence probe and while the worker is absent.
 */
export function isWorkerPresent(status: WorkerPresence): boolean {
  return status.present && !status.loading
}
