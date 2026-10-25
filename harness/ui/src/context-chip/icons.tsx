/**
 * The ring gauge the context chip and its breakdown draw: a track circle with
 * an arc for the used share, starting at twelve o'clock. Lucide has no
 * proportional ring, so this is the one inline SVG the worker keeps, in the
 * `icons.tsx` file the worker-ui lint sets aside for it. Colour comes from
 * the caller (the usage tone); the track is the same surface tint as every
 * other empty bar in the breakdown.
 */

import type { CSSProperties } from 'react'

export interface RingGaugeProps {
  /** Used share, 0–1; anything outside is clamped. */
  ratio: number
  /** Outer diameter in px: 16 beside text, larger in the hero. */
  size?: number
  /** Stroke width in px. */
  stroke?: number
  /** Arc colour; the track is always `--color-surface-active`. */
  color: string
  className?: string
}

export function RingGauge({ ratio, size = 16, stroke = 2.5, color, className }: RingGaugeProps) {
  const share = Number.isFinite(ratio) ? Math.min(1, Math.max(0, ratio)) : 0
  const centre = size / 2
  const radius = (size - stroke) / 2
  const circumference = 2 * Math.PI * radius
  const arc = share * circumference
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
        stroke="var(--color-surface-active)"
        strokeWidth={stroke}
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
    </svg>
  )
}
