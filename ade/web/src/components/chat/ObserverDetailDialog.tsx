import { formatRelative } from '@iii-dev/console-ui/format'
import { Check, ChevronRight, Copy } from 'lucide-react'
import { type CSSProperties, type ReactNode, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { Hint } from '@/components/ui/Hint'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import { StatusDot } from '@/components/ui/StatusDot'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { copyTextToClipboard } from '@/lib/clipboard'
import { activityIdPrefix } from '@/lib/sessions/stored-records'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { SystemMessage } from '@/types/chat'

/**
 * What a fire delivers, from the row's data alone: a wake into this chat, or
 * a plain function call. No observed function or target is special-cased —
 * a function that does not exist yet renders exactly like the known ones.
 */
export function deliveryLabel(observer: SessionObserverInfo): string {
  return observer.delivery.kind === 'call'
    ? `calls ${observer.delivery.functionId}`
    : 'notifies this chat'
}

/**
 * The row's lifecycle text, from structured lifecycle data alone. Historical
 * `retired` records do not say how the observer ended, so keep their copy
 * deliberately neutral instead of presenting every retirement as a removal.
 */
export function lifecycleNote(observer: SessionObserverInfo): string | null {
  if (observer.fired) {
    switch (observer.retirementReason) {
      case 'once_consumed':
        return 'once · consumed automatically'
      case 'max_fires':
        return 'fire limit reached'
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'removed'
      case 'invalidated':
        return 'invalidated'
      case 'exhausted':
        return 'exhausted'
    }

    // Records normally repeat these lifecycle events as both outcome and
    // retirement reason. Tolerate a partially enriched record while
    // preserving the same distinct labels.
    switch (observer.outcome) {
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'removed'
      case 'invalidated':
        return 'invalidated'
      default:
        return 'retired'
    }
  }
  const parts: string[] = []
  if (observer.once) parts.push('once')
  if ((observer.fires ?? 0) > 0)
    parts.push(`${observer.fires} fire${observer.fires === 1 ? '' : 's'}`)
  if (observer.maxFires !== undefined) parts.push(`max ${observer.maxFires}`)
  if (observer.expiresAt !== undefined)
    parts.push(`until ${new Date(observer.expiresAt).toLocaleString()}`)
  return parts.length > 0 ? parts.join(' · ') : null
}

/** `every 5 seconds` from a cron step field; null when it is not a step. */
function every(field: string, unit: string): string | null {
  if (field === '*') return `every ${unit}`
  const step = /^\*\/(\d+)$/.exec(field)
  if (!step) return null
  const n = Number(step[1])
  return n === 1 ? `every ${unit}` : `every ${n} ${unit}s`
}

/**
 * A cron expression in words, for the common shapes an agent writes (the
 * cron worker's 7 fields: sec min hour day month weekday year; 5 and 6 are
 * read the same way). Anything else returns null and shows as written.
 */
export function cronInWords(expression: string): string | null {
  const parts = expression.trim().split(/\s+/)
  if (parts.length < 5 || parts.length > 7) return null
  const [sec, min, hour, ...rest] = parts.length === 5 ? ['0', ...parts] : parts
  if (!rest.slice(0, 3).every((field) => field === '*' || field === '?'))
    return null
  const words =
    (min === '*' && hour === '*' ? every(sec, 'second') : null) ??
    (sec === '0' && hour === '*' && min !== '0'
      ? every(min, 'minute')
      : null) ??
    (sec === '0' && min === '0' ? every(hour, 'hour') : null)
  if (words) return words
  if (sec === '0' && /^\d+$/.test(min) && /^\d+$/.test(hour))
    return `every day at ${hour.padStart(2, '0')}:${min.padStart(2, '0')}`
  return null
}

const capitalize = (text: string) =>
  text ? text[0].toUpperCase() + text.slice(1) : text

/** `45 seconds`, `2 minutes`, `1 hour`: a delay in its largest whole unit. */
function durationInWords(ms: number): string {
  const [n, unit] =
    ms >= 3_600_000 && ms % 3_600_000 === 0
      ? [ms / 3_600_000, 'hour']
      : ms >= 60_000 && ms % 60_000 === 0
        ? [ms / 60_000, 'minute']
        : [Math.round(ms / 1000), 'second']
  return `${n} ${unit}${n === 1 ? '' : 's'}`
}

/** `Today at 18:43`, `Tomorrow at 09:30`, or a short date and time. */
export function whenInWords(at: number, now = Date.now()): string {
  const date = new Date(at)
  const time = date.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
  const day = (offset: number) => {
    const d = new Date(now)
    d.setDate(d.getDate() + offset)
    return d.toDateString()
  }
  if (date.toDateString() === day(0)) return `today at ${time}`
  if (date.toDateString() === day(1)) return `tomorrow at ${time}`
  if (date.toDateString() === day(-1)) return `yesterday at ${time}`
  return date.toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })
}

