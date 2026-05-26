import { useCallback, useState } from 'react'

export interface LiveAnnouncement {
  /** Stable monotonic id; lets React/SR redraw and re-announce identical text. */
  readonly seq: number
  readonly text: string
  readonly urgency: 'polite' | 'assertive'
}

export interface LiveAnnouncerApi {
  announcement: LiveAnnouncement | null
  /** Announce a message politely (queued behind any in-flight speech). */
  announce: (text: string) => void
  /** Announce a message assertively (interrupts in-flight speech). */
  announceAssertive: (text: string) => void
}

/**
 * Backing state for a shared `<LiveRegion>`. The hook returns the
 * latest message + an `announce` function; the component renders the
 * message into a visually hidden `role="status"` / `role="alert"`
 * node so assistive tech picks it up.
 *
 * Each new announcement carries a monotonically increasing `seq` so
 * re-announcing the same text (e.g. "auto-accepted: fs::ls" twice
 * in a row) still triggers an SR update — without it, React would
 * see identical props and skip the DOM patch, swallowing the
 * announcement.
 */
export function useLiveAnnouncer(): LiveAnnouncerApi {
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(
    null,
  )

  const announce = useCallback((text: string) => {
    if (!text) return
    setAnnouncement((prev) => ({
      seq: (prev?.seq ?? 0) + 1,
      text,
      urgency: 'polite',
    }))
  }, [])

  const announceAssertive = useCallback((text: string) => {
    if (!text) return
    setAnnouncement((prev) => ({
      seq: (prev?.seq ?? 0) + 1,
      text,
      urgency: 'assertive',
    }))
  }, [])

  return { announcement, announce, announceAssertive }
}
