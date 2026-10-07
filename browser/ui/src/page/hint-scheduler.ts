/**
 * Pointer-driven hover-hint requests for annotate mode. Each pointer move
 * offers the newest page point; at most one request goes out per animation
 * frame and at most one is in flight, always for the newest point, and a
 * point already asked about is not asked again. When the pointer rests
 * nothing runs: no timer re-samples the cursor.
 */

export interface HintPoint {
  x: number
  y: number
}

export interface HintSchedulerOptions<R> {
  /** Ask the worker what a pin dropped at this page point would name. */
  request: (x: number, y: number) => Promise<R>
  /** The answer for `point`; `null` when the request failed. */
  onResult: (result: R | null, point: HintPoint) => void
  /** Frame scheduler (defaults to `requestAnimationFrame`). */
  frame?: (callback: () => void) => number
  cancelFrame?: (id: number) => void
}

export interface HintScheduler {
  /** The pointer moved to `point` (null: off the picture). */
  move: (point: HintPoint | null) => void
  dispose: () => void
}

const samePoint = (a: HintPoint | null, b: HintPoint | null) => a !== null && b !== null && a.x === b.x && a.y === b.y

export function createHintScheduler<R>(options: HintSchedulerOptions<R>): HintScheduler {
  const frame = options.frame ?? ((callback: () => void) => window.requestAnimationFrame(callback))
  const cancelFrame = options.cancelFrame ?? ((id: number) => window.cancelAnimationFrame(id))
  let latest: HintPoint | null = null
  let asked: HintPoint | null = null
  let pending: number | undefined
  let inFlight = false
  let disposed = false

  const schedule = () => {
    if (disposed || inFlight || pending !== undefined) return
    if (!latest || samePoint(latest, asked)) return
    pending = frame(flush)
  }

  const flush = () => {
    pending = undefined
    const point = latest
    if (disposed || !point || samePoint(point, asked)) return
    asked = point
    inFlight = true
    void options
      .request(point.x, point.y)
      .then(
        (result) => {
          if (!disposed) options.onResult(result, point)
        },
        () => {
          if (!disposed) options.onResult(null, point)
        },
      )
      .finally(() => {
        inFlight = false
        // The pointer kept moving while this was out: ask about where it
        // is now (next frame), and only then.
        schedule()
      })
  }

  return {
    move(point) {
      latest = point
      if (!point) {
        // Leaving the picture forgets the last answer, so coming back to
        // the same spot asks again.
        asked = null
        return
      }
      schedule()
    },
    dispose() {
      disposed = true
      if (pending !== undefined) cancelFrame(pending)
      pending = undefined
    },
  }
}
