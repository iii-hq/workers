/**
 * Shared visual vocabulary for timeline spans — the live TimelineStrip and
 * the static TraceTimeline resolve a span's color the same way, so a span
 * reads identically wherever it renders. The detail-view extras (kind
 * icons, in-bar labels, the hover/selection ring recipe) live here too.
 */

import { Flame, Sparkle, SquareFunction, Zap } from 'lucide-react'
import type { CSSProperties } from 'react'
import { getWorkerColor, SPAN_STATUS_COLORS } from '../../lib/traceColors'
import type { TimelineSpan, TimelineSpanKind } from './layout'

export const KIND_ICONS: Record<
  TimelineSpanKind,
  typeof Zap | typeof Sparkle | typeof Flame | typeof SquareFunction
> = {
  zap: Zap,
  sparkle: Sparkle,
  flame: Flame,
  lambda: SquareFunction,
}

/**
 * Whether a bar earns a kind glyph. `lambda` — a worker function
 * invocation — is what most bars are, so a glyph on each of them was a
 * repeated "F" that told the reader nothing; the other kinds (an inbound
 * root, an outbound client call such as an LLM, a queue hop) are the
 * exceptions worth marking.
 */
export function showsKindGlyph(kind: TimelineSpanKind): boolean {
  return kind !== 'lambda'
}

/**
 * A span's colour: alert on error, else the override the mapper set, else
 * the worker's hue — by `workerKey` when the bar carries one (the strip's
 * spans do), so the masthead and the detail views colour one worker the
 * same way; a bar with neither falls back to its label.
 */
export function resolveColor(span: TimelineSpan): string {
  if (span.status === 'error') return SPAN_STATUS_COLORS.error
  return span.color ?? getWorkerColor(span.workerKey ?? span.label ?? span.id)
}

/**
 * A detail-view bar is a solid block of its colour — the worker's dusty
 * mid-tone fill, the charts' red on error — the way the masthead strip
 * draws its lines, so one worker is one unbroken colour across the page.
 * The label and the kind glyph sit on it in the fill's own ink
 * (`traceColors.inkOnFill`: near-black on a worker, white on an error),
 * and the bar wears its one-step-darker edge (`traceColors.borderOnFill`)
 * as an inset hairline; the ring below sits on top of that edge, for
 * state.
 *
 * Ring stack for bars. Selection is the page ink, two pixels, with a one
 * pixel gap of panel colour so it reads as a ring and not a thicker bar;
 * hover is a single pixel of half-strength ink. Selection beats hover.
 */
export function ringFor(
  selected: boolean,
  hovered: boolean,
): string | undefined {
  if (selected)
    return '0 0 0 1px var(--color-panel-raised), 0 0 0 3px var(--color-ink)'
  if (hovered)
    return '0 0 0 1px color-mix(in oklab, var(--color-ink) 55%, transparent)'
  return undefined
}

/** LRM mark: pins the label's edge punctuation to LTR inside the RTL clip. */
const LRM = '‎'

const BAR_LABEL_STYLE: CSSProperties = {
  // Leading-ellipsis trick: RTL direction makes text-overflow clip the
  // START of the (LTR) name, so a too-long "chat.respond.stream" reads
  // "…respond.stream" — the tail is the discriminating part of span names.
  direction: 'rtl',
  textAlign: 'left',
}

/**
 * The span name inside a bar, in the bar's own ink (it inherits the bar's
 * `color`). Reveals naturally as the bar grows (flex min-w-0 + overflow
 * clip); when it doesn't fit, the ellipsis sits at the BEGINNING of the
 * name.
 */
export function BarLabel({ text }: { text: string }) {
  return (
    <span
      className="min-w-0 flex-1 overflow-hidden font-mono text-xs font-semibold leading-none whitespace-nowrap text-ellipsis"
      style={BAR_LABEL_STYLE}
    >
      {LRM}
      {text}
      {LRM}
    </span>
  )
}