/** The `$eq` a filter pins a field to, when it pins one. */
function pinned(filters: unknown, field: string): string | null {
  const rule = (filters as Record<string, unknown> | undefined)?.[field]
  const value = (rule as { $eq?: unknown } | undefined)?.$eq
  return typeof value === 'string' ? value : null
}

/**
 * What sets the observer off, in words: `Every 5 seconds`, `When
 * demo-orders changes`, `Once, after 45 seconds`. A source it has no words
 * for reads `When <function> runs`.
 */
export function observerTrigger(observer: SessionObserverInfo): string {
  const args = observer.args ?? {}
  switch (observer.functionId) {
    case 'cron::on-tick': {
      const expression = args.expression
      if (typeof expression === 'string')
        return capitalize(
          cronInWords(expression) ?? `On the schedule ${expression}`,
        )
      break
    }
    case 'harness::on-timer': {
      if (typeof args.in_ms === 'number')
        return `Once, after ${durationInWords(args.in_ms)}`
      if (typeof args.at === 'number') return `Once, ${whenInWords(args.at)}`
      break
    }
    case 'state::on-changed': {
      const scope = pinned(observer.filters, 'input.scope')
      const key = pinned(observer.filters, 'input.key')
      if (scope && key) return `When ${key} in ${scope} changes`
      if (scope) return `When ${scope} changes`
      return 'When any state changes'
    }
    case 'database::on-row-changed': {
      const table = typeof args.table === 'string' ? args.table : null
      return table
        ? `When a ${table} row changes`
        : 'When a database row changes'
    }
  }
  return `When ${observer.functionId} runs`
}

const lowerFirst = (text: string) =>
  text ? text[0].toLowerCase() + text.slice(1) : text

/**
 * The observer in one sentence: what it looks for, how often, and what
 * happens then. "Looks for “Order changed” every 5 seconds. When it
 * happens, it tells this chat."
 */
export function observerSentence(observer: SessionObserverInfo): string {
  const when = lowerFirst(observerTrigger(observer))
  const then =
    observer.delivery.kind === 'call'
      ? `calls ${observer.delivery.functionId}`
      : 'tells this chat'
  return observer.action
    ? `Looks for “${observer.action}” ${when}. When it happens, it ${then}.`
    : `Runs ${when}. Each time it fires, it ${then}.`
}

/** How it stops, in words, for the dialog's "Stops" line. */
function stopsInWords(observer: SessionObserverInfo, ended: boolean): string {
  if (ended) {
    const why = lifecycleNote(observer)
    const words: Record<string, string> = {
      expired: 'Ended: time ran out',
      removed: 'Ended: removed',
      'once · consumed automatically': 'Ended after it fired once',
      'fire limit reached': 'Ended: fire limit reached',
      invalidated: 'Ended: its source went away',
      exhausted: 'Ended: fire limit reached',
    }
    return (why && words[why]) || 'Ended'
  }
  if (observer.once) return 'After it fires once'
  if (observer.maxFires !== undefined)
    return `After ${observer.maxFires} fire${observer.maxFires === 1 ? '' : 's'}`
  if (observer.expiresAt !== undefined)
    return capitalize(whenInWords(observer.expiresAt))
  return 'When you remove it'
}

/** The condition functions an observer checks before it fires. */
function conditionNames(observer: SessionObserverInfo): string[] {
  return (observer.conditions ?? []).flatMap((condition) => {
    const id =
      condition && typeof condition === 'object'
        ? (condition as { function_id?: unknown }).function_id
        : undefined
    return typeof id === 'string' ? [id] : []
  })
}

function isEmptyObject(value: unknown): boolean {
  if (value === null || value === undefined) return true
  if (typeof value === 'object')
    return Object.keys(value as Record<string, unknown>).length === 0
  return false
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

/** A labeled block of JSON inside the technical details. */
function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="grid gap-1.5">
      <div className="text-xs font-medium text-ink-faint">{label}</div>
      <div className="overflow-hidden rounded-lg ring-1 ring-dialog-edge">
        <JsonHighlight code={formatJson(value)} wrap />
      </div>
    </div>
  )
}

