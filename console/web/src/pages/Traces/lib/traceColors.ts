/**
 * Service color resolution under the iii Schematic.
 *
 * The schematic is a single-accent palette (DESIGN.md §3). Service
 * identity is encoded through ink shades, never through a chromatic
 * palette. Errors collapse onto `--color-alert`; the currently active
 * trace/span collapses onto `--color-accent`. Everything else picks one
 * of four ink shades based on a hash of the service name so that runs
 * with the same services produce stable, distinguishable bars without
 * introducing color.
 *
 * `getServiceColor` returns a CSS color reference (suitable for inline
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

export function getServiceColor(service: string): string {
  return INK_SHADES[hashString(service) % INK_SHADES.length]
}

export const SPAN_STATUS_COLORS = {
  ok: 'var(--color-ink)',
  error: 'var(--color-alert)',
  pending: 'var(--color-warn)',
  unset: 'var(--color-ink-ghost)',
} as const

export type SpanStatusKey = keyof typeof SPAN_STATUS_COLORS
