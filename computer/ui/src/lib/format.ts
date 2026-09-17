/** Pure formatting helpers for the computer page. */

import { formatRelative } from '@iii-dev/console-ui/format'

/** Epoch millis → short relative time; `—` when the worker has no timestamp. */
export function formatAge(unixMs: number): string {
  return unixMs > 0 ? formatRelative(unixMs) : '—'
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
