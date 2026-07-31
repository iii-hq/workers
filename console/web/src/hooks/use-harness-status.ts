import { useCallback, useEffect, useMemo, useState } from 'react'
import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { useWorkerLifecycle } from './use-worker-lifecycle'
import {
  readWorkerPresence,
  type WorkerPresenceState,
} from './use-worker-presence'

/**
 * Watches the engine for the `harness` worker and drives an in-app install of
 * it via `worker::add`, surfacing live progress.
 *
 * Presence reconciles two runtime signals:
 *   1. `engine::workers::list` says whether the worker is callable now.
 *   2. `worker::status` explains installed, starting, provisioning, and stopped
 *      states when the worker is not connected.
 * A real-time `worker` lifecycle trigger keeps the UI responsive to an add
 * started from the in-app CTA or from `iii worker add harness` in a terminal.
 *
 * `install()` triggers `worker::add` for the registry `harness` source. It
 * resolves with a terminal outcome, but the per-stage progress
 * (started -> downloading -> downloaded -> done) only arrives through the
 * `worker` trigger, so we render that as a small console. When the trigger type
 * isn't published (or no events arrive) we still resolve cleanly off the
 * `worker::add` result and a presence re-check — a graceful, progress-less
 * fallback.
 */

/** Registry name + engine worker name for the harness meta-worker. */
const HARNESS_WORKER_NAME = 'harness'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const HARNESS_WATCH_FN = 'console::harness-watch'

/** One line in the install console (mapped from a `worker` lifecycle event). */
export interface InstallStage {
  /** Lifecycle stage: started | downloading | downloaded | done | failed | … */
  stage: string
  /** Canonical worker name / source label carried by the event. */
  worker?: string
  /** Optional 0..1 download progress. */
  progress?: number
  /** Present when `stage === 'failed'`. */
  error?: { code?: string; message?: string }
  /** Event timestamp (ms) or arrival time. */
  at: number
}

export interface HarnessStatus {
  /** Whether the `harness` worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
  /** Reconciled engine/worker-manager state for the harness process. */
  state: WorkerPresenceState
  /** Optional worker-manager detail when the process is not connected. */
  detail: string | null
  /** An add is in progress (in-app or detected from the CLI). */
  installing: boolean
  /** Ordered console lines for the current/last add. */
  stages: InstallStage[]
  /** Last install error message, or null. */
  error: string | null
  /** Kick off `worker::add` for the harness registry source. */
  install: () => void
  /** Clear the error and retry the add. */
  retry: () => void
}

/**
 * `WorkerCallRequest` — the payload bound functions receive on each `worker`
 * lifecycle transition. Parsed loosely: the engine is the source of truth and
 * we only read a handful of fields.
 */
const workerEventSchema = z.object({
  operation: z.string().optional(),
  stage: z.string().optional(),
  worker: z.string().nullable().optional(),
  source: z.unknown().optional(),
  version: z.string().nullable().optional(),
  status: z.string().nullable().optional(),
  progress: z.number().nullable().optional(),
  timestamp_ms: z.number().nullable().optional(),
  error: z
    .object({
      code: z.string().optional(),
      message: z.string().optional(),
    })
    .nullable()
    .optional(),
})
type WorkerEvent = z.infer<typeof workerEventSchema>

function parseWorkerEvent(payload: unknown): WorkerEvent | null {
  const parsed = workerEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

/** Loose match: is this lifecycle event about the harness worker? */
function isHarnessEvent(evt: WorkerEvent): boolean {
  const w = typeof evt.worker === 'string' ? evt.worker.toLowerCase() : ''
  if (w === HARNESS_WORKER_NAME || w.includes(HARNESS_WORKER_NAME)) return true
  if (evt.source != null) {
    try {
      if (
        JSON.stringify(evt.source).toLowerCase().includes(HARNESS_WORKER_NAME)
      ) {
        return true
      }
    } catch {
      // non-serialisable source; fall through
    }
  }
  return false
}

function toStage(evt: WorkerEvent): InstallStage {
  return {
    stage: evt.stage ?? 'unknown',
    worker: evt.worker ?? undefined,
    progress: typeof evt.progress === 'number' ? evt.progress : undefined,
    error: evt.error ?? undefined,
    at: typeof evt.timestamp_ms === 'number' ? evt.timestamp_ms : Date.now(),
  }
}

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats harness as present so the normal empty
 *   state shows).
 */
