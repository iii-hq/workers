import {
  Check,
  ChevronRight,
  Circle,
  CircleAlert,
  Copy,
  LoaderCircle,
  SkipForward,
} from 'lucide-react'
import { useReducedMotion } from 'motion/react'
import type * as React from 'react'
import { useEffect, useId, useRef, useState } from 'react'
import { KeyChoice } from '@/components/secrets/KeyChoice'
import { copyTextToClipboard } from '@/lib/clipboard'
import { describeStep, type PlanStep, stepDetail } from '@/lib/onboarding/plan'
import {
  DEFAULT_ENV_FILE,
  defaultKeyInput,
  type KeyDetection,
  type KeyInput,
  type KeyStore,
  keyInputReady,
  keyStore,
} from '@/lib/secrets'
import { cn } from '@/lib/utils'
import { Button } from './controls'
import { type ActivityEntry, SLOW_STEP_MS } from './use-onboarding'

/** Where a step sits among the setup steps (Welcome and Ready not counted). */
export interface StepPosition {
  /** 1-based. */
  index: number
  total: number
}

/**
 * `Step 2 of 3 · Optional`, from the wizard's actual list of steps. The rail
 * already shows the position, so the header keeps only the "Optional" part
 * (as its badge); the function stays for steps written against it.
 */
export function stepEyebrow(
  position: StepPosition | undefined,
  optional = false,
): string | undefined {
  const parts = [
    position ? `Step ${position.index} of ${position.total}` : null,
    optional ? 'Optional' : null,
  ].filter((part): part is string => part !== null)
  return parts.length > 0 ? parts.join(' · ') : undefined
}

/** The step's title, one line under it, and a small action at its right. */
export function StepHeader({
  eyebrow,
  title,
  badge,
  lead,
  action,
}: {
  /** A `stepEyebrow`; only its "Optional" survives, as the badge. */
  eyebrow?: string
  title: string
  /** A short tag beside the title: "Optional". */
  badge?: React.ReactNode
  lead?: React.ReactNode
  action?: React.ReactNode
}) {
  const tag = badge ?? (eyebrow?.includes('Optional') ? 'Optional' : null)
  return (
    <header className="flex flex-col gap-1">
      <div className="flex min-h-8 items-center justify-between gap-3">
        <h2 className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-balance font-sans text-xl font-semibold leading-6 tracking-[-0.01em] text-ink">
          {eyebrow ? <span className="sr-only">{eyebrow}. </span> : null}
          {title}
          {tag ? <StatusChip tone="neutral">{tag}</StatusChip> : null}
        </h2>
        {action}
      </div>
      {lead ? (
        <p className="max-w-[60ch] text-pretty font-sans text-sm leading-5 text-neutral-600 dark:text-neutral-400">
          {lead}
        </p>
      ) : null}
    </header>
  )
}

export function Section({
  title,
  hint,
  aside,
  children,
}: {
  title: string
  /** One quiet line beside the title. */
  hint?: React.ReactNode
  aside?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-2" aria-label={title}>
      <div className="flex min-h-5 flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5">
        <h3 className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 font-sans text-lg font-medium leading-7 text-ink">
          <span className="shrink-0">{title}</span>
          {hint ? (
            <span className="truncate text-xs font-normal text-neutral-500 dark:text-neutral-400">
              {hint}
            </span>
          ) : null}
        </h3>
        {aside}
      </div>
      {children}
    </section>
  )
}

/** A bordered group of rows with a hairline between each. */
export function Rows({
  children,
  className,
  as: Tag = 'div',
}: {
  children: React.ReactNode
  className?: string
  as?: 'div' | 'ul'
}) {
  return (
    <Tag
      className={cn(
        'flex flex-col divide-y divide-neutral-200 overflow-hidden rounded-lg border border-neutral-200 bg-white dark:divide-neutral-800 dark:border-neutral-800 dark:bg-neutral-950',
        className,
      )}
    >
      {children}
    </Tag>
  )
}

export type Tone = 'ok' | 'warn' | 'neutral' | 'accent'

/**
 * A small status: a bordered tag, with a dot for `ok` and `warn` so the
 * state reads from the shape before the text.
 */
