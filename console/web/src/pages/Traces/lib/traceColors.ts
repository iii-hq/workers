/**
 * Worker color resolution under the iii Schematic.
 *
 * The schematic is a single-accent palette (DESIGN.md §3). Worker
 * identity is encoded through ink shades, never through a chromatic
 * palette. Errors collapse onto `--color-alert`; the currently active
 * trace/span collapses onto `--color-accent`. Everything else picks one
 * of four ink shades based on a hash of the worker name so that runs
 * with the same workers produce stable, distinguishable bars without
 * introducing color.
 *
 * `getWorkerColor` returns a CSS color reference (suitable for inline
 * `style={{ backgroundColor: ... }}` or as the second argument to an
 * SVG `stroke`/`fill`). The shades are CSS variables so theme swaps
 * automatically.
 */

const INK_SHADES = [
  'var(--color-ink)',
  'var(--color-ink-faint)',
  'var(--color-ink-ghost)',
  'var(--color-rule)',
] as const

function hashString(input: string): number {
  let h = 0
  for (let i = 0; i < input.length; i++) {
    h = (h * 31 + input.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

export function getWorkerColor(worker: string): string {
  return INK_SHADES[hashString(worker) % INK_SHADES.length]
}

export const SPAN_STATUS_COLORS = {
  ok: 'var(--color-ink)',
  error: 'var(--color-alert)',
  pending: 'var(--color-warn)',
  unset: 'var(--color-ink-ghost)',
} as const

export type SpanStatusKey = keyof typeof SPAN_STATUS_COLORS
