import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { z } from 'zod'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { useWorkerLifecycle } from './use-worker-lifecycle'

/**
 * Generic presence probe for an OPTIONAL standalone engine worker. Answers a
 * single question — "is worker `workerName` connected right now?" — so callers
 * can gate worker-specific UI + RPC on it and never trigger "function not
 * found".
 *
 * Presence is reconciled from four signals:
 *   1. An initial `engine::workers::list` read on mount.
 *   2. `engine::workers-available`, which covers raw process connections and
 *      disconnects (including crashes/restarts outside the worker manager).
 *   3. A real-time `worker` add/remove lifecycle trigger, so the UI reacts the
 *      instant the worker is added or removed — whether from the CLI
 *      (`iii worker add <name>`) or another surface.
 *   4. Browser WebSocket reconnect, which re-reads the authoritative snapshot
 *      because events fired during the outage are not replayed.
 *
 * Each consumer MUST pass a unique `watchFnId` so its browser-local handler for
 * the `worker` trigger does not collide with another presence probe's.
 */

export interface WorkerPresence {
  /** Whether the worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
  /**
   * Advances whenever a present worker instance changes or the browser
   * reconnects. Consumers that own worker-scoped trigger bindings include it
   * in their effect dependencies so those bindings are recreated even across
   * a present→present restart.
   */
  revision: number
  /** Authoritatively re-read presence; used by manual recovery controls. */
  refresh: () => Promise<boolean>
}

interface WorkerPresenceState {
  present: boolean
  loading: boolean
  revision: number
}

interface WorkerSnapshot {
  present: boolean
  workerId: string | null
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
): Promise<WorkerSnapshot> {
  const res = await client.trigger<{
    workers?: Array<{ id?: unknown; name?: unknown }>
  }>('engine::workers::list', {})
  const workers = Array.isArray(res?.workers) ? res.workers : []
  const worker = workers.find((w) => w?.name === name)
  if (!worker) return { present: false, workerId: null }
  return {
    present: true,
    // Current engines expose an instance id. The name fallback preserves
    // compatibility with older engines, which can still detect absent→present.
    workerId: typeof worker.id === 'string' ? worker.id : name,
  }
}

export interface WorkerPresenceWatcher {
  refresh(options?: { forceRevision?: boolean }): Promise<boolean>
  markAbsent(): void
  dispose(): void
}

interface CreateWorkerPresenceWatcherOptions {
  client: IiiClient
  workerName: string
  localFnId: string
  initial: WorkerPresenceState
  onChange: (state: WorkerPresenceState) => void
}

/**
 * Imperative presence state machine kept outside React so reconnect, worker
 * catalogue ticks, stale async reads, and cleanup can be tested together.
 */
