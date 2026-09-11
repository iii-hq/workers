import type * as React from 'react'
import type { LiveAnnouncement } from '@/hooks/use-live-announcer'

interface LiveRegionProps {
  announcement: LiveAnnouncement | null
}

/**
 * Visually hidden ARIA live region. Renders the most recent
 * announcement from `useLiveAnnouncer`, with two parallel nodes
 * (polite + assertive) so screen readers honour each queue
 * separately. Empty when there's nothing to announce.
 *
 * The `key` on each inner span forces React to remount when `seq`
 * advances, which is how SRs notice repeated identical text.
 *
 * `sr-only` is a Tailwind utility from the global config; if absent
 * we fall back to inline styles equivalent to the WCAG-recommended
 * visually-hidden pattern.
 */
const SR_ONLY_STYLE: React.CSSProperties = {
  position: 'absolute',
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: 'hidden',
  clip: 'rect(0, 0, 0, 0)',
  whiteSpace: 'nowrap',
  border: 0,
}

export function LiveRegion({ announcement }: LiveRegionProps) {
  const politeText =
    announcement && announcement.urgency === 'polite' ? announcement.text : ''
  const assertiveText =
    announcement && announcement.urgency === 'assertive'
      ? announcement.text
      : ''
  const seq = announcement?.seq ?? 0
  return (
    <>
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        style={SR_ONLY_STYLE}
      >
        <span key={`p-${seq}`}>{politeText}</span>
      </div>
      <div
        role="alert"
        aria-live="assertive"
        aria-atomic="true"
        style={SR_ONLY_STYLE}
      >
        <span key={`a-${seq}`}>{assertiveText}</span>
      </div>
    </>
  )
}