export function StatusChip({
  tone,
  children,
}: {
  tone: Tone
  children: React.ReactNode
}) {
  return (
    <span
      data-tone={tone}
      className="inline-flex h-5 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full border border-neutral-200 bg-neutral-50 px-2 font-sans text-[12px] font-medium leading-none text-neutral-600 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300"
    >
      {tone === 'ok' || tone === 'warn' ? (
        <span
          aria-hidden
          className={cn(
            'size-1.5 rounded-full',
            tone === 'ok' ? 'bg-ok' : 'bg-warn',
          )}
        />
      ) : null}
      {children}
    </span>
  )
}

/**
 * A section that starts folded: the label and a one-line summary stay in
 * view, the body grows open below. Detail that helps a curious reader but
 * is not needed to finish setup lives in one of these.
 */
export function Disclosure({
  label,
  summary,
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  children,
}: {
  label: React.ReactNode
  /** Trailing one-liner, visible while folded. */
  summary?: React.ReactNode
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?: (open: boolean) => void
  children: React.ReactNode
}) {
  const [ownOpen, setOwnOpen] = useState(defaultOpen)
  const open = openProp ?? ownOpen
  const body = useId()
  const toggle = () => {
    setOwnOpen(!open)
    onOpenChange?.(!open)
  }
  return (
    <div className="flex flex-col">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={body}
        onClick={toggle}
        className="group -mx-1.5 flex h-8 items-center gap-1.5 rounded-md px-1.5 text-left font-sans text-[13px] font-medium text-neutral-600 transition-colors duration-150 hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus dark:text-neutral-400"
      >
        <ChevronRight
          aria-hidden
          className={cn(
            'size-4 shrink-0 transition-transform duration-200 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
            open && 'rotate-90',
          )}
        />
        <span className="shrink-0">{label}</span>
        {summary ? (
          <span className="ml-auto flex min-w-0 items-center gap-1.5 pl-3 text-xs font-normal">
            {summary}
          </span>
        ) : null}
      </button>
      <div
        id={body}
        className="grid grid-rows-[0fr] opacity-0 transition-[grid-template-rows,opacity] duration-200 ease-[var(--motion-ease-standard)] data-open:grid-rows-[1fr] data-open:opacity-100 motion-reduce:transition-none"
        data-open={open || undefined}
        inert={!open}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="pt-1.5">{children}</div>
        </div>
      </div>
    </div>
  )
}

/**
 * Everything the engine does for this step, as a terminal would print it:
 * the operations that ran (live while they run), then the ones queued for
 * the button. Folded by default with the latest line as its summary, so
 * the status stays visible without the detail crowding the choices; it
 * opens by itself when something fails.
 */
