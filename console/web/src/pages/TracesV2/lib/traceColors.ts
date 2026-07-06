/**
 * Service color resolution for TracesV2.
 *
 * Service identity is encoded through a stable 8-color chromatic palette
 * (the same one the timeline lab stories established): a service name hashes
 * to one of eight hues, so runs with the same services always produce the
 * same, distinguishable colors. The palette is theme-independent — the hues
 * hold enough contrast against both the cream and the dark paper.
 *
 * Status still overrides identity: errors collapse onto `--color-alert`,
 * and the currently active trace/span carries the accent ring.
 *
 * `getServiceColor` returns a CSS color (suitable for inline
 * `style={{ backgroundColor: ... }}` or as an SVG `stroke`/`fill`).
 */

export const SERVICE_PALETTE = [
  '#6366f1',
  '#0ea5e9',
  '#14b8a6',
  '#f59e0b',
  '#ec4899',
  '#8b5cf6',
  '#10b981',
  '#f97316',
] as const

function hashString(input: string): number {
  let h = 0
  for (let i = 0; i < input.length; i++) {
    h = (h * 31 + input.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

export function getServiceColor(service: string): string {
  return SERVICE_PALETTE[hashString(service) % SERVICE_PALETTE.length]
}

export const SPAN_STATUS_COLORS = {
  ok: 'var(--color-ink)',
  error: 'var(--color-alert)',
  pending: 'var(--color-warn)',
  unset: 'var(--color-ink-ghost)',
} as const

export type SpanStatusKey = keyof typeof SPAN_STATUS_COLORS
