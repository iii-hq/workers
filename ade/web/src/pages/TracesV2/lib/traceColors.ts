/**
 * Worker color resolution for TracesV2.
 *
 * Worker identity is encoded through a stable 8-color chromatic palette
 * (the same one the timeline lab stories established): a worker name hashes
 * to one of eight hues, so runs with the same workers always produce the
 * same, distinguishable colors. The palette is theme-independent — the hues
 * hold enough contrast against both the cream and the dark paper.
 *
 * Status still overrides identity: errors collapse onto `--color-alert`,
 * and the currently active trace/span carries a neutral ink ring.
 *
 * `getWorkerColor` returns a CSS color (suitable for inline
 * `style={{ backgroundColor: ... }}` or as an SVG `stroke`/`fill`).
 */

/* Eight muted hues — periwinkle, teal, sand, mauve, sage, terracotta,
   steel, ochre — desaturated enough to sit on the neutral surfaces without
   shouting, distinct enough to tell workers apart at a glance. */
export const WORKER_PALETTE = [
  '#5e6ad2',
  '#4a9d9c',
  '#c08a5a',
  '#9b6fa8',
  '#6f9a74',
  '#c76c66',
  '#6c82a6',
  '#b89a4f',
] as const

function hashString(input: string): number {
  let h = 0
  for (let i = 0; i < input.length; i++) {
    h = (h * 31 + input.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

export function getWorkerColor(worker: string): string {
  return WORKER_PALETTE[hashString(worker) % WORKER_PALETTE.length]
}

export const SPAN_STATUS_COLORS = {
  ok: 'var(--color-ink)',
  error: '#d2524e',
  pending: 'var(--color-neutral-400)',
  unset: 'var(--color-ink-ghost)',
} as const

export type SpanStatusKey = keyof typeof SPAN_STATUS_COLORS
