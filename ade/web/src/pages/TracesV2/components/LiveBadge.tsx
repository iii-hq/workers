/**
 * The traces page's one "is this still moving" mark: a quiet pill with a
 * small dot. Live is the warn amber — the page's "still running" hue,
 * shared with the pending bars' sweep and the timeline's now-line — with
 * a soft expanding ring (`live-dot`); at rest the dot is the muted ink
 * and the ring is gone. The same pill heads the Activity strip, the trace
 * card, and a paused feed, so the state reads the same everywhere.
 */

import type { LucideIcon } from 'lucide-react'
import { cn } from '@/lib/utils'

interface LiveBadgeProps {
  /** Something is still running: amber dot, ring, the `live` label. */
  live: boolean
  /** What to say while live (default "Live"). */
  liveLabel?: string
  /** What to say at rest (default "Complete"). */
  restLabel?: string
  /** A glyph in place of the dot at rest (the paused strip's pause mark). */
  restIcon?: LucideIcon
  /** `warn` tints the rest state amber (paused), the default is neutral. */
  restTone?: 'neutral' | 'warn'
  className?: string
}

export function LiveBadge({
  live,
  liveLabel = 'Live',
  restLabel = 'Complete',
  restIcon: RestIcon,
  restTone = 'neutral',
  className,
}: LiveBadgeProps) {
  return (
    <span
      role="status"
      className={cn(
        'inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full border px-2 text-xs font-medium whitespace-nowrap',
        live
          ? 'border-warn/25 bg-warn-muted text-warn-strong'
          : restTone === 'warn'
            ? 'border-warn/25 bg-warn-muted text-warn-strong'
            : 'border-border bg-muted/60 text-muted-foreground',
        className,
      )}
    >
      {live ? (
        <span
          aria-hidden
          className="size-1.5 rounded-full bg-warn live-dot [--live-dot-color:var(--color-warn)]"
        />
      ) : RestIcon ? (
        <RestIcon aria-hidden className="size-4" />
      ) : (
        <span
          aria-hidden
          className="size-1.5 rounded-full bg-muted-foreground/60"
        />
      )}
      {live ? liveLabel : restLabel}
    </span>
  )
}
