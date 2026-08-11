/**
 * Serialize an async reload behind one in-flight request. Doorbell events
 * arrive at-least-once and can burst (every claim/retire of a rapidly firing
 * binding rings), so `refresh()` calls during a flight coalesce into exactly
 * one trailing rerun — never overlapping requests, so an older response can
 * never land after (and overwrite) a newer one.
 *
 * `reset()` bumps the generation: an in-flight response is discarded instead
 * of applied, and its coalesced rerun is dropped. Callers use it when the
 * target changes (conversation switch) or on unmount.
 */
export function serialRefresh<T>(
  load: () => Promise<T>,
  apply: (value: T) => void,
): { refresh: () => void; reset: () => void } {
  let inFlight = false
  let pending = false
  let generation = 0
  const refresh = () => {
    if (inFlight) {
      pending = true
      return
    }
    inFlight = true
    const started = generation
    load()
      .then((value) => {
        if (generation === started) apply(value)
      })
      .catch(() => {})
      .finally(() => {
        if (generation !== started) return
        inFlight = false
        if (pending) {
          pending = false
          refresh()
        }
      })
  }
  return {
    refresh,
    reset: () => {
      generation += 1
      inFlight = false
      pending = false
    },
  }
}
