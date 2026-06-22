import { useCallback, useEffect, useMemo, useState } from 'react'
import { z } from 'zod'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { useWorkerLifecycle } from './use-worker-lifecycle'

/**
 * Presence probe for the standalone `approval-gate` worker. The gate owns the
 * `approval::*` functions; it is OPTIONAL — the console must run cleanly with or
 * without it. This hook answers a single question: "is `approval-gate`
 * connected right now?" so callers can gate the approval UI + RPC on it.
 *
 * Unlike `useHarnessStatus`, there is no install CTA: we never push the gate on
 * the operator. Presence is fed by two signals, mirroring the harness probe:
 *   1. An initial `engine::workers::list` read on mount.
 *   2. A real-time `worker` add/remove lifecycle trigger, so the UI reacts the
 *      instant the gate is added or removed — whether from the CLI
 *      (`iii worker add approval-gate`) or another surface.
 */

/** Engine worker name for the standalone approval-gate worker. */
const APPROVAL_GATE_WORKER_NAME = 'approval-gate'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const APPROVAL_GATE_WATCH_FN = 'console::approval-gate-watch'

export interface ApprovalGateStatus {
  /** Whether the `approval-gate` worker is currently connected. */
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

/** Loose match: is this lifecycle event about the approval-gate worker? */
function isApprovalGateEvent(evt: WorkerEvent): boolean {
  const w = typeof evt.worker === 'string' ? evt.worker.toLowerCase() : ''
  if (w === APPROVAL_GATE_WORKER_NAME || w.includes(APPROVAL_GATE_WORKER_NAME)) {
    return true
  }
  if (evt.source != null) {
    try {
      if (
        JSON.stringify(evt.source)
          .toLowerCase()
          .includes(APPROVAL_GATE_WORKER_NAME)
      ) {
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

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats the gate as present so the approval UI shows
 *   in isolation, matching the mock fixtures).
 */
export function useApprovalGateStatus(enabled: boolean): ApprovalGateStatus {
  const [present, setPresent] = useState<boolean>(!enabled)
  const [loading, setLoading] = useState<boolean>(enabled)

  // Live add/remove so installing or dropping the gate mid-session flips the UI
  // without a reload. Functional setState only — no reactive deps.
  const handleEvent = useCallback((payload: unknown) => {
    const evt = parseWorkerEvent(payload)
    if (!evt || !isApprovalGateEvent(evt) || evt.stage !== 'done') return
    if (evt.operation === 'add') setPresent(true)
    else if (evt.operation === 'remove') setPresent(false)
  }, [])

  useWorkerLifecycle({
    enabled,
    fnId: APPROVAL_GATE_WATCH_FN,
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
        const found = await checkWorkerPresent(client, APPROVAL_GATE_WORKER_NAME)
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
  }, [enabled])

  return useMemo(() => ({ present, loading }), [present, loading])
}

/**
 * Whether the approval-gate's `approval::*` functions are registered and safe to
 * trigger. False during the initial presence probe and while the gate is absent.
 */
export function isApprovalGateAvailable(status: ApprovalGateStatus): boolean {
  return status.present && !status.loading
}
