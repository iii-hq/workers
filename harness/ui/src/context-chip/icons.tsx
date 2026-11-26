/**
 * The ring gauge the context chip and its breakdown draw: a track circle with
 * an arc for the used share, starting at twelve o'clock. Lucide has no
 * proportional ring, so this is the one inline SVG the worker keeps, in the
 * `icons.tsx` file the worker-ui lint sets aside for it. Colour comes from
 * the caller (the usage tone); the track is a tint of the surrounding text so it
 * reads on any surface.
 */

import type { CSSProperties } from 'react'

export interface RingGaugeProps {
  /** Used share, 0–1; anything outside is clamped. */
  ratio: number
  /** Outer diameter in px: 16 beside text, larger in the hero. */
  size?: number
  /** Stroke width in px. */
  stroke?: number
  /** Arc colour; the track is a tint of `currentColor`. */
  color: string
  /** A threshold, 0–1 from twelve o'clock: a short amber mark across the
      track, where auto-compact fires (chat-projects §3.3.3). */
  tick?: number | null
  className?: string
}

/* The empty share, as a tint of the text beside it: `--color-surface-active`
   vanished against the header it sits in, so an unused window drew nothing. */
const TRACK: CSSProperties = {
  stroke: 'color-mix(in oklab, currentColor 32%, transparent)',
}

/** The tick's two ends: across the track, from just inside it to the edge. */
function tickLine(tick: number | null | undefined, centre: number, radius: number, stroke: number) {
  if (tick == null || !Number.isFinite(tick)) return null
  const angle = Math.min(1, Math.max(0, tick)) * 2 * Math.PI - Math.PI / 2
  const inner = Math.max(0, radius - stroke / 2 - 0.75)
  const round = (n: number) => Math.round(n * 100) / 100
  return {
    x1: round(centre + inner * Math.cos(angle)),
    y1: round(centre + inner * Math.sin(angle)),
    x2: round(centre + centre * Math.cos(angle)),
    y2: round(centre + centre * Math.sin(angle)),
  }
}

export function RingGauge({ ratio, size = 16, stroke = 2.5, color, tick, className }: RingGaugeProps) {
  const share = Number.isFinite(ratio) ? Math.min(1, Math.max(0, ratio)) : 0
  const centre = size / 2
  const radius = (size - stroke) / 2
  const circumference = 2 * Math.PI * radius
  const arc = share * circumference
  const mark = tickLine(tick, centre, radius, stroke)
  const motion: CSSProperties = {
    transition: 'stroke-dasharray var(--motion-duration-panel) var(--motion-ease-standard)',
  }
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      aria-hidden="true"
      focusable="false"
    >
      <circle
        cx={centre}
        cy={centre}
        r={radius}
        fill="none"
        strokeWidth={stroke}
        style={TRACK}
      />
      {/* A zero-length dash with a round cap still paints a dot: the arc only
          exists once there is something to show. */}
      {arc > 0 ? (
        <circle
          cx={centre}
          cy={centre}
          r={radius}
          fill="none"
          stroke={color}
          strokeWidth={stroke}
          strokeLinecap={share >= 1 ? 'butt' : 'round'}
          strokeDasharray={`${arc} ${circumference - arc}`}
          transform={`rotate(-90 ${centre} ${centre})`}
          style={motion}
        />
      ) : null}
      {mark ? (
        <line
          data-ring-tick=""
          {...mark}
          stroke="var(--color-warn)"
          strokeWidth={1.5}
          strokeLinecap="round"
        />
      ) : null}
    </svg>
  )
}
