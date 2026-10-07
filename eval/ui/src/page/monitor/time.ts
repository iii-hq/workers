// How the monitor writes a moment and a span: local clock times, the day a row belongs to, and how long ago.
// Pure and shared by the list, the notices and the settings.

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

const pad = (value: number) => String(value).padStart(2, '0')

/** `19:41`, local time. */
export function clock(ms: number): string {
  const date = new Date(ms)
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** The local calendar day of `ms`, as a whole number of days. */
function dayOf(ms: number): number {
  const date = new Date(ms)
  return Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY
}

/** `Today 13:21`, `Yesterday 22:45`, then `02 Oct 22:45`: the day is part of the time. */
export function dayClock(ms: number, now: number): string {
  const days = dayOf(now) - dayOf(ms)
  if (days === 0) return `Today ${clock(ms)}`
  if (days === 1) return `Yesterday ${clock(ms)}`
  const date = new Date(ms)
  const month = date.toLocaleString('en-US', { month: 'short' })
  return `${pad(date.getDate())} ${month} ${clock(ms)}`
}

/** `52 s`, `3m 55s`, `1h 04m`: the span of an analysis. */
export function span(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000))
  if (total < 60) return `${total} s`
  const minutes = Math.floor(total / 60)
  if (minutes < 60) return `${minutes}m ${pad(total % 60)}s`
  return `${Math.floor(minutes / 60)}h ${pad(minutes % 60)}m`
}

/** `just now`, `2 min ago`, `3 h ago`, `2 days ago`. */
export function ago(at: number, now: number): string {
  const elapsed = Math.max(0, now - at)
  if (elapsed < MINUTE) return 'just now'
  if (elapsed < HOUR) return `${Math.floor(elapsed / MINUTE)} min ago`
  if (elapsed < DAY) return `${Math.floor(elapsed / HOUR)} h ago`
  const days = Math.floor(elapsed / DAY)
  return `${days} ${days === 1 ? 'day' : 'days'} ago`
}

/** `2–6 min`, `45–90 s`: a range of spans; one figure when both ends round alike. */
export function spanRange(low: number, high: number): string {
  const unit = high < MINUTE ? 1000 : MINUTE
  const [from, to] = [low, high].map((ms) => Math.max(1, Math.round(ms / unit)))
  return `${from === to ? from : `${from}–${to}`} ${unit === 1000 ? 's' : 'min'}`
}