/** Monospace value with a copy affordance for long opaque ids. */
function CopyableId({ value }: { value: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      <span className="font-mono text-[13px] break-all text-ink">{value}</span>
      <Hint label={copied ? 'Copied' : 'Copy ID'}>
        <button
          type="button"
          onClick={() => {
            // Helper, not navigator.clipboard: the API is undefined over
            // `http://<LAN-IP>` (insecure context) and the raw call no-ops.
            void copyTextToClipboard(value).then((ok) => {
              if (!ok) return
              setCopied(true)
              window.setTimeout(() => setCopied(false), 1200)
            })
          }}
          className="shrink-0 rounded text-ink-faint transition-colors hover:text-ink"
          aria-label={copied ? 'Copied' : 'Copy observer ID'}
        >
          {copied ? (
            <Check size={16} aria-hidden />
          ) : (
            <Copy size={16} aria-hidden />
          )}
        </button>
      </Hint>
    </span>
  )
}

/** One line of "How it works": a quiet label and a plain value. */
function Step({
  label,
  children,
  detail,
}: {
  label: string
  children: ReactNode
  /** The machine name under the words (a function id), in mono. */
  detail?: string
}) {
  return (
    <div className="grid grid-cols-[6.5rem_1fr] gap-3 py-2">
      <dt className="text-ink-faint">{label}</dt>
      <dd className="min-w-0 text-ink">
        {children}
        {detail ? (
          <div className="mt-0.5 truncate font-mono text-xs text-ink-faint">
            {detail}
          </div>
        ) : null}
      </dd>
    </div>
  )
}

/** Running or ended, as a dot and a word. */
export function ObserverStatus({ ended }: { ended: boolean }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-xs font-medium text-ink-faint">
      <StatusDot tone={ended ? 'ghost' : 'ok'} />
      {ended ? 'Ended' : 'Watching'}
    </span>
  )
}

const RECENT_EVENTS = 5

/**
 * One event in words. A record of the observer stopping (it expired, went
 * stale) is not the thing it watched for, so it never borrows the action.
 */
export function eventInWords(event: SystemMessage): {
  text: string
  tone: 'warn' | 'ghost'
} {
  if (activityIdPrefix(event.id, 'skip') !== null)
    return { text: 'Checked, not ready yet', tone: 'ghost' }
  if (activityIdPrefix(event.id, 'expired') !== null)
    return { text: 'Time ran out, stopped watching', tone: 'ghost' }
  if (activityIdPrefix(event.id, 'stale') !== null)
    return { text: 'Stopped: its target is gone', tone: 'ghost' }
  if (activityIdPrefix(event.id, 'unregistered') !== null)
    return { text: 'Removed', tone: 'ghost' }
  return { text: event.observer?.action ?? event.content, tone: 'warn' }
}

/**
 * One observer, in words first: what sets it off, what it checks, what it
 * does and when it stops, then what it did in this chat. Ids and JSON wait
 * behind "Technical details". Opened from the chat's side panel.
 */
