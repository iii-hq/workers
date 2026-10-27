import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { useWorkersChanged } from './use-workers-changed'

/**
 * Watches the engine for the `harness` worker and drives an in-app install of
 * it via `compose::add`.
 *
 * `present` is an `engine::workers::list` read on mount, read again on every
 * `engine::workers-available` event, so the UI reacts the instant harness is
 * added — whether from the in-app CTA or a `iii trigger compose::add
 * worker=harness` run in the operator's own terminal (this is the
 * educational, "you can do this in the CLI too" angle).
 *
 * `install()` triggers `compose::add` for the registry `harness` worker and
 * resolves off its result and a presence re-check.
 */

/** Registry name + engine worker name for the harness meta-worker. */
const HARNESS_WORKER_NAME = 'harness'
/** Base id for the browser-local `engine::workers-available` handler. */
const HARNESS_WATCH_FN = 'console::harness-watch'

export interface HarnessStatus {
  /** Whether the `harness` worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
  /** An in-app add is in progress. */
  installing: boolean
  /** Last install error message, or null. */
  error: string | null
  /** Add the harness registry worker to the active Compose project. */
  install: () => void
  /** Clear the error and retry the add. */
  retry: () => void
}

async function checkHarnessPresent(client: IiiClient): Promise<boolean> {
  const res = await client.trigger<{ workers?: Array<{ name?: unknown }> }>(
    'engine::workers::list',
    {},
  )
  const workers = Array.isArray(res?.workers) ? res.workers : []
  return workers.some((w) => w?.name === HARNESS_WORKER_NAME)
}

/** Add harness to the active Compose project and wait for reconciliation. */
export async function addHarnessToCompose(
  client: Pick<IiiClient, 'trigger'>,
): Promise<void> {
  await client.trigger('compose::add', {
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
  const [error, setError] = useState<string | null>(null)

  // Overlapping reads can resolve out of order: a reply older than one
  // already applied is dropped so it never overwrites newer presence.
  const reads = useRef({ issued: 0, applied: 0 })
  const applyPresence = useCallback((seq: number, found: boolean) => {
    if (seq <= reads.current.applied) return
    reads.current.applied = seq
    setPresent(found)
  }, [])

  // Harness added or removed out of band (a CLI `compose::add`): read the
  // worker list again. A failed re-read keeps the last answer.
  const recheck = useCallback(() => {
    const seq = ++reads.current.issued
    void (async () => {
      try {
        const client = await getIiiClient()
        applyPresence(seq, await checkHarnessPresent(client))
      } catch {
        // keep the last known presence
      }
    })()
  }, [applyPresence])

  useWorkersChanged({ enabled, fnId: HARNESS_WATCH_FN, onChange: recheck })

  // Presence probe on mount; later changes arrive through the trigger above.
  // No polling, no focus re-checks.
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      setPresent(true)
      return
    }
    let cancelled = false
    const seq = ++reads.current.issued
    void (async () => {
      const client = await getIiiClient()
      try {
        const found = await checkHarnessPresent(client)
        if (!cancelled) applyPresence(seq, found)
      } catch {
        if (!cancelled) applyPresence(seq, false)
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [enabled, applyPresence])

  const install = useCallback(() => {
    if (!enabled) return
    setError(null)
    setInstalling(true)
    void (async () => {
      const client = await getIiiClient()
      try {
        await addHarnessToCompose(client)
        // Terminal success. A workers-available event may have already set
        // present; setting it again is harmless. Reads still in flight are
        // older than this and must not undo it.
        applyPresence(++reads.current.issued, true)
        setInstalling(false)
      } catch (err) {
        setInstalling(false)
        // The add may have actually landed even though the call rejected
        // (e.g. a late timeout) — re-check before surfacing the error.
        try {
          if (await checkHarnessPresent(client)) {
            applyPresence(++reads.current.issued, true)
            setError(null)
            return
          }
        } catch {
          // ignore; fall through to the original error
        }
        setError(normalizeErrorMessage(err))
      }
    })()
  }, [enabled, applyPresence])

  return useMemo(
    () => ({
      present,
      loading,
      installing,
      error,
      install,
      retry: install,
    }),
    [present, loading, installing, error, install],
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
  return 'install the harness worker to send a message…'
}
