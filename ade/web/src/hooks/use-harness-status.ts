import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { followOperation } from '@/pages/Workers/follow-operation'
import { workerListed } from './use-worker-presence'
import { useWorkersChanged } from './use-workers-changed'

/**
 * Watches the engine and the compose daemon for the `harness` worker and
 * drives an in-app install, start or update of it.
 *
 * `present` is an `engine::workers::list` read on mount, read again on every
 * `engine::on-workers-changed` event, so the UI reacts the instant harness is
 * added — whether from the in-app CTA or an `iii call compose::add` run in
 * the operator's own terminal (this is the educational, "you can do this in
 * the CLI too" angle).
 *
 * `stopped` is a `compose::list` read: the harness is declared in a compose
 * project but its container is not running. Read on mount, again on every
 * `ade::compose::on-changed` event (the daemon wrote its state, or the
 * compose file changed) and alongside every presence re-read. A
 * `compose::add` of a worker that is already declared changes nothing, so a
 * stopped harness needs `start()` (`compose::up`) or `updateAndStart()`
 * (`compose::update`, then `compose::up` if it is still down) instead of
 * `install()`.
 */

/** Registry name + engine worker name + compose container for the harness meta-worker. */
const HARNESS_WORKER_NAME = 'harness'
/** Base id for the browser-local `engine::on-workers-changed` handler. */
const HARNESS_WATCH_FN = 'ade::harness-watch'
/** Base id for the browser-local `ade::compose::on-changed` handler. */
const HARNESS_COMPOSE_WATCH_FN = 'ade::harness-compose-watch'
/** The ADE's compose project watch: called when the daemon's state or the compose file changes. */
const COMPOSE_CHANGED_FN = 'ade::compose::on-changed'
/** `compose::up` waits for the container to come up; the Workers page gives it the same. */
const COMPOSE_UP_TIMEOUT_MS = 600_000

/** The compose action in flight or the one that failed. */
export type HarnessAction = 'add' | 'start' | 'update'

export interface HarnessStatus {
  /** Whether the `harness` worker is currently connected. */
  present: boolean
  /** Initial presence probe in flight. */
  loading: boolean
  /**
   * Declared in a compose project but not running (stopped or failed), and
   * not connected to the engine. `install()` would change nothing here;
   * `start()` or `updateAndStart()` brings it up.
   */
  stopped: boolean
  /** An in-app add, start or update is in progress. */
  installing: boolean
  /** Which action is in progress, or which one failed; null when neither. */
  action: HarnessAction | null
  /** Last action's error message, or null. */
  error: string | null
  /** Add the harness registry worker to the active Compose project. */
  install: () => void
  /** Start the declared, stopped harness container (`compose::up`). */
  start: () => void
  /** Update the declared harness to the latest release, then start it. */
  updateAndStart: () => void
  /** Clear the error and retry the last action. */
  retry: () => void
}

/** Where compose has the harness declared: the project file and the container's state. */
export interface HarnessDeclaration {
  file: string | undefined
  state: string
}

/** Container states in which compose is bringing the harness up by itself. */
const STARTING_STATES: ReadonlySet<string> = new Set(['starting', 'restarting'])
/** Container states in which compose considers the harness up. */
const RUNNING_STATES: ReadonlySet<string> = new Set([
  'starting',
  'restarting',
  'ready',
])

interface ComposeProjectList {
  projects?: {
    file?: string
    containers?: { container?: string; state?: string }[]
  }[]
}

/**
 * The harness's compose declaration in a `compose::list` answer, or null
 * when no loaded project declares it (or the daemon did not answer).
 */
export function harnessDeclaration(list: unknown): HarnessDeclaration | null {
  const projects = (list as ComposeProjectList | null | undefined)?.projects
  if (!Array.isArray(projects)) return null
  for (const project of projects) {
    const container = project?.containers?.find(
      (entry) => entry?.container === HARNESS_WORKER_NAME,
    )
    if (container) {
      return {
        file: typeof project.file === 'string' ? project.file : undefined,
        state: typeof container.state === 'string' ? container.state : '',
      }
    }
  }
  return null
}

async function checkHarnessPresent(client: IiiClient): Promise<boolean> {
  return workerListed(
    await client.call('engine::workers::list', {}),
    HARNESS_WORKER_NAME,
  )
}

