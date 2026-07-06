/**
 * Shared visual vocabulary for timeline spans — used by both the live
 * Timeline strip and the static TraceTimeline so a span reads identically
 * wherever it renders: same kind icon, same chromatic service color, same
 * in-bar label, same hover (ink) / selection (accent) ring recipe.
 */

import { Flame, Sparkle, SquareFunction, Zap } from 'lucide-react'
import type { CSSProperties } from 'react'
import { getServiceColor, SPAN_STATUS_COLORS } from '../../lib/traceColors'
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

export const CHIP_SIZE = 16

/** ink shades that are too light to carry a bg-colored icon (legacy
 * `span.color` overrides — the default palette is always mid-tone) */
const LIGHT_BAR_COLORS = new Set([
  'var(--color-ink-ghost)',
  'var(--color-rule)',
])

export function resolveColor(span: TimelineSpan): string {
  if (span.status === 'error') return SPAN_STATUS_COLORS.error
  return span.color ?? getServiceColor(span.label ?? span.id)
}

export function iconColorFor(barColor: string): string {
  return LIGHT_BAR_COLORS.has(barColor) ? 'var(--color-ink)' : 'var(--color-bg)'
}

/**
 * Ring stack for bars and chips: selection (accent) beats hover (ink).
 * `base` is the chip's permanent 1.5px paper gap ring; bars pass none.
 */
export function ringFor(
  selected: boolean,
  hovered: boolean,
  base?: string,
): string | undefined {
  if (selected) {
    return base
      ? '0 0 0 1.5px var(--color-accent)'
      : '0 0 0 2px var(--color-accent)'
  }
  if (hovered) {
    return base ? '0 0 0 1.5px var(--color-ink)' : '0 0 0 1px var(--color-ink)'
  }
  return base
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
