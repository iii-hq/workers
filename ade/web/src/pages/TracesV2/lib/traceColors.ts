/**
 * Worker color resolution for TracesV2.
 *
 * Worker identity is encoded through a stable 8-hue categorical ramp. The
 * values are theme tokens (`--color-worker-1…8` in index.css): dark-badge
 * triples in the Tailwind idiom — a `*-900` fill, a `*-800` edge
 * (`--color-worker-N-border`) and the hue's own `*-50` ink
 * (`--color-worker-N-ink`) — so a span chip is a deep, low-glare block
 * with light type in both themes.
 *
 * Which hue a worker gets: its name hashes to a preferred slot, and the
 * worker keeps that slot when no worker seen this session holds it — so
 * the same workers get the same colours run after run. When the preferred
 * slot is already taken (a hash collision, or more than eight workers),
 * the worker takes the least-used slot instead, scanning from its
 * preferred one; the ramp is ordered so neighbouring slots sit far apart
 * in hue, so the first eight workers of a session are always eight
 * distinct hues whatever their names hash to. Past eight the hues repeat,
 * and `workerSwatch` reports `reused` so a surface can add a second cue
 * (a hollow dot, a dotted edge) beside the name.
 *
 * The registry is a session-wide singleton: every surface (the list rows,
 * the masthead strip, the detail timeline, the breakdown, the span panel)
 * resolves a worker through it, so one worker is one colour everywhere.
 *
 * Status still overrides identity: errors collapse onto `--color-alert`,
 * and the currently active trace/span carries a neutral ink ring.
 *
 * `getWorkerColor` returns a CSS color (a `var(--…)` token — fine in an
 * inline `backgroundColor`, an SVG `fill`, or a `color-mix()`).
 */

export const WORKER_PALETTE = [
  'var(--color-worker-1)',
  'var(--color-worker-2)',
  'var(--color-worker-3)',
  'var(--color-worker-4)',
  'var(--color-worker-5)',
  'var(--color-worker-6)',
  'var(--color-worker-7)',
  'var(--color-worker-8)',
] as const

export interface WorkerSwatch {
  color: string
  /** index into `WORKER_PALETTE` */
  index: number
  /** another worker of this session already wore this hue when it was assigned */
  reused: boolean
}

function hashString(input: string): number {
  let h = 0
  for (let i = 0; i < input.length; i++) {
    h = (h * 31 + input.charCodeAt(i)) | 0
  }
  return Math.abs(h)
}

const assigned = new Map<string, WorkerSwatch>()
const usage: number[] = Array(WORKER_PALETTE.length).fill(0)

function assign(worker: string): WorkerSwatch {
  const preferred = hashString(worker) % WORKER_PALETTE.length
  let index = preferred
  if (usage[preferred] > 0) {
    // The least-used slot, first from the preferred one onwards: the ramp
    // is interleaved so consecutive slots are far apart in hue.
    let best = Number.POSITIVE_INFINITY
    for (let step = 0; step < WORKER_PALETTE.length; step++) {
      const candidate = (preferred + step) % WORKER_PALETTE.length
      if (usage[candidate] < best) {
        best = usage[candidate]
        index = candidate
      }
    }
  }
  const swatch: WorkerSwatch = {
    color: WORKER_PALETTE[index],
    index,
    reused: usage[index] > 0,
  }
  usage[index] += 1
  assigned.set(worker, swatch)
  return swatch
}

/** The worker's hue and whether this session already gave it to another worker. */
export function workerSwatch(worker: string): WorkerSwatch {
  return assigned.get(worker) ?? assign(worker)
}

export function getWorkerColor(worker: string): string {
  return workerSwatch(worker).color
}

const WORKER_TOKEN_RE = /^var\(--color-worker-(\d)\)$/

/**
 * The one-step-darker edge of a fill — `--color-worker-N-border` for a
 * worker hue. Any other fill (the error bar, a legacy hex override) gets
 * itself back, which paints as no visible border.
 */
export function borderOnFill(color: string): string {
  const m = WORKER_TOKEN_RE.exec(color.trim())
  return m ? `var(--color-worker-${m[1]}-border)` : color
}

/** The worker's edge token. */
export function getWorkerBorder(worker: string): string {
  return borderOnFill(getWorkerColor(worker))
}

/** Forget every assignment — for tests and stories that want a fresh ramp. */
export function resetWorkerColors(): void {
  assigned.clear()
  usage.fill(0)
}

/**
 * Status overrides identity. Error is the charts' own saturated red
 * (`--color-trace-error`, white type) rather than the page's `alert`,
 * whose dark coral sits too close to the salmon worker; pending is the
 * warn amber, the page's one "still running" hue.
 */
export const SPAN_STATUS_COLORS = {
  ok: 'var(--color-ink)',
  error: 'var(--color-trace-error)',
  pending: 'var(--color-warn)',
  unset: 'var(--color-ink-ghost)',
} as const

export type SpanStatusKey = keyof typeof SPAN_STATUS_COLORS

/**
 * Relative luminance of a `#rrggbb` colour (WCAG), or null for anything
 * that isn't a hex literal (a `var(--…)` token resolves at paint time, not
 * here).
 */
function hexLuminance(color: string): number | null {
  const m = /^#([0-9a-f]{6})$/i.exec(color.trim())
  if (!m) return null
  const channel = (i: number) => {
    const c = Number.parseInt(m[1].slice(i, i + 2), 16) / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/**
 * The ink that reads on a filled bar of `color`. A worker hue carries its
 * own near-black ink (`--color-worker-N-ink`) for its mid-tone fill; the
 * error fill carries white (`--color-trace-error-ink`); a raw hex fill (a
 * legacy `span.color` override) picks white or near-black by measured
 * contrast.
 */
export function inkOnFill(color: string): string {
  const worker = WORKER_TOKEN_RE.exec(color.trim())
  if (worker) return `var(--color-worker-${worker[1]}-ink)`
  if (color.trim() === SPAN_STATUS_COLORS.error)
    return 'var(--color-trace-error-ink)'
  const lum = hexLuminance(color)
  // Any other token (the pending amber) is tuned for `worker-ink`; only a
  // legacy hex override needs computing.
  if (lum === null) return 'var(--color-worker-ink)'
  // Contrast against white vs against near-black (#0a0a0a ≈ 0.003).
  const onWhite = 1.05 / (lum + 0.05)
  const onBlack = (lum + 0.05) / 0.053
  return onWhite >= onBlack ? '#ffffff' : '#0a0a0a'
}
