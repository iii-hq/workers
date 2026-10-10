import {
  Check,
  ChevronRight,
  Circle,
  CircleAlert,
  Copy,
  LoaderCircle,
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
import type { ActivityEntry } from './use-onboarding'

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
        <h2 className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-balance font-sans text-base font-semibold leading-6 tracking-[-0.01em] text-ink">
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
        <h3 className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 font-sans text-[13px] font-medium leading-5 text-ink">
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
      className="inline-flex h-5 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full border border-neutral-200 bg-neutral-50 px-2 font-sans text-[11px] font-medium leading-none text-neutral-600 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-300"
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
}: {
  /** Operations the primary action will run, not started yet. */
  plan: readonly PlanStep[]
  entries: readonly ActivityEntry[]
  running: boolean
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
  // Keep the line that is moving in view while the log is open.
  useEffect(() => {
    if (!moving || !open) return
    list.current?.lastElementChild?.scrollIntoView({
      block: 'nearest',
      behavior: reduceMotion ? 'instant' : 'smooth',
    })
  }, [moving, open, reduceMotion])
  const queued = running ? [] : plan
  if (entries.length === 0 && queued.length === 0) return null

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
    <Disclosure
      label="Engine log"
      summary={summary}
      open={open}
      onOpenChange={setOpen}
    >
      <ol
        ref={list}
        role="log"
        aria-live="polite"
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
    <div className="flex min-w-0 items-center gap-3 h-4">
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
      <p className="font-sans text-xs leading-4 text-neutral-500 dark:text-neutral-400">
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
