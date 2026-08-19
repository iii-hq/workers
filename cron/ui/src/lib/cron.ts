const pad = (value: number) => String(value).padStart(2, '0')

interface CronFields {
  sec: string
  min: string
  hour: string
  dom: string
  month: string
  weekday: string
  year: string
}

function fields(expression: string): CronFields | null {
  const values = expression.trim().split(/\s+/)
  if (values.length !== 6 && values.length !== 7) return null
  return {
    sec: values[0],
    min: values[1],
    hour: values[2],
    dom: values[3],
    month: values[4],
    weekday: values[5],
    year: values[6] ?? '*',
  }
}

function numberIn(value: string, minimum: number, maximum: number): number | null {
  if (!/^\d+$/.test(value)) return null
  const parsed = Number(value)
  return parsed >= minimum && parsed <= maximum ? parsed : null
}

function step(value: string, maximum: number): number | null {
  const match = /^\*\/(\d+)$/.exec(value)
  const parsed = match ? Number(match[1]) : 0
  return parsed > 0 && parsed <= maximum ? parsed : null
}

interface FieldRules {
  label: string
  minimum: number
  maximum: number
  allowAny?: boolean
  names?: Record<string, number>
}

const MONTH_NAMES: Record<string, number> = {
  jan: 1,
  january: 1,
  feb: 2,
  february: 2,
  mar: 3,
  march: 3,
  apr: 4,
  april: 4,
  may: 5,
  jun: 6,
  june: 6,
  jul: 7,
  july: 7,
  aug: 8,
  august: 8,
  sep: 9,
  september: 9,
  oct: 10,
  october: 10,
  nov: 11,
  november: 11,
  dec: 12,
  december: 12,
}

const WEEKDAY_NAMES: Record<string, number> = {
  sun: 1,
  sunday: 1,
  mon: 2,
  monday: 2,
  tue: 3,
  tues: 3,
  tuesday: 3,
  wed: 4,
  wednesday: 4,
  thu: 5,
  thurs: 5,
  thursday: 5,
  fri: 6,
  friday: 6,
  sat: 7,
  saturday: 7,
}

function ordinal(value: string, rules: FieldRules): number | null {
  const numeric = numberIn(value, rules.minimum, rules.maximum)
  if (numeric !== null) return numeric
  return rules.names?.[value.toLowerCase()] ?? null
}

function validSpecifier(value: string, rules: FieldRules, stepped: boolean): boolean {
  if (value === '*' || (rules.allowAny && value === '?')) return true
  const range = value.split('-')
  if (range.length === 2) {
    const start = ordinal(range[0], rules)
    const end = ordinal(range[1], rules)
    return start !== null && end !== null && start <= end
  }
  if (range.length !== 1) return false
  if (stepped && /^[a-z]+$/i.test(value)) return false
  return ordinal(value, rules) !== null
}

function validField(value: string, rules: FieldRules): boolean {
  return value.split(',').every((segment) => {
    const period = segment.split('/')
    if (period.length > 2 || !period[0]) return false
    if (period.length === 2 && (!/^\d+$/.test(period[1]) || Number(period[1]) === 0)) {
      return false
    }
    return validSpecifier(period[0], rules, period.length === 2)
  })
}

export function validateCron(expression: string): string | null {
  if (!expression.trim()) return 'Enter a cron expression.'
  const parsed = fields(expression)
  if (!parsed) return 'Use six fields (sec min hour day month weekday), with an optional year.'
  const checks: Array<[string, FieldRules]> = [
    [parsed.sec, { label: 'Seconds', minimum: 0, maximum: 59 }],
    [parsed.min, { label: 'Minutes', minimum: 0, maximum: 59 }],
    [parsed.hour, { label: 'Hours', minimum: 0, maximum: 23 }],
    [parsed.dom, { label: 'Day of month', minimum: 1, maximum: 31, allowAny: true }],
    [parsed.month, { label: 'Month', minimum: 1, maximum: 12, names: MONTH_NAMES }],
    [parsed.weekday, { label: 'Weekday', minimum: 1, maximum: 7, allowAny: true, names: WEEKDAY_NAMES }],
    [parsed.year, { label: 'Year', minimum: 1970, maximum: 2100 }],
  ]
  for (const [value, rules] of checks) {
    if (!validField(value, rules)) return `${rules.label} field is invalid.`
  }
  return null
}

export function describeCron(expression: string): string | null {
  const parsed = fields(expression)
  if (!parsed || parsed.year !== '*' || numberIn(parsed.sec, 0, 59) !== 0) return null
  if (parsed.dom !== '*' || parsed.month !== '*') return null

  const hour = numberIn(parsed.hour, 0, 23)
  const minute = numberIn(parsed.min, 0, 59)
  const minuteStep = parsed.hour === '*' ? step(parsed.min, 59) : null
  let time: string | null = null
  if (hour !== null && minute !== null) time = `${pad(hour)}:${pad(minute)} UTC`
  else if (minuteStep !== null) time = `every ${minuteStep} minutes`
  else if (parsed.hour === '*' && minute !== null) time = `hourly at :${pad(minute)} UTC`
  if (!time) return null

  if (parsed.weekday === '*' || parsed.weekday === '?') {
    return hour !== null ? `every day at ${time}` : time
  }
  const names = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
  const days = parsed.weekday.split(',').map((value) => {
    const numeric = numberIn(value, 1, 7)
    return numeric !== null && numeric >= 1 ? names[numeric - 1] : null
  })
  if (days.some((value) => value === null) || hour === null) return null
  return `${days.join(', ')} at ${time}`
}

export function nextCronRun(expression: string, now: Date): Date | null {
  const parsed = fields(expression)
  if (!parsed || parsed.year !== '*' || numberIn(parsed.sec, 0, 59) !== 0) return null
  if (parsed.dom !== '*' || parsed.month !== '*' || !['*', '?'].includes(parsed.weekday)) {
    return null
  }
  const hour = numberIn(parsed.hour, 0, 23)
  const minute = numberIn(parsed.min, 0, 59)
  const next = new Date(now)
  next.setUTCSeconds(0, 0)

  if (hour !== null && minute !== null) {
    next.setUTCHours(hour, minute)
    if (next <= now) next.setUTCDate(next.getUTCDate() + 1)
    return next
  }
  if (parsed.hour === '*') {
    const interval = step(parsed.min, 59)
    if (interval !== null) {
      const current = now.getUTCMinutes()
      const nextMinute = Math.floor(current / interval) * interval + interval
      if (nextMinute > 59) {
        next.setUTCMinutes(0)
        next.setUTCHours(next.getUTCHours() + 1)
      } else {
        next.setUTCMinutes(nextMinute)
      }
      return next
    }
    if (minute !== null) {
      next.setUTCMinutes(minute)
      if (next <= now) next.setUTCHours(next.getUTCHours() + 1)
      return next
    }
  }
  return null
}

export function untilLabel(target: Date, now: Date): string {
  const seconds = Math.max(0, Math.round((target.getTime() - now.getTime()) / 1000))
  if (seconds < 60) return `in ${seconds}s`
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `in ${minutes}m`
  const hours = Math.floor(minutes / 60)
  const remainder = minutes % 60
  return remainder ? `in ${hours}h ${remainder}m` : `in ${hours}h`
}