export function EngineLog({
  plan,
  entries,
  running,
  onSkip,
}: {
  /** Operations the primary action will run, not started yet. */
  plan: readonly PlanStep[]
  entries: readonly ActivityEntry[]
  running: boolean
  /** "Skip for now" on a step past `SLOW_STEP_MS`; omitted, none is offered. */
  onSkip?: () => void
}) {
  const reduceMotion = useReducedMotion()
  const last = entries[entries.length - 1]
  const failed = last?.status === 'failed'
  const [open, setOpen] = useState(false)
  const list = useRef<HTMLOListElement>(null)
  const moving = last ? `${last.id}:${last.status}` : ''
  useEffect(() => {
    if (failed) setOpen(true)
  }, [failed])
  // Continue starts its installs here, at the foot of a long step: open the
  // log and bring it into view as the run starts. Once per run, so someone
  // who folds it again keeps it folded.
  const section = useRef<HTMLDivElement>(null)
  const revealed = useRef(false)
  const revealTimer = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (!running) {
      revealed.current = false
      return
    }
    // The first entry may land a render after `running` does.
    if (revealed.current || !section.current || entries.length === 0) return
    revealed.current = true
    setOpen(true)
    // Scroll once the log has opened, so what it shows is in view too. Kept
    // in a ref: the next entry re-runs this effect and must not cancel it.
    revealTimer.current = window.setTimeout(
      () => {
        if (section.current) revealInScroller(section.current, !reduceMotion)
      },
      reduceMotion ? 0 : DISCLOSURE_MS,
    )
  }, [running, entries.length, reduceMotion])
  useEffect(() => () => window.clearTimeout(revealTimer.current), [])
  // Keep the line that is moving in view while the log is open.
  useEffect(() => {
    if (!moving || !open) return
    list.current?.lastElementChild?.scrollIntoView({
      block: 'nearest',
      behavior: reduceMotion ? 'instant' : 'smooth',
    })
  }, [moving, open, reduceMotion])
  const queued = running ? [] : plan
  const now = useNow(running)
  if (entries.length === 0 && queued.length === 0) return null
  const current = running && last?.status === 'running' ? last : null

  const doneCount = entries.filter((entry) => entry.status === 'done').length
  const summary =
    running && last ? (
      <>
        <LogGlyph status="running" />
        <span className="truncate text-ink">{last.title}</span>
      </>
    ) : failed && last ? (
      <>
        <LogGlyph status="failed" />
        <span className="truncate text-ink">{last.title}</span>
      </>
    ) : queued.length > 0 ? (
      <span className="font-mono tabular-nums text-neutral-500 dark:text-neutral-400">
        {queued.length} queued
      </span>
    ) : (
      <>
        <LogGlyph status="done" />
        <span className="font-mono tabular-nums text-neutral-500 dark:text-neutral-400">
          {doneCount} done
        </span>
      </>
    )

  return (
    <div ref={section} className="flex scroll-mt-4 flex-col gap-2">
      {/* Outside the folded (inert) log, so progress is announced while it
          is shut; no seconds in it, or it would speak every second. */}
      <span role="status" aria-live="polite" className="sr-only">
        {announcement(last, running, doneCount)}
      </span>
      {current ? (
        <RunningLine entry={current} now={now} onSkip={onSkip} />
      ) : null}
      <Disclosure
        label="Engine log"
        summary={summary}
        open={open}
        onOpenChange={setOpen}
      >
        <ol
          ref={list}
          aria-label="Engine log"
          className={cn(TERMINAL_SURFACE, 'gap-3 text-[13px] leading-5')}
        >
          {entries.map((entry) => (
            <li key={entry.id} className="flex gap-2.5">
              <LogGlyph status={entry.status} terminal />
              <LogLine
                title={entry.title}
                command={entry.detail}
                tone={entry.status === 'failed' ? 'failed' : 'ink'}
                trailing={
                  typeof entry.progress === 'number'
                    ? `${Math.round(entry.progress * 100)}%`
                    : undefined
                }
              >
                {entry.note ? (
                  <span
                    className={cn(
                      'break-words',
                      entry.status === 'failed'
                        ? 'text-rose-700 dark:text-rose-300'
                        : 'text-neutral-600 dark:text-neutral-400',
                    )}
                  >
                    {entry.status === 'failed' ? '✗ ' : '→ '}
                    {entry.note}
                    {entry.status === 'running'
                      ? ` (${seconds(now, entry.noteAt ?? entry.startedAt)})`
                      : null}
                  </span>
                ) : null}
              </LogLine>
            </li>
          ))}
          {queued.map((step, index) => {
            const title = describeStep(step)
            const detail = stepDetail(step)
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: a plan is rebuilt whole; its order is its identity
              <li key={`queued-${index}`} className="flex gap-2.5">
                <LogGlyph status="queued" terminal />
                <LogLine title={title} command={detail} tone="queued">
                  {step.kind === 'add-workers'
                    ? step.workers.map((worker) => (
                        <span key={worker} className="text-neutral-500">
                          # {worker}: {step.why[worker]}
                        </span>
                      ))
                    : null}
                </LogLine>
              </li>
            )
          })}
        </ol>
      </Disclosure>
    </div>
  )
}

function LogLine({
  title,
  command,
  tone,
  trailing,
  children,
}: {
  title: string
  command?: string
  tone: 'ink' | 'queued' | 'failed'
  trailing?: string
  children?: React.ReactNode
}) {
  return (
    <span className="flex min-w-0 flex-1 flex-col">
      <span className="flex items-baseline justify-between gap-3">
        <span
          className={cn(
            'font-sans text-[13px] font-medium',
            tone === 'queued'
              ? 'text-neutral-500 dark:text-neutral-400'
              : 'text-ink',
          )}
        >
          {title}
        </span>
        {trailing ? (
          <span className="shrink-0 tabular-nums text-neutral-500">
            {trailing}
          </span>
        ) : null}
      </span>
      {command ? (
        <span className="break-all text-neutral-600 dark:text-neutral-400">
          <span aria-hidden className="select-none text-neutral-500">
            ${' '}
          </span>
          {command}
        </span>
      ) : null}
      {children}
    </span>
  )
}