/** The harness's compose declaration; null when compose does not answer. */
async function readHarnessDeclaration(
  client: Pick<IiiClient, 'call'>,
): Promise<HarnessDeclaration | null> {
  try {
    return harnessDeclaration(
      await client.call('compose::list', {}, { timeoutMs: 10_000 }),
    )
  } catch {
    return null
  }
}

function withFile(
  file: string | undefined,
  payload: Record<string, unknown>,
): Record<string, unknown> {
  return file ? { file, ...payload } : payload
}

/** Add harness to the active Compose project and wait for reconciliation. */
export async function addHarnessToCompose(
  client: Pick<IiiClient, 'call'>,
): Promise<void> {
  await client.call('compose::add', {
    workers: [HARNESS_WORKER_NAME],
  })
}

/** Start the declared harness container; `compose::up` answers once it is up or failed. */
export async function startHarnessInCompose(
  client: Pick<IiiClient, 'call'>,
  file: string | undefined,
): Promise<void> {
  const outcome = await client.call<{
    status?: 'ok' | 'failed'
    error?: { code: string; message: string } | null
  }>('compose::up', withFile(file, { container: HARNESS_WORKER_NAME }), {
    timeoutMs: COMPOSE_UP_TIMEOUT_MS,
  })
  if (outcome?.status === 'failed') {
    throw new Error(outcome.error?.message || 'compose::up failed')
  }
}

/**
 * Update the declared harness to the latest release, following the
 * operation by its pushed progress, then start it if the update left it
 * down (an update restarts only what changed; a stopped container with
 * nothing new stays stopped).
 */
export async function updateAndStartHarnessInCompose(
  client: Pick<IiiClient, 'call' | 'on' | 'observe' | 'browserId'>,
  file: string | undefined,
): Promise<void> {
  const snapshot = await followOperation({
    iii: client,
    start: (operationId) =>
      client.call(
        'compose::update',
        withFile(file, {
          workers: [HARNESS_WORKER_NAME],
          operation_id: operationId,
        }),
        { timeoutMs: 60_000 },
      ),
    read: (operation_id) =>
      client.call(
        'compose::operation',
        { operation_id },
        { timeoutMs: 10_000 },
      ),
  })
  if (snapshot === null) return
  if (snapshot.status !== 'succeeded') {
    throw new Error(
      snapshot.last_event?.detail || `compose::update ${snapshot.status}`,
    )
  }
  const declared = await readHarnessDeclaration(client)
  if (declared && !RUNNING_STATES.has(declared.state)) {
    await startHarnessInCompose(client, declared.file ?? file)
  }
}

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats harness as present so the normal empty
 *   state shows).
 */
