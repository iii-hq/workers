import type { UsageLimit } from '@/lib/backend/claude-accounts'
import { level, percent, type UsageLevel } from '@/lib/claude-accounts-words'
import { cn } from '@/lib/utils'

/** Each tmux colour token's stroke, spelled out so Tailwind sees every class (AccountDot's fills). */
const ARC: Record<string, string> = {
  blue: 'stroke-account-blue',
  green: 'stroke-account-green',
  violet: 'stroke-account-violet',
  orange: 'stroke-account-orange',
  pink: 'stroke-account-pink',
}

/** The track: neutral, amber from 80 % (`high`), red at 100 % (`full`). */
const TRACK: Record<UsageLevel, string> = {
  normal: 'stroke-border',
  high: 'stroke-warn/40',
  full: 'stroke-alert/40',
}

/** The wide surfaces' geometry (D178): a 24 px ring, radius 9, stroke 3.5. */
const BOX = 24
const R = 9
const STROKE = 3.5

/**
 * One Claude Code account's week as a ring (D178): the arc in the
 * account's colour from 12 o'clock, clockwise, round caps; closed at 100 %.
 * Without a week window (no reading, signed out) only the track shows.
 * Decorative: the footer's button carries the words.
 */
export function UsageRing({
  week,
  color,
  className,
}: {
  week: UsageLimit | null | undefined
  color: string
  className?: string
}) {
  const c = BOX / 2
  const pct = week ? percent(week.usedPercent) : 0
  const tone = level(week)
  const length = 2 * Math.PI * R
  const arc = ARC[color] ?? 'stroke-account-gray'
  return (
    <svg
      aria-hidden="true"
      width={BOX}
      height={BOX}
      viewBox={`0 0 ${BOX} ${BOX}`}
      data-usage-ring={week ? pct : 'none'}
      data-usage-level={tone}
      className={cn('shrink-0', className)}
    >
      <circle
        cx={c}
        cy={c}
        r={R}
        fill="none"
        strokeWidth={STROKE}
        className={TRACK[tone]}
      />
      {pct >= 100 ? (
        <circle
          cx={c}
          cy={c}
          r={R}
          fill="none"
          strokeWidth={STROKE}
          className={arc}
        />
      ) : pct > 0 ? (
        <circle
          cx={c}
          cy={c}
          r={R}
          fill="none"
          strokeWidth={STROKE}
          strokeLinecap="round"
          strokeDasharray={`${((length * pct) / 100).toFixed(2)} ${length.toFixed(2)}`}
          transform={`rotate(-90 ${c} ${c})`}
          className={arc}
        />
      ) : null}
    </svg>
  )
}