/**
 * The status mark: theme ink on the disclosure row, fixed colours on the
 * terminal surface (`terminal`), which is dark in both themes.
 */
/** The time now, once a second while `active`; frozen otherwise. */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 1_000)
    return () => window.clearInterval(timer)
  }, [active])
  return now
}

/** How long `Disclosure` takes to open (its grid-track transition). */
const DISCLOSURE_MS = 200

/**
 * Scroll `element` into view inside its nearest scrolling ancestor only (the
 * step body), never the page: all of it when it fits, else its top.
 */
export function revealInScroller(element: HTMLElement, smooth: boolean) {
  let scroller = element.parentElement
  while (
    scroller &&
    !/(auto|scroll)/.test(getComputedStyle(scroller).overflowY)
  ) {
    scroller = scroller.parentElement
  }
  if (!scroller) return
  const view = scroller.getBoundingClientRect()
  const box = element.getBoundingClientRect()
  const margin = 16
  const toTop = scroller.scrollTop + box.top - view.top - margin
  const toBottom = scroller.scrollTop + box.bottom - view.bottom + margin
  const top = Math.max(
    0,
    Math.min(toTop, Math.max(scroller.scrollTop, toBottom)),
  )
  scroller.scrollTo({ top, behavior: smooth ? 'smooth' : 'instant' })
}

/** What the live region says about the latest step: one line, no timer. */
function announcement(
  last: ActivityEntry | undefined,
  running: boolean,
  doneCount: number,
): string {
  if (!last) return ''
  if (last.status === 'running') {
    return running ? `${last.title}: ${last.note ?? 'in progress'}` : ''
  }
  if (last.status === 'failed') {
    return `${last.title} failed${last.note ? `: ${last.note}` : ''}`
  }
  if (last.status === 'skipped') return `${last.title} skipped`
  return running ? `${last.title} done` : `${doneCount} done`
}

/** Whole seconds since `since`, as the CLI counts them: `12s`, `1m 05s`. */
export function seconds(now: number, since: number | undefined): string {
  const total = Math.max(0, Math.floor((now - (since ?? now)) / 1_000))
  if (total < 60) return `${total}s`
  const minutes = Math.floor(total / 60)
  return `${minutes}m ${String(total % 60).padStart(2, '0')}s`
}

/**
 * The step running now, said the way the CLI says it — what it waits on and
 * for how long — in one card with the setup's other cards. Past
 * `SLOW_STEP_MS` the card opens a second row offering "Skip for now".
 */
function RunningLine({
  entry,
  now,
  onSkip,
}: {
  entry: ActivityEntry
  now: number
  onSkip?: () => void
}) {
  const slow =
    onSkip !== undefined &&
    entry.startedAt !== undefined &&
    now - entry.startedAt >= SLOW_STEP_MS
  return (
    <div className={CARD}>
      <p className="flex min-w-0 items-center gap-2.5 px-3.5 py-2.5 font-sans text-[13px] leading-5 text-ink">
        <LoaderCircle
          strokeWidth={1.5}
          className="size-4 shrink-0 animate-spin text-neutral-500 motion-reduce:animate-none dark:text-neutral-400"
          aria-hidden
        />
        <span className="min-w-0 truncate">
          {entry.note ?? entry.title}{' '}
          <span className="font-mono text-[12px] tabular-nums text-neutral-500 dark:text-neutral-400">
            ({seconds(now, entry.noteAt ?? entry.startedAt)})
          </span>
        </span>
      </p>
      {/* Opens by its grid track, so it can reverse midway; inert while shut. */}
      <div
        inert={!slow}
        className={cn(
          'grid transition-[grid-template-rows,opacity] duration-200 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:transition-none',
          slow ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0',
        )}
      >
        <div className="min-h-0 overflow-hidden">
          {onSkip ? (
            <SlowRow
              onSkip={onSkip}
              className="border-t border-neutral-200 dark:border-neutral-800"
            />
          ) : null}
        </div>
      </div>
    </div>
  )
}

/** The setup's card surface: the same frame `Rows` draws. */
const CARD =
  'flex flex-col overflow-hidden rounded-lg border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950'

/**
 * A step past `SLOW_STEP_MS`, on its own card (the Browser step's download).
 * Skipping ends only this step; what it asked of the engine may still finish.
 */