export function useHarnessStatus(enabled: boolean): HarnessStatus {
  const [present, setPresent] = useState<boolean>(!enabled)
  const [declared, setDeclared] = useState<HarnessDeclaration | null>(null)
  const [loading, setLoading] = useState<boolean>(enabled)
  const [busy, setBusy] = useState<boolean>(false)
  const [action, setAction] = useState<HarnessAction | null>(null)
  const [error, setError] = useState<string | null>(null)
  const declaredRef = useRef(declared)
  declaredRef.current = declared

  // Overlapping reads can resolve out of order: a reply older than one
  // already applied is dropped so it never overwrites newer presence.
  const reads = useRef({ issued: 0, applied: 0 })
  const applyPresence = useCallback((seq: number, found: boolean) => {
    if (seq <= reads.current.applied) return
    reads.current.applied = seq
    setPresent(found)
  }, [])

  // The same for the compose declaration.
  const composeReads = useRef({ issued: 0, applied: 0 })
  const applyDeclaration = useCallback(
    (seq: number, next: HarnessDeclaration | null) => {
      if (seq <= composeReads.current.applied) return
      composeReads.current.applied = seq
      setDeclared(next)
    },
    [],
  )

  // Compose state changed (the daemon started or stopped a container, or
  // the file changed): read the declaration again.
  const recheckCompose = useCallback(() => {
    const seq = ++composeReads.current.issued
    void (async () => {
      try {
        const client = await getIiiClient()
        applyDeclaration(seq, await readHarnessDeclaration(client))
      } catch {
        // keep the last known declaration
      }
    })()
  }, [applyDeclaration])

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
    recheckCompose()
  }, [applyPresence, recheckCompose])

  useWorkersChanged({ enabled, fnId: HARNESS_WATCH_FN, onChange: recheck })

  // `ade::compose::on-changed`, with a browser-local handler unique per tab
  // and per mount (same scheme as `useWorkersChanged`).
  const instanceId = useId().replace(/[^a-zA-Z0-9]/g, '')
  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let offHandler: (() => void) | undefined
    let offObserver: (() => void) | undefined
    void (async () => {
      try {
        const client = await getIiiClient()
        if (cancelled) return
        const localFnId = `${HARNESS_COMPOSE_WATCH_FN}::${instanceId}`
        offHandler = client.on(localFnId, () => recheckCompose())
        offObserver = client.observe(
          COMPOSE_CHANGED_FN,
          `${localFnId}::${client.browserId}`,
        )
      } catch {
        offObserver?.()
        offHandler?.()
        offObserver = undefined
        offHandler = undefined
      }
    })()
    return () => {
      cancelled = true
      offObserver?.()
      offHandler?.()
    }
  }, [enabled, instanceId, recheckCompose])

  // Presence and declaration probes on mount; later changes arrive through
  // the observers above. No polling, no focus re-checks.
  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      setPresent(true)
      return
    }
    let cancelled = false
    const seq = ++reads.current.issued
    const composeSeq = ++composeReads.current.issued
    void (async () => {
      const client = await getIiiClient()
      try {
        const [found, declaration] = await Promise.all([
          checkHarnessPresent(client).catch(() => false),
          readHarnessDeclaration(client),
        ])
        if (cancelled) return
        applyPresence(seq, found)
        applyDeclaration(composeSeq, declaration)
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [enabled, applyPresence, applyDeclaration])

  const run = useCallback(
    (next: HarnessAction, perform: (client: IiiClient) => Promise<void>) => {
      if (!enabled) return
      setError(null)
      setAction(next)
      setBusy(true)
      void (async () => {
        const client = await getIiiClient()
        try {
          await perform(client)
          // Terminal success. A workers-changed event may have already set
          // present; setting it again is harmless. Reads still in flight are
          // older than this and must not undo it.
          applyPresence(++reads.current.issued, true)
          setBusy(false)
        } catch (err) {
          setBusy(false)
          // The action may have actually landed even though the call
          // rejected (e.g. a late timeout) — re-check before surfacing the
          // error. Numbered before the read, so a re-read issued meanwhile
          // wins.
          const seq = ++reads.current.issued
          try {
            if (await checkHarnessPresent(client)) {
              applyPresence(seq, true)
              setError(null)
              return
            }
          } catch {
            // ignore; fall through to the original error
          }
          setError(normalizeErrorMessage(err))
        } finally {
          recheckCompose()
        }
      })()
    },
    [enabled, applyPresence, recheckCompose],
  )

  const install = useCallback(() => run('add', addHarnessToCompose), [run])
  const start = useCallback(
    () =>
      run('start', (client) =>
        startHarnessInCompose(client, declaredRef.current?.file),
      ),
    [run],
  )
  const updateAndStart = useCallback(
    () =>
      run('update', (client) =>
        updateAndStartHarnessInCompose(client, declaredRef.current?.file),
      ),
    [run],
  )
  const retry = useCallback(() => {
    if (action === 'start') start()
    else if (action === 'update') updateAndStart()
    else install()
  }, [action, install, start, updateAndStart])

  return useMemo(() => {
    // Compose is bringing it up on its own (a CLI `compose::up`, a restart):
    // show that as a start in progress rather than a stopped harness.
    const composeStarting =
      !present && declared !== null && STARTING_STATES.has(declared.state)
    const stopped = !present && declared !== null && !composeStarting
    const installing = busy || (error === null && composeStarting)
    return {
      present,
      loading,
      stopped,
      installing,
      action: busy || error !== null ? action : installing ? 'start' : null,
      error,
      install,
      start,
      updateAndStart,
      retry,
    }
  }, [
    present,
    declared,
    loading,
    busy,
    action,
    error,
    install,
    start,
    updateAndStart,
    retry,
  ])
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
  if (status.installing) {
    if (status.action === 'start') return 'starting harness…'
    if (status.action === 'update') return 'updating harness…'
    return 'installing harness…'
  }
  if (status.stopped) return 'start the harness worker to send a message…'
  return 'install the harness worker to send a message…'
}
