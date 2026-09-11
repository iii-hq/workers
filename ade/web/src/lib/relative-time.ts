/** Session-manager dates use milliseconds, while transcript events from
 * older workers may still arrive as unix seconds. */
export function timestampMilliseconds(timestamp: number): number {
  return timestamp < 1_000_000_000_000 ? timestamp * 1000 : timestamp
}

/** Short elapsed copy intended to be composed into labels such as
 * "5m ago" and "Active for 5m". */
export function formatElapsed(
  timestamp: number | null | undefined,
  now = Date.now(),
): string | null {
  if (timestamp == null || !Number.isFinite(timestamp)) return null
  const elapsedSeconds = Math.max(
    0,
    Math.floor((now - timestampMilliseconds(timestamp)) / 1000),
  )
  if (elapsedSeconds < 10) return 'just now'
  if (elapsedSeconds < 60) return `${elapsedSeconds}s`
  const elapsedMinutes = Math.floor(elapsedSeconds / 60)
  if (elapsedMinutes < 60) return `${elapsedMinutes}m`
  const elapsedHours = Math.floor(elapsedMinutes / 60)
  if (elapsedHours < 24) return `${elapsedHours}h`
  return `${Math.floor(elapsedHours / 24)}d`
}
