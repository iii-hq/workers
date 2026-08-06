/**
 * Context-usage tone: one pair of thresholds and one colour map, shared by
 * every surface that draws a usage bar (the session chip and the metrics
 * card's mini bar) so a bar can never say "fine" in one place and "alert" in
 * another.
 */

export type Tone = 'ok' | 'warn' | 'alert'

const WARN_THRESHOLD = 0.75
const DANGER_THRESHOLD = 0.9

export const TONE_COLOR: Record<Tone, string> = {
  ok: 'var(--color-accent)',
  warn: 'var(--color-warn)',
  alert: 'var(--color-alert)',
}

/** Tone for a used/usable ratio (already clamped by the caller). */
export function toneFor(ratio: number): Tone {
  if (ratio >= DANGER_THRESHOLD) return 'alert'
  if (ratio >= WARN_THRESHOLD) return 'warn'
  return 'ok'
}
