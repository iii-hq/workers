import { useCallback, useEffect, useMemo, useState } from 'react'
import { z } from 'zod'
import { unwrapEnvelope } from '@/components/chat/sandbox/parsers'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { useWorkerLifecycle } from './use-worker-lifecycle'

/**
 * Generic presence probe for an OPTIONAL standalone engine worker. The
 * engine catalogue answers "connected now" while worker-manager status
 * answers "installed/running"; both are required to explain a worker that is
 * installed but stopped or still booting.
 */

export type WorkerPresenceState =
  | 'connected'
  | 'starting'
  | 'provisioning'
  | 'stopped'
  | 'absent'
  | 'unknown'

export interface WorkerPresence {
  /** Whether the worker is currently connected and safe to call. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
  /** Distinguishes absent, installed-but-stopped, and booting workers. */
  state: WorkerPresenceState
  /** Optional operator-facing detail, usually the latest worker error. */
  detail: string | null
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

const engineWorkersSchema = z.object({
  workers: z.array(
    z.object({
      name: z.string().nullable().optional(),
      status: z.string().optional(),
    }),
  ),
})

const workerStatusSchema = z.object({
  installed: z.boolean(),
  running: z.boolean(),
  stderr_tail: z.array(z.string()).optional().default([]),
  stdout_tail: z.array(z.string()).optional().default([]),
  hint: z.string().optional(),
})

export interface WorkerPresenceSnapshot {
  present: boolean
  state: WorkerPresenceState
  detail: string | null
}

function latestWorkerDetail(
  status: z.infer<typeof workerStatusSchema>,
): string | null {
  // stderr is the failure signal for a stopped worker. Prefer it when both
  // tails are populated so a benign stdout line cannot hide the boot error.
  const lines =
    status.stderr_tail.length > 0 ? status.stderr_tail : status.stdout_tail
  return lines.length > 0
    ? (lines[lines.length - 1] ?? null)
    : (status.hint ?? null)
}

/**
 * Reconcile the engine catalogue with worker-manager status. A successful
 * manager read is intentionally not treated as connected: a managed process
 * can be alive while its registration is still missing or has crashed.
 */
export async function readWorkerPresence(
  client: IiiClient,
  name: string,
): Promise<WorkerPresenceSnapshot> {
  const [engineResult, managerResult] = await Promise.allSettled([
    client.trigger<unknown>('engine::workers::list', {}),
    client.trigger<unknown>('worker::status', { name }),
  ])

  const engine =
    engineResult.status === 'fulfilled'
      ? engineWorkersSchema.safeParse(unwrapEnvelope(engineResult.value)).data
      : null
  const manager =
    managerResult.status === 'fulfilled'
      ? workerStatusSchema.safeParse(unwrapEnvelope(managerResult.value)).data
      : null
  const connected =
    engine?.workers.some(
      (worker) =>
        worker.name === name && worker.status?.toLowerCase() === 'connected',
    ) ?? false

  if (connected) {
    return { present: true, state: 'connected', detail: null }
  }

  if (manager) {
    if (!manager.installed) {
      return { present: false, state: 'absent', detail: null }
    }
    if (manager.running) {
      return {
        present: false,
        state: 'starting',
        detail: 'worker process is running but has not connected to the engine',
      }
    }
    const detail = latestWorkerDetail(manager)
    return {
      present: false,
      state: detail ? 'stopped' : 'provisioning',
      detail,
    }
  }

  // A rejected or malformed manager response is not proof that the worker is
  // absent. Keep the UI honest when an older/restarting manager cannot answer.
  return {
    present: false,
    state: 'unknown',
    detail: 'could not read worker-manager presence',
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
  const [present, setPresent] = useState<boolean>(!enabled)
  const [loading, setLoading] = useState<boolean>(enabled)
  const [state, setState] = useState<WorkerPresenceState>(
    enabled ? 'unknown' : 'connected',
  )
  const [detail, setDetail] = useState<string | null>(null)
  const [probeNonce, setProbeNonce] = useState(0)

  // Live add/remove so installing or dropping the worker mid-session flips the
  // UI without a reload. `workerName` is a stable literal per consumer, so
  // this callback's identity does not churn the lifecycle subscription.
  const handleEvent = useCallback(
    (payload: unknown) => {
      const evt = parseWorkerEvent(payload)
      if (
        !evt ||
        !eventMatchesWorker(evt, workerName) ||
        (evt.stage !== 'done' && evt.stage !== 'failed')
      ) {
        return
      }
      if (evt.operation === 'remove') {
        setPresent(false)
        setState('absent')
        setDetail(null)
      } else if (evt.stage === 'failed') {
        setPresent(false)
        setState('stopped')
        setDetail('worker manager failed to start the worker')
      } else {
        // `worker::add` done means the manager finished its operation, not
        // necessarily that the process has registered with the engine yet.
        setPresent(false)
        setState('starting')
        setDetail(null)
        setProbeNonce((nonce) => nonce + 1)
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

  // Initial presence probe plus a re-probe after lifecycle completion. Live
  // changes arrive through the `worker` trigger above; no blind polling.
  // biome-ignore lint/correctness/useExhaustiveDependencies: probeNonce is an explicit re-probe trigger after lifecycle completion.
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      setPresent(true)
      setState('connected')
      setDetail(null)
      return
    }
    let cancelled = false
    setLoading(true)
    void (async () => {
      try {
        const client = await getIiiClient()
        const snapshot = await readWorkerPresence(client, workerName)
        if (!cancelled) {
          setPresent(snapshot.present)
          setState(snapshot.state)
          setDetail(snapshot.detail)
        }
      } catch {
        if (!cancelled) {
          setPresent(false)
          setState('unknown')
          setDetail('could not read worker presence')
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [enabled, workerName, probeNonce])

  return useMemo(
    () => ({ present, loading, state, detail }),
    [present, loading, state, detail],
  )
}

/**
 * Whether the probed worker is connected and its functions safe to trigger.
 * False during the initial presence probe and while the worker is absent.
 */
export function isWorkerPresent(status: WorkerPresence): boolean {
  return status.present && !status.loading
}
