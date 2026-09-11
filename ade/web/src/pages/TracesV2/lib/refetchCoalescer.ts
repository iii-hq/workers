/**
 * Coalesces a stream of "something changed" notifications into bounded
 * refetches: at most one refetch in flight, exactly one trailing rerun for
 * notifications that landed mid-flight, and a floor on the interval between
 * consecutive starts.
 *
 * Why the trace surfaces cannot simply invalidate on every activity tick:
 * react-query's `invalidateQueries` CANCELS the fetch already in flight
 * (`cancelRefetch` defaults to true) and starts another. That cancellation is
 * client-side only — the engine keeps executing the abandoned scan — so under
 * sustained span activity (the `trace` trigger ticks every ~300ms) the engine
 * accumulates overlapping scans, their latency climbs past the tick interval,
 * and every response is discarded before it can land. Measured live
 * (MOT-4621, four concurrent harness turns): six `engine::traces::list` calls
 * in flight at once, p50 1.9s and max 5.9s per call, and a list that stayed
 * frozen for a minute of activity, then jumped to the final state in one
 * batch — rows appearing, flashing and switching status all at once.
 *
 * With the coalescer a busy engine sees one scan per surface at a time, the
 * list settles after every burst (the trailing rerun carries whatever the
 * in-flight scan missed), and updates arrive at a steady cadence instead of
 * in bursts.
 */

type TimerId = ReturnType<typeof setTimeout>

export interface RefetchCoalescerOptions {
  /** The refetch. Its promise settling (either way) frees the in-flight slot. */
  run: () => Promise<unknown>
  /** Notifications closer together than this collapse into one refetch. */
  debounceMs: number
  /** Minimum ms between two consecutive refetch STARTS. A function is read
   *  each time a refetch is armed, so callers can vary it with their state
   *  (e.g. a costlier seed under a text search). */
  minIntervalMs: number | (() => number)
  /** Checked right before a refetch starts; `false` drops it (a later
   *  notification arms a fresh one). Paused / hidden-tab gates live here. */
  shouldRun?: () => boolean
  /** Clock and timers, injectable for tests. */
  now?: () => number
  setTimeout?: (fn: () => void, ms: number) => TimerId
  clearTimeout?: (id: TimerId) => void
}

export interface RefetchCoalescer {
  /** A change notification landed. */
  request(): void
  /** True while a refetch is running. */
  readonly inFlight: boolean
  /** Drop the armed timer and any pending rerun; an in-flight refetch still
   *  settles on its own but nothing follows it. */
  dispose(): void
}

export function createRefetchCoalescer(
  options: RefetchCoalescerOptions,
): RefetchCoalescer {
  const now = options.now ?? (() => Date.now())
  const schedule =
    options.setTimeout ?? ((fn: () => void, ms: number) => setTimeout(fn, ms))
  const cancel = options.clearTimeout ?? ((id: TimerId) => clearTimeout(id))
  const minInterval = () =>
    typeof options.minIntervalMs === 'function'
      ? options.minIntervalMs()
      : options.minIntervalMs

  let timer: TimerId | undefined
  let inFlight = false
  let rerun = false
  let disposed = false
  let lastStart = Number.NEGATIVE_INFINITY

  const settle = () => {
    inFlight = false
    if (disposed || !rerun) return
    rerun = false
    arm()
  }

  const start = () => {
    inFlight = true
    lastStart = now()
    let promise: Promise<unknown>
    try {
      promise = Promise.resolve(options.run())
    } catch (error) {
      promise = Promise.reject(error)
    }
    // A failed refetch must not wedge the slot — the next notification
    // simply tries again.
    promise.then(settle, settle)
  }

  const arm = () => {
    if (disposed || timer !== undefined) return
    const wait = Math.max(options.debounceMs, lastStart + minInterval() - now())
    timer = schedule(() => {
      timer = undefined
      if (disposed) return
      if (inFlight) {
        rerun = true
        return
      }
      if (options.shouldRun && !options.shouldRun()) return
      start()
    }, wait)
  }

  return {
    request() {
      if (disposed) return
      if (inFlight) {
        rerun = true
        return
      }
      arm()
    },
    get inFlight() {
      return inFlight
    },
    dispose() {
      disposed = true
      rerun = false
      if (timer !== undefined) {
        cancel(timer)
        timer = undefined
      }
    },
  }
}
