import { Chip, StatusDot } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import {
  Activity,
  AlertTriangle,
  Check,
  ChevronDown,
  Circle,
  CircleCheck,
  CircleDot,
  Info,
  Minus,
  X,
} from 'lucide-react'
import { type Mark, marks, PHASES, type Tone } from '../model'
import type { CycleRecord } from '../types'

export const DOT: Record<Tone, 'accent' | 'alert' | 'warn' | 'ok' | 'ink'> = {
  neutral: 'ink',
  accent: 'accent',
  ok: 'ok',
  warn: 'warn',
  alert: 'alert',
}

export const PANEL: Record<Tone, 'info' | 'success' | 'warn' | 'alert'> = {
  neutral: 'info',
  accent: 'info',
  ok: 'success',
  warn: 'warn',
  alert: 'alert',
}

/** The StatusPanel's 16px icon for a tone. */
export function ToneIcon({ tone }: { tone: Tone }) {
  const Icon = { neutral: Info, accent: Activity, ok: CircleCheck, warn: AlertTriangle, alert: X }[tone]
  return <Icon size={16} aria-hidden />
}

export const usd = (value: number) => `$${value.toFixed(2)}`

/** `12h ago`, `just now`. */
export function ago(at: number | string) {
  const relative = formatRelative(at)
  return relative === 'just now' ? relative : `${relative} ago`
}

/** A status word in ink on its tone's tint; a neutral one has no dot. */
export function StatusChip({ tone, label, pulse }: { tone: Tone; label: string; pulse?: boolean }) {
  return (
    <Chip className="eval-ui-chip" data-tone={tone}>
      {tone === 'neutral' ? null : <StatusDot tone={DOT[tone]} pulse={pulse} />} {label}
    </Chip>
  )
}

/** The caret of a `<details>` summary, turned while open. */
export const Caret = () => <ChevronDown size={16} className="eval-ui-caret" aria-hidden />

const MARK_WORD: Record<Mark, string> = {
  done: 'done',
  now: 'running',
  stop: 'stopped',
  fail: 'failed',
  todo: 'not reached',
  off: 'off',
}

/** The four phases as one strip of marks. */
export function Strip({ record }: { record: CycleRecord }) {
  const all = marks(record)
  return (
    <span
      className="eval-ui-strip"
      role="img"
      aria-label={PHASES.map((phase, at) => `${phase} ${MARK_WORD[all[at]]}`).join(', ')}
    >
      {all.map((mark, at) => (
        <span key={PHASES[at]} data-mark={mark} />
      ))}
    </span>
  )
}

/** A step's 16px mark, coloured by its state. */
export function MarkIcon({ mark }: { mark: Mark }) {
  const Icon = { done: Check, now: CircleDot, stop: Minus, fail: X, todo: Circle, off: Minus }[mark]
  return <Icon size={16} className="eval-ui-mark" data-mark={mark} aria-hidden />
}
