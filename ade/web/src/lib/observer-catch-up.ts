/**
 * Ordering a catch-up read after an observer turns active.
 *
 * An observer only receives the calls made once the engine has activated it
 * ("can miss calls that happen before `ready()` resolves"), and on a
 * reconnect the SDK replays it: it passes through `suspended`/`pending` and
 * back to `active`, and the calls in between are lost. A live view that
 * observes and then reads the current state once must therefore read:
 *
 *   1. once every observer is active (`ready()`), or after a bounded wait so a
 *      source whose worker is absent never hangs the read, and
 *   2. again each time an observer comes back `active` after that read
 *      (a reconnect, or one that only turned active after the bounded wait).
 *
 * Events and reads stay idempotent at each call site; this only orders them.
 * A disposer without `ready`/`onStateChange` (a console older than 0.26)
 * keeps the old behaviour: one read straight away, no re-read.
 */

import type { ObserverDisposer } from '@/lib/iii-client'

/** What `observe` returns; an older console's disposer carries no state. */
export type CatchUpObserver = (() => void) &
  Partial<Pick<ObserverDisposer, 'state' | 'ready' | 'onStateChange'>>

/** Why a catch-up read runs: the first activation, or a later one. */
export type CatchUpCause = 'ready' | 'reactivated'

/** How long the first read waits for the observers to turn active. */
export const CATCH_UP_READY_WAIT_MS = 3_000

type Tracked = CatchUpObserver &
  Pick<ObserverDisposer, 'ready' | 'onStateChange'>

function tracksState(observer: unknown): observer is Tracked {
  const o = observer as Partial<Tracked> | null | undefined
  return (
    typeof o?.ready === 'function' && typeof o?.onStateChange === 'function'
  )
}

type Observers =
  | CatchUpObserver
  | null
  | undefined
  | ReadonlyArray<CatchUpObserver | null | undefined>

function listOf(observers: Observers): CatchUpObserver[] {
  const list = Array.isArray(observers) ? observers : [observers]
  return list.filter((o): o is CatchUpObserver => typeof o === 'function')
}

/**
 * Resolves once every observer is active, or after `timeoutMs`, whichever
 * comes first. Never rejects: a refused or disposed observer counts as
 * settled (the caller's read still runs). Resolves at once when no observer
 * carries `ready` (an older console).
 */
export function whenObserversReady(
  observers: Observers,
  timeoutMs: number = CATCH_UP_READY_WAIT_MS,
): Promise<void> {
  const waits = listOf(observers)
    .filter(tracksState)
    .map((o) => {
      try {
        return o.ready().catch(() => undefined)
      } catch {
        return Promise.resolve()
      }
    })
  if (waits.length === 0) return Promise.resolve()
  let timer: ReturnType<typeof setTimeout> | undefined
  return Promise.race([
    Promise.all(waits).then(() => undefined),
    new Promise<void>((resolve) => {
      timer = setTimeout(resolve, timeoutMs)
    }),
  ]).finally(() => clearTimeout(timer))
}

export interface CatchUpOptions {
  /** Bound on the first read's wait for `ready()`; defaults to 3 s. */
  readyTimeoutMs?: number
}

/**
 * Run `read('ready')` once every observer is active (bounded wait), then
 * `read('reactivated')` whenever one of them turns `active` again from any
 * other state. Returns a stop function: it drops the state listeners and
 * cancels a read still waiting for `ready()`. Without stateful observers the
 * first read runs synchronously, as the call sites did before 0.26.
 */
export function catchUpWhenActive(
  observers: Observers,
  read: (cause: CatchUpCause) => void,
  options: CatchUpOptions = {},
): () => void {
  const tracked = listOf(observers).filter(tracksState)
  let stopped = false
  const run = (cause: CatchUpCause) => {
    if (!stopped) read(cause)
  }
  if (tracked.length === 0) {
    run('ready')
    return () => {
      stopped = true
    }
  }
  let caughtUp = false
  const unwatch: Array<() => void> = []
  for (const observer of tracked) {
    let previous = observer.state
    try {
      unwatch.push(
        observer.onStateChange((state) => {
          const was = previous
          previous = state
          // Before the first read, that read covers the activation.
          if (state === 'active' && was !== 'active' && caughtUp) {
            run('reactivated')
          }
        }),
      )
    } catch {
      // An observer that cannot report its state: the first read stands.
    }
  }
  void whenObserversReady(tracked, options.readyTimeoutMs).then(() => {
    if (stopped) return
    caughtUp = true
    run('ready')
  })
  return () => {
    stopped = true
    for (const off of unwatch.splice(0)) {
      try {
        off()
      } catch {
        // already gone
      }
    }
  }
}

/** A subscription's cleanup that also exposes its observers, for a catch-up. */
export type ObservedSubscription = (() => void) & {
  readonly observers: readonly CatchUpObserver[]
}

/** Attach `observers` (the subscription's, or its parts') to `cleanup`. */
export function withObservers(
  cleanup: () => void,
  observers: ReadonlyArray<CatchUpObserver | ObservedSubscription>,
): ObservedSubscription {
  const list = observers.flatMap((o) =>
    Array.isArray((o as Partial<ObservedSubscription>).observers)
      ? [...(o as ObservedSubscription).observers]
      : [o as CatchUpObserver],
  )
  return Object.assign(cleanup, { observers: list })
}