export function SlowStepNotice({ onSkip }: { onSkip: () => void }) {
  return (
    <div role="status" className={CARD}>
      <SlowRow onSkip={onSkip} />
    </div>
  )
}

function SlowRow({
  onSkip,
  className,
}: {
  onSkip: () => void
  className?: string
}) {
  return (
    <div
      className={cn(
        'flex items-center gap-3 px-3.5 py-2.5 font-sans text-[13px] leading-5',
        className,
      )}
    >
      {/* The setup's one attention mark: the dot "Not signed in" carries. */}
      <span
        aria-hidden
        className="flex h-5 w-4 shrink-0 items-center justify-center self-start"
      >
        <span className="size-1.5 rounded-full bg-warn" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium text-ink">Taking longer than usual</span>
        <span className="text-pretty text-neutral-600 dark:text-neutral-400">
          Skip to carry on. It can finish in the background.
        </span>
      </span>
      <Button variant="outline" size="sm" onClick={onSkip}>
        Skip for now
      </Button>
    </div>
  )
}

function LogGlyph({
  status,
  terminal = false,
}: {
  status: ActivityEntry['status'] | 'queued'
  terminal?: boolean
}) {
  const className = 'mt-0.5 size-4 shrink-0'
  if (status === 'running') {
    return (
      <LoaderCircle
        aria-label="running"
        className={cn(
          className,
          'animate-spin motion-reduce:animate-none',
          'text-ink',
        )}
      />
    )
  }
  if (status === 'failed') {
    return (
      <CircleAlert
        aria-label="failed"
        className={cn(
          className,
          terminal ? 'text-rose-700 dark:text-rose-400' : 'text-alert',
        )}
      />
    )
  }
  if (status === 'skipped') {
    return (
      <SkipForward
        aria-label="skipped"
        className={cn(className, 'p-px text-neutral-500 dark:text-neutral-400')}
      />
    )
  }
  if (status === 'queued') {
    return (
      <Circle
        aria-label="queued"
        className={cn(
          className,
          'p-[3px]',
          terminal
            ? 'text-neutral-600'
            : 'text-neutral-400 dark:text-neutral-500',
        )}
      />
    )
  }
  return (
    <Check
      aria-label="done"
      className={cn(
        className,
        terminal ? 'text-emerald-700 dark:text-emerald-400' : 'text-ink',
      )}
    />
  )
}

/**
 * What a step did, for steps written against upstream's `ActivityLog`: the
 * engine log with nothing queued.
 */
export function ActivityLog({
  entries,
}: {
  entries: readonly ActivityEntry[]
  title?: string
}) {
  return (
    <EngineLog
      plan={[]}
      entries={entries}
      running={entries.some((entry) => entry.status === 'running')}
    />
  )
}

/**
 * A terminal block: one or more shell commands to run elsewhere, on the
 * neutral code surface (a light panel in the light theme, a raised dark one
 * in the dark theme), with the program, its flags and a trailing comment
 * coloured the way a shell would. Each line has a round copy button
 * that shows a check for a moment once the command is on the clipboard.
 */
export function Terminal({ children }: { children: React.ReactNode }) {
  return (
    <div className={cn(TERMINAL_SURFACE, 'gap-1.5 text-sm leading-6')}>
      {children}
    </div>
  )
}

/** The code surface: one neutral step off the dialog in either theme. */
const TERMINAL_SURFACE =
  'flex flex-col rounded-lg border border-neutral-200 bg-neutral-50 px-4 py-3 font-code text-ink dark:border-neutral-800 dark:bg-neutral-900'