export function ObserverDetailDialog({
  observer,
  ended = observer?.fired === true,
  events = [],
  busy,
  onOpenChange,
  onDispose,
}: {
  observer: SessionObserverInfo | null
  /** Ended, as the panel knows it (it may know more than the row says). */
  ended?: boolean
  /** This observer's events in this chat, oldest first. */
  events?: readonly SystemMessage[]
  busy: boolean
  onOpenChange: (open: boolean) => void
  onDispose: (id: string) => void
}) {
  const [details, setDetails] = useState(false)
  const name = observer ? observer.label || observer.functionId : ''
  const conditions = observer ? conditionNames(observer) : []
  const recent = [...events].reverse().slice(0, RECENT_EVENTS)
  const waits = events.filter(
    (event) => activityIdPrefix(event.id, 'skip') !== null,
  ).length
  return (
    <Dialog
      open={observer !== null}
      onOpenChange={(open) => {
        if (!open) setDetails(false)
        onOpenChange(open)
      }}
    >
      <DialogContent
        className="max-h-[min(40rem,calc(100dvh-2rem))] gap-0 overflow-y-auto p-0"
        style={{ '--dialog-max-width': '32rem' } as CSSProperties}
        data-observer-dialog=""
      >
        {observer ? (
          <>
            <div className="flex items-start gap-3 p-5 pb-4">
              <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-warn-muted">
                <ObserverIcon
                  aria-hidden
                  className={cn(
                    'size-[18px]',
                    ended ? 'fill-ink-faint' : 'fill-warn',
                  )}
                />
              </div>
              <div className="min-w-0 flex-1 pr-6">
                <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
                  <DialogTitle className="text-base font-semibold text-ink">
                    {name}
                  </DialogTitle>
                  <ObserverStatus ended={ended} />
                </div>
                <DialogDescription className="mt-1.5 text-sm text-pretty text-ink-faint">
                  {observerSentence(observer)}
                </DialogDescription>
              </div>
            </div>

            <section className="border-t border-dialog-edge px-5 py-3">
              <h3 className="text-xs font-medium text-ink-faint">
                How it works
              </h3>
              <dl className="mt-1 divide-y divide-dialog-edge text-sm">
                <Step label="Watches" detail={observer.functionId}>
                  {observerTrigger(observer)}
                </Step>
                {conditions.length > 0 ? (
                  <Step label="Only when" detail={conditions.join(' · ')}>
                    {conditions.length === 1
                      ? 'Its condition says go'
                      : `All ${conditions.length} conditions say go`}
                  </Step>
                ) : null}
                <Step
                  label="Then"
                  detail={
                    observer.delivery.kind === 'call'
                      ? observer.delivery.functionId
                      : undefined
                  }
                >
                  {observer.delivery.kind === 'call'
                    ? 'Calls a function'
                    : 'Tells this chat'}
                </Step>
                <Step label="Stops">{stopsInWords(observer, ended)}</Step>
              </dl>
            </section>

            {events.length > 0 ? (
              <section className="border-t border-dialog-edge px-5 py-3">
                <h3 className="text-xs font-medium text-ink-faint">
                  Recent activity · {events.length}{' '}
                  {events.length === 1 ? 'event' : 'events'}
                  {waits > 0 ? `, ${waits} not ready yet` : ''}
                </h3>
                <ul className="mt-2 grid gap-1 text-sm" data-observer-recent="">
                  {recent.map((event) => {
                    const { text, tone } = eventInWords(event)
                    return (
                      <li
                        key={event.id}
                        className="flex items-center gap-2.5 py-1"
                      >
                        <StatusDot tone={tone} />
                        <span className="min-w-0 flex-1 truncate text-ink">
                          {text}
                        </span>
                        <span className="shrink-0 text-xs text-ink-faint tabular-nums">
                          {formatRelative(
                            event.observer?.fired_at ?? event.createdAt,
                          )}
                        </span>
                      </li>
                    )
                  })}
                </ul>
              </section>
            ) : null}

            <section className="border-t border-dialog-edge px-5 py-3">
              <button
                type="button"
                aria-expanded={details}
                onClick={() => setDetails((open) => !open)}
                className="-mx-1 flex items-center gap-1.5 rounded px-1 text-xs font-medium text-ink-faint transition-colors hover:text-ink"
              >
                <ChevronRight
                  aria-hidden
                  className={cn(
                    'size-4 transition-transform',
                    details && 'rotate-90',
                  )}
                />
                Technical details
              </button>
              {details ? (
                <div className="mt-3 grid gap-3 pb-1 text-sm">
                  <div className="grid gap-1.5">
                    <div className="text-xs font-medium text-ink-faint">
                      Observer ID
                    </div>
                    <CopyableId value={observer.id} />
                  </div>
                  {observer.namespace ? (
                    <div className="grid gap-1.5">
                      <div className="text-xs font-medium text-ink-faint">
                        Namespace
                      </div>
                      <span className="font-mono text-[13px] text-ink">
                        {observer.namespace}
                      </span>
                    </div>
                  ) : null}
                  {observer.createdAt !== undefined ? (
                    <div className="grid gap-1.5">
                      <div className="text-xs font-medium text-ink-faint">
                        Created
                      </div>
                      <span className="text-ink">
                        {new Date(observer.createdAt).toLocaleString()}
                      </span>
                    </div>
                  ) : null}
                  {isEmptyObject(observer.args) ? null : (
                    <JsonBlock label="Args" value={observer.args} />
                  )}
                  {isEmptyObject(observer.filters) ? null : (
                    <JsonBlock label="Filters" value={observer.filters} />
                  )}
                  {observer.conditions && observer.conditions.length > 0 ? (
                    <JsonBlock label="Conditions" value={observer.conditions} />
                  ) : null}
                </div>
              ) : null}
            </section>

            <DialogFooter className="m-0 rounded-b-xl border-dialog-edge bg-transparent px-5 py-3">
              {ended ? null : (
                <Button
                  type="button"
                  variant="destructive"
                  disabled={busy}
                  onClick={() => onDispose(observer.id)}
                >
                  {busy ? 'Removing…' : 'Remove observer'}
                </Button>
              )}
              <DialogClose render={<Button variant="secondary" />}>
                Close
              </DialogClose>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
