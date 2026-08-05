/**
 * Cron expression reading: a plain-language description and, where it can be
 * derived honestly, the next fire time.
 *
 * Both refuse to guess. `describeCron` returns null for anything past the
 * common shapes (ranges, multi-field lists, `L`/`#` extensions) because a
 * wrong translation is worse than the raw expression, and `nextRun` covers
 * only the shapes whose next occurrence follows from the fields alone.
 *
 * The console SPA carries the same `describeCron` for chat rendering
 * (`console/web/src/components/chat/engine/parsers.ts`). Injected UI cannot
 * import across the two projects, and `@iii-dev/console-ui` is deliberately
 * the only versioned contract, so this is a copy on purpose.
 */

const MONTHS = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
]
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

const num = (s: string, max: number): number | null =>
  /^\d+$/.test(s) && Number(s) <= max ? Number(s) : null

const step = (s: string): number | null => {
  const m = /^\*\/(\d+)$/.exec(s)
  return m ? Number(m[1]) : null
}

const pad = (n: number) => String(n).padStart(2, '0')

/** Split into fields, normalizing the 5, 6 and 7-field dialects. */
function fields(expression: string) {
  const f = expression.trim().split(/\s+/)
  if (f.length < 5 || f.length > 7) return null
  const withSeconds = f.length >= 6
  const [min, hour, dom, mon, dow] = withSeconds ? f.slice(1) : f
  return {
    sec: withSeconds ? f[0] : '0',
    min,
    hour,
    dom,
    mon,
    dow,
    year: f.length === 7 ? f[6] : '*',
    withSeconds,
  }
}

/**
 * Humanize the common cron shapes: fixed time, minute steps, single
 * day-of-month/month, weekday lists. `null` means "show the raw expression".
 */
export function describeCron(expression: string): string | null {
  const f = fields(expression)
  if (!f || f.year !== '*') return null

  const secondsWild = f.sec === '*'
  const secNum = num(f.sec, 59)
  if (!secondsWild && secNum == null) return null

  const h = num(f.hour, 23)
  const m = num(f.min, 59)
  let time: string | null = null
  let daily = false
  if (h != null && m != null) {
    time = `at ${pad(h)}:${pad(m)}`
    daily = true
  } else if (f.hour === '*') {
    const minStep = step(f.min)
    if (minStep != null) time = `every ${minStep} min`
    else if (f.min === '*')
      time = f.sec === '*' ? 'every second' : 'every minute'
    else if (m != null) time = `at :${pad(m)} every hour`
    else return null
  } else {
    const hourStep = step(f.hour)
    if (hourStep != null && m != null) time = `every ${hourStep}h at :${pad(m)}`
    else return null
  }
  // Every description except "every second" speaks at minute granularity, so
  // it is only honest when the schedule fires once per matching minute
  // (seconds pinned to 0). `* 0 17 * * *` fires every second DURING 17:00 —
  // saying "at 17:00" would hide sixty firings.
  if (time !== 'every second' && (secondsWild || secNum !== 0)) return null

  const dn = num(f.dom, 31)
  const mn = num(f.mon, 12)
  if (f.dom !== '*' && (dn == null || dn < 1)) return null
  if (f.mon !== '*' && (mn == null || mn < 1)) return null
  let date: string | null = null
  if (dn != null && mn != null) date = `on ${MONTHS[mn - 1]} ${dn}`
  else if (dn != null) date = `on day ${dn} of every month`
  else if (mn != null) date = `in ${MONTHS[mn - 1]}`

  let week: string | null = null
  if (f.dow !== '*' && f.dow !== '?') {
    const names = f.dow.split(',').map((d) => {
      const n = num(d, 7)
      if (n == null) return null
      // Numeric weekday numbering differs by dialect: the seconds-first form
      // is the Rust `cron` crate's (Quartz-style, 1=Sun..7=Sat); classic
      // five-field cron is 0=Sun..6=Sat with 7 also Sunday.
      if (f.withSeconds) return n >= 1 ? WEEKDAYS[n - 1] : null
      return WEEKDAYS[n % 7]
    })
    if (names.some((n) => n == null)) return null
    week = `every ${names.join(', ')}`
  }

  // A day-of-month AND a weekday restriction is OR semantics in cron, subtle
  // enough that the raw expression is the honest rendering.
  if (date && week) return null
  if (week) return daily ? `${week} ${time}` : `${time} ${week}`
  if (date) return `${time} ${date}`
  return daily ? `every day ${time}` : (time as string)
}

/**
 * Next fire time for the unrestricted shapes only: a daily fixed time, a
 * minute step, a fixed minute each hour. Anything with a date or weekday
 * restriction returns null rather than a number the page cannot stand behind.
 */
export function nextCronRun(expression: string, now: Date): Date | null {
  const f = fields(expression)
  if (!f || f.year !== '*') return null
  if (f.dom !== '*' || f.mon !== '*' || (f.dow !== '*' && f.dow !== '?')) {
    return null
  }
  if (num(f.sec, 59) !== 0) return null

  const h = num(f.hour, 23)
  const m = num(f.min, 59)
  const next = new Date(now)
  next.setSeconds(0, 0)

  if (h != null && m != null) {
    next.setHours(h, m)
    if (next <= now) next.setDate(next.getDate() + 1)
    return next
  }
  if (f.hour === '*') {
    const minStep = step(f.min)
    if (minStep != null && minStep > 0) {
      const minute = now.getMinutes()
      const upcoming = Math.floor(minute / minStep) * minStep + minStep
      next.setMinutes(upcoming)
      return next
    }
    if (m != null) {
      next.setMinutes(m)
      if (next <= now) next.setHours(next.getHours() + 1)
      return next
    }
    if (f.min === '*') {
      next.setMinutes(now.getMinutes() + 1)
      return next
    }
  }
  return null
}

/** "in 4 min", "in 2 h 10 min", "in 12 s" — the wait, not a wall-clock time. */
export function untilLabel(target: Date, now: Date): string {
  const seconds = Math.max(
    0,
    Math.round((target.getTime() - now.getTime()) / 1000),
  )
  if (seconds < 60) return `in ${seconds} s`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `in ${minutes} min`
  const hours = Math.floor(minutes / 60)
  const rest = minutes % 60
  return rest ? `in ${hours} h ${rest} min` : `in ${hours} h`
}
