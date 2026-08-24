/**
 * Coalesce refresh requests per resource. One request may run at a time and
 * any number of arrivals while it is in flight become exactly one follow-up.
 *
 * @param {() => Promise<void>} run
 */
export function createRefreshGate(run) {
  /** @type {Promise<void> | null} */
  let inFlight = null
  let pending = false

  const request = () => {
    if (inFlight) {
      pending = true
      return inFlight
    }

    inFlight = (async () => {
      try {
        await run()
      } finally {
        inFlight = null
        if (pending) {
          pending = false
          void request()
        }
      }
    })()
    return inFlight
  }

  const whenIdle = async () => {
    while (inFlight) {
      const current = inFlight
      try {
        await current
      } catch {
        // The caller owns request errors; this helper only waits for idleness.
      }
      if (inFlight === current) await Promise.resolve()
    }
  }

  return { request, whenIdle }
}
