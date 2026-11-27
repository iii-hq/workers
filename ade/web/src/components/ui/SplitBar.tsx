import { cn } from '@/lib/utils'

/** A segment's look: a project hue, "no project", or outside the Chat. */
export type SplitBarTone =
  | 'blue'
  | 'purple'
  | 'teal'
  | 'green'
  | 'amber'
  | 'rose'
  | 'no-project'
  | 'outside'

export interface SplitBarSegment {
  key: string
  /** Share of the whole bar, 0 to 100. */
  percent: number
  tone: SplitBarTone
}

/** Each tone's fill, spelled out so Tailwind sees every class. */
const FILL: Record<SplitBarTone, string> = {
  blue: 'fill-glyph-blue',
  purple: 'fill-glyph-purple',
  teal: 'fill-glyph-teal',
  green: 'fill-glyph-green',
  amber: 'fill-glyph-amber',
  rose: 'fill-glyph-rose',
  'no-project': 'fill-account-no-project',
  outside: 'fill-account-outside',
}

/** The gap between two segments, in the bar's 0–100 units. */
const GAP = 0.6

/**
 * A share of the week split into segments (chat-projects §4.3.1, D28): each
 * project in its hue, "no project", and "Outside the Chat" in the neutral
 * grey, on the muted track; what is left of the bar is the unused week.
 *
 * Widths are SVG attributes in a 0–100 view box, so the bar has no inline
 * style (`lint:design`). Silent unless given a label: the legend beside it
 * says the numbers.
 */
export function SplitBar({
  segments,
  label,
  className,
}: {
  segments: readonly SplitBarSegment[]
  /** The accessible name ("This week by project: acme-web 33%, …"). */
  label?: string
  className?: string
}) {
  const shown = segments.filter((segment) => segment.percent > 0)
  let x = 0
  const rects = shown.map((segment, index) => {
    const start = x
    const width = Math.max(0, Math.min(100 - start, segment.percent))
    x = start + width
    // A gap after every segment but the last one drawn.
    const gap = index < shown.length - 1 ? Math.min(GAP, width / 2) : 0
    return { segment, start, width: Math.max(0, width - gap) }
  })
  return (
    <svg
      data-slot="split-bar"
      viewBox="0 0 100 6"
      preserveAspectRatio="none"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={cn(
        'block h-1.5 w-full overflow-hidden rounded-full',
        className,
      )}
    >
      <rect x="0" y="0" width="100" height="6" className="fill-muted" />
      {rects.map(({ segment, start, width }) => (
        <rect
          key={segment.key}
          data-split-segment={segment.tone}
          x={start}
          y="0"
          width={width}
          height="6"
          className={FILL[segment.tone]}
        />
      ))}
    </svg>
  )
}