export function useHarnessStatus(enabled: boolean): HarnessStatus {
  const [present, setPresent] = useState<boolean>(!enabled)
  const [loading, setLoading] = useState<boolean>(enabled)
  const [state, setState] = useState<WorkerPresenceState>(
    enabled ? 'unknown' : 'connected',
  )
  const [detail, setDetail] = useState<string | null>(null)
  const [installing, setInstalling] = useState<boolean>(false)
  const [stages, setStages] = useState<InstallStage[]>([])
  const [error, setError] = useState<string | null>(null)
  const [probeNonce, setProbeNonce] = useState(0)

  // Process a single inbound `worker` lifecycle event. Uses functional
  // setState only so the handler has no reactive deps and stays stable.
  const handleEvent = useCallback((payload: unknown) => {
    const evt = parseWorkerEvent(payload)
    if (!evt) return
    if (evt.operation !== 'add' || !isHarnessEvent(evt)) return

    setStages((prev) => [...prev, toStage(evt)])

    if (evt.stage === 'failed') {
      setPresent(false)
      setState('stopped')
      setDetail(
        evt.error?.message ?? 'worker manager failed to start the harness',
      )
      setError(
        evt.error?.message
          ? normalizeErrorMessage(evt.error)
          : 'failed to add the harness worker',
      )
      setInstalling(false)
    } else if (evt.stage === 'done') {
      // A completed manager operation does not guarantee that the process has
      // registered with the engine yet. Reconcile both signals immediately.
      setPresent(false)
      setState('starting')
      setDetail(null)
      setError(null)
      setInstalling(false)
      setProbeNonce((nonce) => nonce + 1)
    } else {
      // started / downloading / downloaded — make sure the console is shown
      // even when the add was kicked off from the CLI rather than the CTA.
      setInstalling(true)
      setError(null)
    }
  }, [])

  // Live `worker` add subscription for install progress + CLI detection.
  // The shared hook always sends a `stages` filter — omitting it matches no
  // events (the engine filters on operations AND stages).
  useWorkerLifecycle({
    enabled,
    fnId: HARNESS_WATCH_FN,
    operations: ['add'],
    onEvent: handleEvent,
  })

  // Presence probe on mount and after a completed add. Reconcile the engine
  // catalogue with worker-manager status so an installed-but-stopped worker
  // does not masquerade as an absent worker.
  // biome-ignore lint/correctness/useExhaustiveDependencies: probeNonce is an explicit re-probe trigger after worker::add completes.
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
        const snapshot = await readWorkerPresence(client, HARNESS_WORKER_NAME)
        if (!cancelled) {
          setPresent(snapshot.present)
          setState(snapshot.state)
          setDetail(snapshot.detail)
          if (snapshot.present) setError(null)
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
  }, [enabled, probeNonce])

  const install = useCallback(() => {
    if (!enabled) return
    setError(null)
    setStages([])
    setInstalling(true)
    void (async () => {
      let client: Awaited<ReturnType<typeof getIiiClient>> | undefined
      try {
        client = await getIiiClient()
        await client.trigger('worker::add', {
          source: { kind: 'registry', name: HARNESS_WORKER_NAME },
          wait: true,
        })
        // Terminal success. Trigger events may have already flipped these, but
        // reconcile again for the no-events fallback and for the short window
        // between manager completion and engine registration.
        const snapshot = await readWorkerPresence(client, HARNESS_WORKER_NAME)
        setPresent(snapshot.present)
        setState(snapshot.state)
        setDetail(snapshot.detail)
        setInstalling(false)
        if (snapshot.present) setError(null)
      } catch (err) {
        setInstalling(false)
        // The add may have actually landed even though the call rejected
        // (e.g. a late timeout) — re-check before surfacing the error.
        try {
          if (!client) throw new Error('iii client unavailable')
          const snapshot = await readWorkerPresence(client, HARNESS_WORKER_NAME)
          setPresent(snapshot.present)
          setState(snapshot.state)
          setDetail(snapshot.detail)
          if (snapshot.present) {
            setError(null)
            return
          }
        } catch {
          // ignore; fall through to the original error
        }
        setError(normalizeErrorMessage(err))
      }
    })()
  }, [enabled])

  return useMemo(
    () => ({
      present,
      loading,
      state,
      detail,
      installing,
      stages,
      error,
      install,
      retry: install,
    }),
    [present, loading, state, detail, installing, stages, error, install],
  )
}

/**
 * Whether harness-owned bus functions (`router::models::list`, etc.) are
 * registered and safe to trigger. False during the initial presence probe and
 * while the worker is absent.
 *
 * NOTE: `approval::*` is NOT harness-owned — it lives on the standalone,
 * optional `approval-gate` worker. Gate approval logic on
 * `useApprovalGateStatus` / `isApprovalGateAvailable`, not this.
 */
export function isHarnessAvailable(status: HarnessStatus): boolean {
  return status.present && !status.loading
}

/**
 * Whether the chat composer should be locked. Mirrors the empty-state gating:
 * hold off while the initial presence probe is in flight (avoids a flash of
 * disabled chrome), then block when harness is absent, installing, or failed.
 */
export function isChatBlockedByHarness(status: HarnessStatus): boolean {
  if (status.loading) return false
  if (!status.present) return true
  if (status.installing) return true
  if (status.error != null) return true
  return false
}

/** Composer placeholder copy while the harness gate is closed. */
export function harnessComposerPlaceholder(status: HarnessStatus): string {
  if (status.installing) return 'installing harness…'
  if (status.state === 'starting') return 'starting harness…'
  if (status.state === 'provisioning') return 'preparing harness…'
  if (status.state === 'stopped') return 'harness is stopped…'
  if (status.state === 'unknown') return 'checking harness…'
  return 'install the harness worker to send a message…'
}
