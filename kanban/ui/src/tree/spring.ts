/**
 * A critically damped spring (no bounce, no overshoot) for motion the chart makes by itself — a card moving to
 * its new place, the camera re-fitting as the tree grows. Unlike a timed curve, a spring keeps its velocity when
 * its target moves mid-flight: a second change never restarts the motion, it bends it. That is what keeps a
 * chart that changes every few hundred milliseconds smooth.
 *
 * Response is the time constant a person reads as "how long it takes" (Apple's `response`); damping is critical.
 */
export const RESPONSE_S = 0.42
const STIFFNESS = ((2 * Math.PI) / RESPONSE_S) ** 2
const DAMPING = 2 * Math.sqrt(STIFFNESS)
/** Integration step: small and fixed, so a dropped frame changes nothing but how many steps run. */
const STEP_S = 1 / 240
/** Longest frame integrated in one go (a background tab coming back must not fling things). */
const MAX_FRAME_S = 1 / 20

/** One axis, pulled toward 0: `x` is the distance still to go, `v` its velocity (units per second). */
export type Axis = { x: number; v: number }

export const axis = (x = 0): Axis => ({ x, v: 0 })

export function advance(a: Axis, dtSeconds: number): void {
  const dt = Math.min(MAX_FRAME_S, Math.max(0, dtSeconds))
  const steps = Math.max(1, Math.ceil(dt / STEP_S))
  const h = dt / steps
  for (let i = 0; i < steps; i += 1) {
    const accel = -STIFFNESS * a.x - DAMPING * a.v
    a.v += accel * h
    a.x += a.v * h
  }
}

/** Close enough to rest to stop drawing: under a quarter pixel away and barely moving. */
export const settled = (a: Axis, scale = 1) => Math.abs(a.x) * scale < 0.25 && Math.abs(a.v) * scale < 6