export function CommandLine({
  command,
  note,
}: {
  command: string
  /** What to do after the command, as a shell comment. */
  note?: string
}) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<number | null>(null)
  useEffect(
    () => () => {
      if (timer.current != null) window.clearTimeout(timer.current)
    },
    [],
  )
  const copy = () => {
    void copyTextToClipboard(command).then((ok) => {
      if (!ok) return
      setCopied(true)
      if (timer.current != null) window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => setCopied(false), 1500)
    })
  }
  const [program, ...rest] = command.split(' ')
  return (
    <div className="flex min-w-0 items-center gap-3">
      <code className="min-w-0 flex-1 truncate">
        <span aria-hidden className="select-none text-neutral-500">
          ${' '}
        </span>
        <span className="font-medium text-emerald-700 dark:text-emerald-400">
          {program}
        </span>
        {rest.map((token, index) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tokens of one fixed string
            key={index}
            className={
              token.startsWith('-')
                ? 'text-sky-700 dark:text-sky-300'
                : 'text-ink'
            }
          >
            {' '}
            {token}
          </span>
        ))}
        {note ? (
          <span className="text-neutral-500">
            {'  '}# {note}
          </span>
        ) : null}
      </code>
      {copied ? (
        <span
          aria-hidden
          className="shrink-0 font-sans text-[12px] font-medium text-emerald-700 dark:text-emerald-400"
        >
          Copied
        </span>
      ) : null}
      <button
        type="button"
        onClick={copy}
        aria-label={`Copy ${command}`}
        className="relative -my-1 flex size-8 shrink-0 items-center justify-center rounded-full text-neutral-500 transition-colors duration-150 hover:bg-neutral-200/70 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus focus-visible:ring-offset-2 focus-visible:ring-offset-neutral-50 dark:text-neutral-400 dark:hover:bg-white/10 dark:focus-visible:ring-offset-neutral-900"
      >
        <Copy
          aria-hidden
          className={cn(
            'absolute size-4 transition-[opacity,transform] duration-150 ease-[var(--motion-ease-standard)] motion-reduce:transition-none',
            copied ? 'scale-25 opacity-0' : 'scale-100 opacity-100',
          )}
        />
        <Check
          aria-hidden
          strokeWidth={2.5}
          className={cn(
            'absolute size-4 text-emerald-700 transition-[opacity,transform] duration-150 ease-[var(--motion-ease-standard)] motion-reduce:transition-none dark:text-emerald-400',
            copied ? 'scale-100 opacity-100' : 'scale-25 opacity-0',
          )}
        />
        <span role="status" className="sr-only">
          {copied ? 'Copied' : ''}
        </span>
      </button>
    </div>
  )
}

/** The wizard's key chooser: the console's shared `KeyChoice`. */
export function KeyField({
  envVar,
  detection,
  value,
  onChange,
  keysUrl,
  stores,
  envFile,
  className,
}: {
  envVar: string
  detection: KeyDetection | null
  value: KeyInput | undefined
  onChange: (next: KeyInput) => void
  keysUrl?: string
  /** Where the consumer can read the key from (default: encrypted only). */
  stores?: readonly KeyStore[]
  /** The secrets worker's env file, by name. */
  envFile?: string
  className?: string
}) {
  return (
    <div
      className={cn(
        'flex flex-col gap-2.5 text-[13px] [&_a]:text-[13px] [&_button]:text-[13px] [&_fieldset]:gap-2 [&_input]:rounded-md [&_input]:border-neutral-200 [&_input]:bg-white [&_input]:font-sans dark:[&_input]:border-neutral-800 dark:[&_input]:bg-neutral-950',
        className,
      )}
    >
      <KeyChoice
        name={envVar}
        detection={detection}
        value={value}
        onChange={onChange}
        keysUrl={keysUrl}
        stores={stores}
        envFile={envFile}
      />
      <p className="font-sans text-[13px] leading-5 text-neutral-500 dark:text-neutral-400">
        {keyStore(value ?? defaultKeyInput(detection)) === 'env'
          ? `Kept in this project's ${envFile ?? DEFAULT_ENV_FILE} file.`
          : 'Stored encrypted on this machine. It never lands in a file you commit.'}
      </p>
    </div>
  )
}

export { defaultKeyInput, keyInputReady }

/** A step: scrolling body over a footer that keeps its actions in reach. */
export function StepLayout({
  children,
  footer,
  centered = false,
}: {
  children: React.ReactNode
  footer: React.ReactNode
  /** Sit a short step in the middle of the dialog instead of at the top. */
  centered?: boolean
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        data-setup-scroll
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pt-5 pb-5 [scrollbar-gutter:stable] [scrollbar-width:thin] @md:px-6 @lg:pt-1"
      >
        <div
          className={cn(
            'flex w-full flex-col gap-5',
            centered && 'min-h-full justify-center',
          )}
        >
          {children}
        </div>
      </div>
      <footer className="flex h-14 shrink-0 items-center justify-between gap-2 border-t border-neutral-200 bg-white px-4 dark:border-neutral-800 dark:bg-neutral-950 @md:px-5">
        {footer}
      </footer>
    </div>
  )
}
