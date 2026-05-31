/**
 * Live-refresh wiring for the devtools Traces view.
 *
 * The harness fans out an empty `ui::traces::changed` signal (coalesced from
 * the `agent::turn_end` stream) to every all-sessions subscriber. This module
 * subscribes to that signal and invalidates the Traces React Query caches so
 * the page refetches on a real "a turn's spans just landed" beat instead of
 * polling `engine::traces::*` on a 3s timer.
 *
 * The imperative core (`startTracesSubscription`) and the pure invalidation
 * handler (`makeTracesChangedHandler`) are framework-free so they unit-test
 * without a DOM; `useTracesLiveRefresh` is the thin React wrapper.
 */

import type { QueryClient } from '@tanstack/react-query'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef } from 'react'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'

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
 * on return. (The app-wide QueryClient disables `refetchOnWindowFocus`, so
 * focus refetch is NOT a recovery path — the visibility listener is.)
 */
export function makeTracesChangedHandler(
  qc: QueryClient,
  isPausedRef: { current: boolean },
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
  }
}

/**
 * Register the `ui::traces::changed` handler, subscribe this browser to all
 * sessions, and re-subscribe on every reconnect. Returns a cleanup that
 * unregisters the handler + connection listener and unsubscribes.
 *
 * `ui::subscribe` is a one-shot RPC (not a registered function), so the SDK
 * does NOT replay it on reconnect the way it replays `on(...)` handlers — if
 * the harness restarted while the socket was down, the subscription would be
 * lost. Re-subscribing on each `'connected'` transition closes that gap.
 * `ui::subscribe` is idempotent (a `Set.add` in the harness FanoutState), so
 * the redundant call on the initial connect is harmless.
 *
 * On every `'connected'` we also re-sync via `onSignal()` (a refetch). Without
 * polling, an initial fetch that raced the WS connect (or the harness coming
 * up) would otherwise leave the page blank until the next `agent::turn_end`.
 * Refetch-on-connect is event-driven — no interval — so it keeps the pure-push
 * model while covering cold-start and reconnect.
 *
 * Signals that fire while the tab is hidden are intentionally dropped by
 * `makeTracesChangedHandler`. To recover them, we re-sync via `onSignal()`
 * when the tab becomes visible again. `doc` is injectable so this is testable
 * without a DOM; it defaults to the global `document` (undefined in SSR/tests).
 */
export function startTracesSubscription(
  client: Pick<
    IiiClient,
    'browserId' | 'on' | 'call' | 'addConnectionStateListener'
  >,
  onSignal: () => void,
  doc:
    | Pick<
        Document,
        'addEventListener' | 'removeEventListener' | 'visibilityState'
      >
    | undefined = typeof document !== 'undefined' ? document : undefined,
): () => void {
  const off = client.on('ui::traces::changed', onSignal)
  dlog('subscription started; ui::traces::changed handler registered', {
    browserId: client.browserId,
  })

  const subscribe = () =>
    client
      .call('ui::subscribe', { browser_id: client.browserId, session_id: null })
      .then(() => dlog('ui::subscribe ok (all sessions)'))
      .catch((err) => dlog('ui::subscribe failed', err))

  subscribe()

  const offConn = client.addConnectionStateListener((state) => {
    dlog('connection state', state)
    if (state !== 'connected') return
    subscribe()
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
    off()
    offConn()
    offVisibility?.()
    client
      .call('ui::unsubscribe', {
        browser_id: client.browserId,
        session_id: null,
      })
      .catch(() => {})
  }
}

/**
 * Subscribe the Traces page to live `ui::traces::changed` signals for the
 * lifetime of the component. The shared `getIiiClient()` singleton is NOT
 * disposed on unmount (it's app-wide), so the explicit cleanup is required.
 */
export function useTracesLiveRefresh({
  isPaused,
}: {
  isPaused: boolean
}): void {
  const qc = useQueryClient()
  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])

  useEffect(() => {
    let stop: (() => void) | undefined
    let disposed = false
    void (async () => {
      const client = await getIiiClient()
      if (disposed) return
      stop = startTracesSubscription(
        client,
        makeTracesChangedHandler(qc, isPausedRef),
      )
    })()
    return () => {
      disposed = true
      stop?.()
    }
    // Pause is read via the ref, so the subscription is set up once per mount.
  }, [qc])
}
