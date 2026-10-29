/**
 * The strings and named values the app shows (the host formats nothing,
 * design §5.5 of the platform): relative times, counts, column and priority
 * names, the status glyphs of the console UI, the named hues. Pure: no `iii`.
 */

import type { Hue } from '@iii-dev/native-ui'
import type { Column } from './api'

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** `now`, `5m`, `3h`, `2d`, then the date (`Oct 3`, `Oct 3, 2025` in another year). */
export function ago(iso: string, now: number): string {
  const at = Date.parse(iso)
  if (Number.isNaN(at)) return ''
  const d = now - at
  if (d < MINUTE) return 'now'
  if (d < HOUR) return `${Math.floor(d / MINUTE)}m`
  if (d < DAY) return `${Math.floor(d / HOUR)}h`
  if (d < 7 * DAY) return `${Math.floor(d / DAY)}d`
  const date = new Date(at)
  const day = `${MONTHS[date.getMonth()]} ${date.getDate()}`
  return date.getFullYear() === new Date(now).getFullYear() ? day : `${day}, ${date.getFullYear()}`
}

/** `5m ago`; `now` stays `just now`. */
export function agoLong(iso: string, now: number): string {
  const short = ago(iso, now)
  if (!short) return ''
  if (short === 'now') return 'just now'
  return /\d[mhd]$/.test(short) ? `${short} ago` : short
}

/** A lane count: `999`, then `1.2k`, `12k`. */
export function count(n: number): string {
  if (n < 1000) return String(n)
  const k = n / 1000
  return `${k < 10 ? k.toFixed(1).replace(/\.0$/, '') : Math.floor(k)}k`
}

/** `1 ticket`, `3 tickets`. */
export const plural = (n: number, one: string) => `${n} ${n === 1 ? one : `${one}s`}`

/** `2026-10-30` → `Oct 30`; anything else is no date. Read as text, so no timezone moves the day. */
export function shortDate(day: string | null | undefined): string | null {
  const match = day?.match(/^(\d{4})-(\d{2})-(\d{2})/)
  const month = match ? MONTHS[Number(match[2]) - 1] : undefined
  return match && month ? `${month} ${Number(match[3])}` : null
}

export function statusLabel(columns: readonly Column[], status: string): string {
  return columns.find((c) => c.id === status)?.label ?? status
}

/** `high` → `High`. */
export function priorityLabel(priority: string): string {
  return priority ? priority.charAt(0).toUpperCase() + priority.slice(1) : 'No priority'
}

/** 0 = the most urgent configured priority (the last), 1 the next; -1 for the others and unknown ones. */
export function priorityRank(priority: string, priorities: readonly string[]): number {
  const i = priorities.indexOf(priority)
  if (i < 0) return -1
  const fromTop = priorities.length - 1 - i
  return fromTop <= 1 && i > 0 ? fromTop : -1
}

export type Step = 'unknown' | 'start' | 'ready' | 'progress' | 'review' | 'done' | 'canceled'

/** The step a column is on the way to done (the console UI's `statusStep`). */
export function statusStep(columns: readonly Column[], status: string, done: string, canceled: string | null): Step {
  if (status === done) return 'done'
  if (status === canceled) return 'canceled'
  const steps = columns.filter((c) => c.id !== done && c.id !== canceled)
  const i = steps.findIndex((c) => c.id === status)
  const n = steps.length
  if (i < 0) return 'unknown'
  if (i === 0) return 'start'
  if (n >= 3 && i === n - 1) return 'review'
  if (n >= 3 && i === 1) return 'ready'
  return 'progress'
}

/** The glyph of each step: dashed start, circle, half, pie in review, check, cross. */
export const STEP_ICON: Record<Step, string> = {
  unknown: 'circle',
  start: 'circle-dashed',
  ready: 'circle',
  progress: 'contrast',
  review: 'chart-pie',
  done: 'circle-check',
  canceled: 'circle-x',
}

/** The chip tone of a step. */
export const STEP_TONE: Record<Step, 'neutral' | 'accent' | 'success' | 'warning'> = {
  unknown: 'neutral',
  start: 'neutral',
  ready: 'neutral',
  progress: 'accent',
  review: 'warning',
  done: 'success',
  canceled: 'neutral',
}

/** The named colors of the catalogue (`hue`): the project and agent colors. */
export const HUES = ['blue', 'purple', 'teal', 'green', 'amber', 'rose'] as const

/** A project's or agent's color as a hue; anything else is `none`. */
export function hueOf(color: string | null | undefined): Hue {
  return color && (HUES as readonly string[]).includes(color) ? (color as Hue) : 'none'
}

/**
 * Pill icons another worker names: the ones the app draws pass, any other
 * becomes `tag` (an unknown name draws a neutral square).
 */
const PILL_ICONS = new Set([
  'calendar',
  'clock',
  'timer',
  'flag',
  'bell',
  'bell-ring',
  'tag',
  'star',
  'zap',
  'flame',
  'bug',
  'link',
  'globe',
  'mail',
  'user',
  'users',
  'bot',
  'rocket',
  'target',
  'triangle-alert',
  'info',
  'check',
  'circle-check',
  'git-branch',
  'git-pull-request',
  'git-merge',
  'message-circle',
  'lock',
  'shield',
  'heart',
])

export const pillIcon = (name: string): string => (PILL_ICONS.has(name.trim()) ? name.trim() : 'tag')

/** A ticket's one-line share text. */
export const shareText = (key: string, title: string) => (title ? `${key}: ${title}` : key)
