import { cn } from '@/lib/utils'

interface RingGaugeProps {
  /** Used share, 0–1; anything outside is clamped. */
  ratio: number
  /** Outer diameter in px: 16 beside text, larger in a hero. */
  size?: number
  /** Stroke width in px. */
  stroke?: number
  /** The arc takes `currentColor`: tone it with a text colour utility. */
  className?: string
}

/**
 * A ring gauge: a track circle with an arc for the used share, starting at
 * twelve o'clock. The arc is `currentColor`, so the caller tones it the
 * way it tones text; the track is the same surface tint as every empty bar.
 * Mirrors the harness worker's chip (harness/ui/src/context-chip/icons.tsx)
 * so the built-in meter and the injected one read as one control.
 */
export function RingGauge({
  ratio,
  size = 16,
  stroke = 2.5,
  className,
}: RingGaugeProps) {
  const share = Number.isFinite(ratio) ? Math.min(1, Math.max(0, ratio)) : 0
  const centre = size / 2
  const radius = (size - stroke) / 2
  const circumference = 2 * Math.PI * radius
  const arc = share * circumference
  return (
    <svg
      className={cn('block shrink-0', className)}
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
      {/* A zero-length dash with a round cap still paints a dot: the arc
          only exists once there is something to show. */}
      {arc > 0 ? (
        <circle
          cx={centre}
          cy={centre}
          r={radius}
          fill="none"
          stroke="currentColor"
          strokeWidth={stroke}
          strokeLinecap={share >= 1 ? 'butt' : 'round'}
          strokeDasharray={`${arc} ${circumference - arc}`}
          transform={`rotate(-90 ${centre} ${centre})`}
          className="transition-[stroke-dasharray] duration-[var(--motion-duration-panel)] ease-[var(--motion-ease-standard)]"
        />
      ) : null}
    </svg>
  )
}
