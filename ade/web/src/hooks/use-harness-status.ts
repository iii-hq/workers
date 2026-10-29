import { useCallback, useEffect, useMemo, useState } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { workerListed } from './use-worker-presence'
import { useWorkersChanged } from './use-workers-changed'

/**
 * Watches the engine for the `harness` worker and drives an in-app install of
 * it via `compose::add`.
 *
 * Two signals feed `present`:
 *   1. An initial `engine::workers::list` read on mount.
 *   2. Every worker join `engine::on-workers-changed` announces, which reads
 *      the list again, so the UI reacts the instant harness connects —
 *      whether from the in-app CTA or an `iii call compose::add` run in the
 *      operator's own terminal (this is the educational, "you can do this in
 *      the CLI too" angle).
 *
 * `install()` calls `compose::add` for the registry `harness` worker and
 * resolves with a terminal outcome; the install console records the add's
 * start and the moment harness connected.
 */

/** Registry name + engine worker name for the harness meta-worker. */
const HARNESS_WORKER_NAME = 'harness'
/** Base id for the browser-local workers-changed handler. */
const HARNESS_WATCH_FN = 'console::harness-watch'

/** One line in the install console. */
export interface InstallStage {
  /** Lifecycle stage: started | done | failed */
  stage: string
  /** The worker being added. */
  worker?: string
  /** Optional 0..1 download progress. */
  progress?: number
  /** Present when `stage === 'failed'`. */
  error?: { code?: string; message?: string }
  /** When the stage was recorded (ms). */
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
  /** Add the harness registry worker to the active Compose project. */
  install: () => void
  /** Clear the error and retry the add. */
  retry: () => void
}

function stageNow(stage: string, error?: InstallStage['error']): InstallStage {
  return {
    stage,
    worker: HARNESS_WORKER_NAME,
    ...(error ? { error } : {}),
    at: Date.now(),
  }
}

/** Close a running add's console with `done`; any other console stays. */
function finishStages(prev: InstallStage[]): InstallStage[] {
  return prev.at(-1)?.stage === 'started' ? [...prev, stageNow('done')] : prev
}

async function checkHarnessPresent(client: IiiClient): Promise<boolean> {
  return workerListed(
    await client.call('engine::workers::list', {}),
    HARNESS_WORKER_NAME,
  )
}

/** Add harness to the active Compose project and wait for reconciliation. */
export async function addHarnessToCompose(
  client: Pick<IiiClient, 'call'>,
): Promise<void> {
  await client.call('compose::add', {
    workers: [HARNESS_WORKER_NAME],
  })
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

  // A worker joined: read the list again. Uses functional setState only so
  // the handler has no reactive deps and stays stable.
  const handleChange = useCallback(() => {
    void (async () => {
      try {
        const client = await getIiiClient()
        if (!(await checkHarnessPresent(client))) return
      } catch {
        return
      }
      setPresent(true)
      setInstalling(false)
      setStages(finishStages)
    })()
  }, [])

  useWorkersChanged({
    enabled,
    fnId: HARNESS_WATCH_FN,
    events: ['worker_connected'],
    onChange: handleChange,
  })

  // One-time presence probe on mount. Live changes — the in-app CTA or a CLI
  // `iii call compose::add` — arrive through the workers-changed observer
  // above (the engine's push channel). No polling, no focus re-checks.
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
        const found = await checkHarnessPresent(client)
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

  const install = useCallback(() => {
    if (!enabled) return
    setError(null)
    setStages([stageNow('started')])
    setInstalling(true)
    void (async () => {
      const client = await getIiiClient()
      try {
        await addHarnessToCompose(client)
        // Terminal success. The workers-changed event may have already
        // flipped these; setting them again is harmless and covers a missed
        // event.
        setPresent(true)
        setInstalling(false)
        setStages(finishStages)
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
        const message = normalizeErrorMessage(err)
        setStages((prev) => [...prev, stageNow('failed', { message })])
        setError(message)
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
 * Whether harness-owned bus functions (`router::models::list`, etc.) are
 * registered and safe to call. False during the initial presence probe and
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
  return 'install the harness worker to send a message…'
}
