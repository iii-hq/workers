/**
 * Pure formatting helpers for the browser UI. Ported from the console's
 * sandbox `format.ts` — only the pieces the session rail needs.
 */

/** Unix seconds → short relative time ("3m ago", "2d ago"). */
export function formatMtime(unixSecs: number, now = Date.now()): string {
  if (!unixSecs || unixSecs <= 0) return '—'
  const deltaSecs = Math.max(0, Math.floor(now / 1000 - unixSecs))
  if (deltaSecs < 60) return deltaSecs <= 1 ? 'just now' : `${deltaSecs}s ago`
  const mins = Math.floor(deltaSecs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days}d ago`
  const months = Math.floor(days / 30)
  if (months < 12) return `${months}mo ago`
  return `${Math.floor(months / 12)}y ago`
}
