import { Badge, StatusDot } from '@iii-dev/console-ui'
import { Ban, Check, FileCode, TriangleAlert } from 'lucide-react'
import type { GroupStatus, GroupSummary } from '../api'
import { hotFrom, sparklineBars, statusLook } from './present.js'

/**
 * The 6px mark that leads a row or a title. The shared `StatusDot` carries
 * every tone it has — and its pulse, the one sanctioned glow, for a running
 * investigation; a quiet state has no tone there, so it is a ghost dot.
 */
export function Dot({ tone, pulse }: { tone: string; pulse?: boolean }) {
  if (tone === 'alert' || tone === 'accent' || tone === 'ok') {
    return <StatusDot className="sentinel-ui-dot" tone={tone} pulse={pulse} aria-hidden="true" />
  }
  return <span className="sentinel-ui-dot sentinel-ui-dot--quiet" aria-hidden="true" />
}

/** The state, in one word, with the glyph that tells it apart at a glance. */
export function StatusBadge({ status }: { status: GroupStatus }) {
  const look = statusLook(status)
  return (
    <Badge variant={look.badge as 'default' | 'ok' | 'warn' | 'alert' | 'accent'}>
      {look.glyph === 'alert' ? <TriangleAlert size={16} aria-hidden="true" /> : null}
      {look.glyph === 'live' ? <Dot tone="accent" pulse /> : null}
      {look.glyph === 'file' ? <FileCode size={16} aria-hidden="true" /> : null}
      {look.glyph === 'check' ? <Check size={16} aria-hidden="true" /> : null}
      {look.glyph === 'ban' ? <Ban size={16} aria-hidden="true" /> : null}
      {status}
    </Badge>
  )
}

/** Twenty-four bars of CSS, oldest first; the hours since a regression are
    hot. No SVG: the design system asks for none, and this needs none. */
export function Sparkline({ group, now }: { group: GroupSummary; now: number }) {
  const counts = group.sparkline
  const bars = sparklineBars(counts, hotFrom(group, counts.length, now))
  const total = counts.reduce((sum, count) => sum + count, 0)
  return (
    <span
      className="sentinel-ui-sparkline"
      role="img"
      aria-label={`${total} in the last ${counts.length} hours`}
    >
      {bars.map((bar, index) => (
        <i
          key={index}
          style={{ height: `${bar.height}%` }}
          data-hot={bar.hot ? 'true' : undefined}
          data-empty={bar.count === 0 ? 'true' : undefined}
        />
      ))}
    </span>
  )
}
