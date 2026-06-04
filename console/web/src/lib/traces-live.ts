/**
 * Live-refresh wiring for the devtools Traces view, driven by the engine
 * `trace` trigger (the iii observability worker).
 *
 * Every span that lands in the engine's in-memory trace store fires the
 * `trace` trigger — the same client-agnostic mechanism as the `log` trigger,
 * available to any iii client. This module registers a browser-local handler,
 * binds a `trace` trigger to it, and — after a short trailing debounce —
 * invalidates the Traces React Query caches so the page refetches on real span
 * activity instead of polling `engine::traces::*` on a 3s timer.
 *
 * The engine fires once PER SPAN (mirroring the `log` trigger), so a busy turn
 * produces a burst; the debounce collapses that burst into a single refetch.
 *
 * The imperative core (`startTracesSubscription`) and the pure invalidation
 * handler (`makeTracesChangedHandler`) are framework-free so they unit-test
 * without a DOM; `useTracesLiveRefresh` is the thin React wrapper.
 */

import type { QueryClient } from '@tanstack/react-query'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'

/**
 * Browser-local function the engine `trace` trigger invokes. The `iii::`
 * prefix marks it engine-internal (`is_iii_builtin_function_id`), so the spans
 * produced by DELIVERING this trigger are tagged `iii.function.kind=internal`
 * — hidden from the Traces view by the default `include_internal:false` query,
 * and skipped by the engine's trigger loop-break. Without this, the trigger's
 * own delivery calls flood the trace list with `traces_changed` spans.
 */
const TRACES_CHANGED_FN = 'iii::console::traces_changed'
/** Engine trigger type registered by the observability worker. */
const TRACE_TRIGGER_TYPE = 'trace'
/** Trailing-edge debounce: collapse a burst of per-span ticks into one refetch. */
const DEFAULT_COALESCE_MS = 400

/** Dev-only trace-stream diagnostics. Silent in production builds. */
function dlog(msg: string, data?: unknown): void {
  if (import.meta.env?.DEV) {
    console.debug(`[traces-live] ${msg}`, data ?? '')
  }
}

/**
 * Build the signal handler that refetches the Traces queries. Pause is read
 * live from a ref so toggling pause never re-creates the subscription. We skip
 * refetching while the tab is hidden to avoid background work; the
 * `visibilitychange` re-sync wired up in `startTracesSubscription` catches up
 * on return. (The app-wide QueryClient disables `refetchOnWindowFocus`, so the
 * visibility listener — not focus — is the recovery path.)
 */
export function makeTracesChangedHandler(
  qc: QueryClient,
  isPausedRef: { current: boolean },
  onExtra?: () => void,
): () => void {
  return () => {
    if (isPausedRef.current) {
      dlog('signal ignored (paused)')
      return
    }
    if (
      typeof document !== 'undefined' &&
      document.visibilityState === 'hidden'
    ) {
      dlog('signal ignored (tab hidden)')
      return
    }
    dlog('signal received → invalidating traces queries')
    qc.invalidateQueries({ queryKey: ['traces'] })
    qc.invalidateQueries({ queryKey: ['traceGroups'] })
    // Extra refresh hook — e.g. silently reload the open trace's detail tree,
    // which isn't a React Query cache and so isn't covered by the invalidations
    // above. Shares the pause/hidden gating.
    onExtra?.()
  }
}

/**
 * Register a browser-local handler, bind an engine `trace` trigger to it, and
 * re-sync on reconnect / tab-visible. Returns a cleanup that clears the
 * debounce timer, unregisters the handler + connection listener, and
 * unregisters the trigger.
 *
 * The iii-browser-sdk replays BOTH registered functions and registered
 * triggers on reconnect (see `onSocketOpen`), so we do NOT manually
 * re-register on `'connected'` — that would create a duplicate trigger. We
 * only re-sync via `onSignal()` so a gap (or a cold initial fetch that raced
 * the WS connect) recovers without a polling fallback.
 *
 * Per-span ticks are coalesced by a trailing debounce; reconnect/visibility
 * re-syncs call `onSignal` directly (immediate). `onSignal` itself honors the
 * pause/hidden gates, so direct re-syncs stay correct.
 */
export function startTracesSubscription(
  client: Pick<
    IiiClient,
    'browserId' | 'on' | 'registerTrigger' | 'addConnectionStateListener'
  >,
  onSignal: () => void,
  opts: {
    coalesceMs?: number
    doc?: Pick<
      Document,
      'addEventListener' | 'removeEventListener' | 'visibilityState'
    >
  } = {},
): () => void {
  const coalesceMs = opts.coalesceMs ?? DEFAULT_COALESCE_MS
  const doc =
    opts.doc ?? (typeof document !== 'undefined' ? document : undefined)

  // Trailing-edge debounce: the engine fires once per span, so collapse a
  // burst into a single refetch.
  let timer: ReturnType<typeof setTimeout> | null = null
  const tick = () => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      onSignal()
    }, coalesceMs)
  }

  // The engine `trace` trigger calls this browser-local function per span.
  const off = client.on(TRACES_CHANGED_FN, tick)
  // `on()` registers under `<fn>::<browserId>`; the trigger must target that id.
  const functionId = `${TRACES_CHANGED_FN}::${client.browserId}`

  const offTrigger = client.registerTrigger({
    type: TRACE_TRIGGER_TYPE,
    function_id: functionId,
    config: {},
  })
  dlog('trace trigger registered', { functionId })

  const offConn = client.addConnectionStateListener((state) => {
    dlog('connection state', state)
    if (state !== 'connected') return
    // Handler + trigger are auto-replayed by the SDK; just re-sync to recover
    // spans that landed while the socket was down (and cold-start races).
    onSignal()
  })

  let offVisibility: (() => void) | undefined
  if (doc) {
    const onVisible = () => {
      if (doc.visibilityState !== 'visible') return
      dlog('tab visible → re-syncing traces queries')
      onSignal()
    }
    doc.addEventListener('visibilitychange', onVisible)
    offVisibility = () => doc.removeEventListener('visibilitychange', onVisible)
  }

  return () => {
    if (timer) {
      clearTimeout(timer)
      timer = null
    }
    off()
    offConn()
    offVisibility?.()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

/**
 * Subscribe the Traces page to live span activity via the engine `trace`
 * trigger for the lifetime of the component. The shared `getIiiClient()`
 * singleton is NOT disposed on unmount (it's app-wide), so the explicit
 * cleanup is required.
 *
 * `onSignal` runs on each (non-paused, visible) signal alongside the list
 * refetch — the page uses it to silently reload the open trace's detail tree,
 * which is fetched imperatively (not a React Query cache) and so isn't covered
 * by the query invalidations. Read live from a ref so it never re-subscribes.
 */
export function useTracesLiveRefresh({
  isPaused,
  onSignal,
}: {
  isPaused: boolean
  onSignal?: () => void
}): void {
  const qc = useQueryClient()
  const isPausedRef = useRef(isPaused)
  const onSignalRef = useRef(onSignal)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])
  useEffect(() => {
    onSignalRef.current = onSignal
  }, [onSignal])

  useEffect(() => {
    let stop: (() => void) | undefined
    let disposed = false
    void (async () => {
      const client = await getIiiClient()
      if (disposed) return
      stop = startTracesSubscription(
        client,
        makeTracesChangedHandler(qc, isPausedRef, () =>
          onSignalRef.current?.(),
        ),
      )
    })()
    return () => {
      disposed = true
      stop?.()
    }
    // Pause + onSignal are read via refs, so the subscription is set up once
    // per mount.
  }, [qc])
}