export function createWorkerPresenceWatcher({
  client,
  workerName,
  localFnId,
  initial,
  onChange,
}: CreateWorkerPresenceWatcherOptions): WorkerPresenceWatcher {
  let state = { ...initial, workerId: null as string | null }
  let generation = 0
  let disposed = false
  let offHandler: (() => void) | undefined
  let offTrigger: (() => void) | undefined
  let offConnection: (() => void) | undefined

  const publish = (snapshot: WorkerSnapshot, forceRevision: boolean) => {
    if (disposed) return
    const identityChanged =
      snapshot.present &&
      (!state.present || state.workerId !== snapshot.workerId)
    const revision =
      state.revision +
      (snapshot.present && (forceRevision || identityChanged) ? 1 : 0)
    const next = {
      present: snapshot.present,
      loading: false,
      revision,
      workerId: snapshot.workerId,
    }
    const changed =
      next.present !== state.present ||
      next.loading !== state.loading ||
      next.revision !== state.revision ||
      next.workerId !== state.workerId
    state = next
    if (changed) {
      onChange({
        present: state.present,
        loading: state.loading,
        revision: state.revision,
      })
    }
  }

  const refresh: WorkerPresenceWatcher['refresh'] = async (options = {}) => {
    const requestGeneration = ++generation
    try {
      const snapshot = await checkWorkerPresent(client, workerName)
      if (!disposed && requestGeneration === generation) {
        publish(snapshot, options.forceRevision === true)
      }
      return snapshot.present
    } catch {
      if (!disposed && requestGeneration === generation) {
        publish({ present: false, workerId: null }, false)
      }
      return false
    }
  }

  const markAbsent = () => {
    generation += 1
    publish({ present: false, workerId: null }, false)
  }

  // Engine-owned connection signal: unlike the worker-manager `worker`
  // lifecycle, this fires for raw process crashes/restarts as well as CLI
  // add/remove operations. Re-read the authoritative list instead of trusting
  // an event payload shape.
  try {
    offHandler = client.on(localFnId, () => {
      void refresh()
    })
    offTrigger = client.registerTrigger({
      type: 'engine::workers-available',
      function_id: `${localFnId}::${client.browserId}`,
      config: {},
    })
  } catch {
    offTrigger?.()
    offHandler?.()
    offTrigger = undefined
    offHandler = undefined
  }

  // This listener is deliberately active even while the target worker is
  // absent. Events fired during the WebSocket outage are not replayed, so a
  // successful reconnect must force both a presence read and consumer
  // re-subscription.
  try {
    offConnection = client.addConnectionStateListener((connectionState) => {
      if (connectionState === 'connected') {
        void refresh({ forceRevision: true })
      }
    })
  } catch {
    offConnection = undefined
  }

  return {
    refresh,
    markAbsent,
    dispose: () => {
      if (disposed) return
      disposed = true
      generation += 1
      offConnection?.()
      offTrigger?.()
      offHandler?.()
    },
  }
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
  const [status, setStatus] = useState<WorkerPresenceState>({
    present: !enabled,
    loading: enabled,
    revision: 0,
  })
  const watcherRef = useRef<WorkerPresenceWatcher | null>(null)
  const refreshRef = useRef<() => Promise<boolean>>(async () => !enabled)
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')

  const refresh = useCallback(() => refreshRef.current(), [])

  // Live add/remove so installing or dropping the worker mid-session flips the
  // UI without a reload. `workerName` is a stable literal per consumer, so this
  // callback's identity does not churn the lifecycle subscription.
  const handleEvent = useCallback(
    (payload: unknown) => {
      const evt = parseWorkerEvent(payload)
      if (
        !evt ||
        !eventMatchesWorker(evt, workerName) ||
        evt.stage !== 'done'
      ) {
        return
      }
      if (evt.operation === 'add') {
        void watcherRef.current?.refresh({ forceRevision: true })
      } else if (evt.operation === 'remove') {
        watcherRef.current?.markAbsent()
      }
    },
    [workerName],
  )

  useWorkerLifecycle({
    enabled,
    fnId: watchFnId,
    operations: ['add', 'remove'],
    onEvent: handleEvent,
  })

  // Initial snapshot plus engine-owned worker-catalogue and browser reconnect
  // signals. No polling.
  useEffect(() => {
    if (!enabled) {
      watcherRef.current = null
      refreshRef.current = async () => true
      setStatus({ present: true, loading: false, revision: 0 })
      return
    }

    let cancelled = false
    setStatus({ present: false, loading: true, revision: 0 })
    void getIiiClient()
      .then((client) => {
        if (cancelled) return
        const watcher = createWorkerPresenceWatcher({
          client,
          workerName,
          localFnId: `${watchFnId}::presence::${instanceId}`,
          initial: { present: false, loading: true, revision: 0 },
          onChange: setStatus,
        })
        watcherRef.current = watcher
        refreshRef.current = () => watcher.refresh({ forceRevision: true })
        void watcher.refresh()
      })
      .catch(() => {
        if (!cancelled) {
          setStatus({ present: false, loading: false, revision: 0 })
        }
      })

    return () => {
      cancelled = true
      watcherRef.current?.dispose()
      watcherRef.current = null
      refreshRef.current = async () => false
    }
  }, [enabled, instanceId, watchFnId, workerName])

  return useMemo(() => ({ ...status, refresh }), [status, refresh])
}

/**
 * Whether the probed worker is connected and its functions safe to trigger.
 * False during the initial presence probe and while the worker is absent.
 */
export function isWorkerPresent(status: WorkerPresence): boolean {
  return status.present && !status.loading
}
