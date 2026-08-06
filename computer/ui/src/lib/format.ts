/** Pure formatting helpers for the computer page. */

/** Epoch millis → short relative time ("3m ago"). */
export function formatAge(unixMs: number, now = Date.now()): string {
  if (!unixMs || unixMs <= 0) return '—'
  const secs = Math.max(0, Math.floor((now - unixMs) / 1000))
  if (secs < 60) return secs <= 1 ? 'just now' : `${secs}s ago`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  return `${Math.floor(hours / 24)}d ago`
}

/** `native` stays as-is; a url is trimmed to host:port for the rail. */
export function shortEndpoint(endpoint: string): string {
  if (!endpoint || endpoint === 'native') return 'native'
  try {
    const url = new URL(endpoint)
    return url.host || endpoint
  } catch {
    return endpoint
  }
}
