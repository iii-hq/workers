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

/** ink shades that are too light to carry a bg-colored icon (legacy
 * `span.color` overrides — the default palette is always mid-tone) */
const LIGHT_BAR_COLORS = new Set([
  'var(--color-ink-ghost)',
  'var(--color-rule)',
])

export function resolveColor(span: TimelineSpan): string {
  if (span.status === 'error') return SPAN_STATUS_COLORS.error
  return span.color ?? getWorkerColor(span.label ?? span.id)
}

export function iconColorFor(barColor: string): string {
  return LIGHT_BAR_COLORS.has(barColor) ? 'var(--color-ink)' : 'var(--color-bg)'
}

/** Ring stack for bars: selection (accent) beats hover (ink). */
export function ringFor(
  selected: boolean,
  hovered: boolean,
): string | undefined {
  if (selected) return '0 0 0 1px var(--color-accent)'
  if (hovered) return '0 0 0 1px var(--color-ink)'
  return undefined
}

/** LRM mark: pins the label's edge punctuation to LTR inside the RTL clip. */
const LRM = '\u200E'

const BAR_LABEL_STYLE: CSSProperties = {
  // Leading-ellipsis trick: RTL direction makes text-overflow clip the
  // START of the (LTR) name, so a too-long "chat.respond.stream" reads
  // "…respond.stream" — the tail is the discriminating part of span names.
  direction: 'rtl',
  textAlign: 'left',
}

/**
 * The span name inside a bar. Reveals naturally as the bar grows (flex
 * min-w-0 + overflow clip); when it doesn't fit, the ellipsis sits at the
 * BEGINNING of the name.
 */
export function BarLabel({ text, color }: { text: string; color: string }) {
  return (
    <span
      className="min-w-0 flex-1 overflow-hidden font-mono text-[10px] leading-none lowercase whitespace-nowrap text-ellipsis"
      style={{ ...BAR_LABEL_STYLE, color }}
    >
      {LRM}
      {text}
      {LRM}
    </span>
  )
}
