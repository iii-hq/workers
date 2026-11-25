/**
 * Share lists change without any artifact::* call: a link expires, a starting
 * tunnel becomes ready, a fresh address reaches DNS. Re-read on those clocks:
 * at the next expiry, and every few seconds while a public link still waits
 * for its URL. The timer is cleared on unmount and on every new list.
 */

import { useEffect } from 'react'
import type { ShareView } from './api'

/** How often a waiting public link is re-checked. */
const WAITING_POLL_MS = 5_000
/** Longest single timer, so a far expiry never overflows setTimeout. */
const MAX_TIMER_MS = 6 * 3600 * 1000

export function isWaitingForUrl(share: ShareView): boolean {
  return share.public && !share.public_url && !share.tunnel_error
}

/** The delay until the list should be read again, or `null` for never. */
export function nextRefreshMs(shares: readonly ShareView[], now = Date.now()): number | null {
  if (shares.some(isWaitingForUrl)) return WAITING_POLL_MS
  const soonest = Math.min(...shares.map((s) => s.expires_at * 1000))
  if (!Number.isFinite(soonest)) return null
  return Math.min(Math.max(soonest - now + 1_000, 1_000), MAX_TIMER_MS)
}

export function useShareRefresh(shares: readonly ShareView[] | undefined, refresh: () => void) {
  useEffect(() => {
    if (!shares) return
    const delay = nextRefreshMs(shares)
    if (delay === null) return
    const timer = window.setTimeout(refresh, delay)
    return () => window.clearTimeout(timer)
  }, [shares, refresh])
}
