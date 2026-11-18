import { useEffect, useState } from 'react'
import { onTabLimitNotice, tabLimitReachedTitle } from '@/lib/tab-limit'

/** How long the notice stays up after the last refused ⌃T. */
export const TAB_LIMIT_NOTICE_MS = 4000

/**
 * The answer to a ⌃T (or the palette's New workspace) at the tab limit: a
 * short floating note under the menu bar that names the limit and the way
 * out, announced politely to screen readers. Pointer users get the same
 * words on the disabled `+`.
 */
export function TabLimitNotice({ limit }: { limit: number }) {
  const [shown, setShown] = useState(0)
  useEffect(() => onTabLimitNotice(() => setShown((count) => count + 1)), [])
  useEffect(() => {
    if (shown === 0) return
    const timer = window.setTimeout(() => setShown(0), TAB_LIMIT_NOTICE_MS)
    return () => window.clearTimeout(timer)
  }, [shown])
  return (
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-none fixed top-14 left-1/2 z-50 w-max max-w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2"
    >
      {shown > 0 ? (
        <p
          data-tab-limit-notice
          className="rounded-xl bg-popover px-3.5 py-2.5 font-inter text-sm text-popover-foreground shadow-md ring-1 ring-popover-edge"
        >
          {tabLimitReachedTitle(limit)}
        </p>
      ) : null}
    </div>
  )
}
