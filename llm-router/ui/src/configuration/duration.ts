/** Stream/idle timeouts are stored as milliseconds; the form edits minutes. */

export function msToMinutes(ms: number): number {
  return ms / 60_000
}

export function minutesToMs(minutes: number): number {
  return Math.round(minutes * 60_000)
}
