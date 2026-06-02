import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { z } from 'zod'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'

/**
 * Watches the engine for the `harness` worker and drives an in-app install of
 * it via `worker::add`, surfacing live progress.
 *
 * Two signals feed `present`:
 *   1. An initial `engine::workers::list` read on mount.
 *   2. A real-time `worker` lifecycle trigger (operations: ['add']) so the UI
 *      reacts the instant harness is added — whether from the in-app CTA or a
 *      `iii worker add harness` run in the operator's own terminal (this is the
 *      educational, "you can do this in the CLI too" angle).
 *
 * `install()` calls `worker::add` for the registry `harness` source. The call
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

async function checkHarnessPresent(client: IiiClient): Promise<boolean> {
  const res = await client.call<{ workers?: Array<{ name?: unknown }> }>(
    'engine::workers::list',
    {},
  )
  const workers = Array.isArray(res?.workers) ? res.workers : []
  return workers.some((w) => w?.name === HARNESS_WORKER_NAME)
}

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats harness as present so the normal empty
 *   state shows).
 */
export function useHarnessStatus(enabled: boolean): HarnessStatus {
  const [present, setPresent] = useState<boolean>(!enabled)
  const [loading, setLoading] = useState<boolean>(enabled)
  const [installing, setInstalling] = useState<boolean>(false)
  const [stages, setStages] = useState<InstallStage[]>([])
  const [error, setError] = useState<string | null>(null)

  // Process a single inbound `worker` lifecycle event. Uses functional
  // setState only so the handler has no reactive deps and stays stable.
  const handleEvent = useCallback((payload: unknown) => {
    const evt = parseWorkerEvent(payload)
    if (!evt) return
    if (evt.operation !== 'add' || !isHarnessEvent(evt)) return

    setStages((prev) => [...prev, toStage(evt)])

    if (evt.stage === 'failed') {
      setError(
        evt.error?.message
          ? normalizeErrorMessage(evt.error)
          : 'failed to add the harness worker',
      )
      setInstalling(false)
    } else if (evt.stage === 'done') {
      setPresent(true)
      setInstalling(false)
    } else {
      // started / downloading / downloaded — make sure the console is shown
      // even when the add was kicked off from the CLI rather than the CTA.
      setInstalling(true)
      setError(null)
    }
  }, [])

  // One-time presence probe + live `worker` subscription. Kept in a ref so
  // the latest handler runs without re-subscribing.
  const handlerRef = useRef(handleEvent)
  handlerRef.current = handleEvent

  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      setPresent(true)
      return
    }
    let cancelled = false
    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined

    void (async () => {
      const client = await getIiiClient()
      try {
        const found = await checkHarnessPresent(client)
        if (!cancelled) setPresent(found)
      } catch {
        if (!cancelled) setPresent(false)
      } finally {
        if (!cancelled) setLoading(false)
      }
      if (cancelled) return

      // Subscribe to harness add events for live progress + CLI detection.
      // If the engine doesn't publish the `worker` trigger type, fall back to
      // the progress-less path (install() re-checks presence on resolve).
      try {
        offHandler = client.on(HARNESS_WATCH_FN, (data: unknown) => {
          handlerRef.current(data)
        })
        offTrigger = client.registerTrigger({
          type: 'worker',
          function_id: `${HARNESS_WATCH_FN}::${client.browserId}`,
          config: { operations: ['add'] },
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
  }, [enabled])

  const install = useCallback(() => {
    if (!enabled) return
    setError(null)
    setStages([])
    setInstalling(true)
    void (async () => {
      const client = await getIiiClient()
      try {
        await client.call('worker::add', {
          source: { kind: 'registry', name: HARNESS_WORKER_NAME },
          wait: true,
        })
        // Terminal success. Trigger events may have already flipped these, but
        // setting them again is harmless and covers the no-events fallback.
        setPresent(true)
        setInstalling(false)
      } catch (err) {
        setInstalling(false)
        // The add may have actually landed even though the call rejected
        // (e.g. a late timeout) — re-check before surfacing the error.
        try {
          if (await checkHarnessPresent(client)) {
            setPresent(true)
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
      installing,
      stages,
      error,
      install,
      retry: install,
    }),
    [present, loading, installing, stages, error, install],
  )
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
  return 'install the harness worker to send a message…'
}
