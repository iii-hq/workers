import { Check, CircleAlert, LoaderCircle } from 'lucide-react'
import type * as React from 'react'
import { useEffect, useRef } from 'react'
import { KeyChoice } from '@/components/secrets/KeyChoice'
import { Chip } from '@/components/ui/Chip'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { describeStep, type PlanStep } from '@/lib/onboarding/plan'
import {
  defaultKeyInput,
  type KeyDetection,
  type KeyInput,
  keyInputReady,
  secretRef,
} from '@/lib/secrets'
import { cn } from '@/lib/utils'
import type { ActivityEntry } from './use-onboarding'

export function StepHeader({
  eyebrow,
  title,
  lead,
  action,
}: {
  eyebrow?: string
  title: string
  lead?: React.ReactNode
  action?: React.ReactNode
}) {
  return (
    <header className="flex flex-col gap-2">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1.5">
          {eyebrow ? <Eyebrow>{eyebrow}</Eyebrow> : null}
          <h2 className="text-pretty font-sans text-[20px] font-semibold leading-tight tracking-[-0.01em] text-ink">
            {title}
          </h2>
        </div>
        {action}
      </div>
      {lead ? (
        <p className="max-w-[60ch] text-pretty font-sans text-[13px] leading-relaxed text-ink-faint">
          {lead}
        </p>
      ) : null}
    </header>
  )
}

export function Section({
  title,
  aside,
  children,
}: {
  title: string
  aside?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-2" aria-label={title}>
      <div className="flex items-center justify-between gap-3">
        <h3 className="font-sans text-[12px] font-semibold text-ink-faint">
          {title}
        </h3>
        {aside}
      </div>
      {children}
    </section>
  )
}

/** A quiet framed group of rows: one background step, no outline. */
export function Rows({
  children,
  className,
}: {
  children: React.ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        'flex flex-col overflow-hidden rounded-md bg-surface [&>*+*]:shadow-[0_-1px_0_var(--color-edge)]',
        className,
      )}
    >
      {children}
    </div>
  )
}

export type Tone = 'ok' | 'warn' | 'neutral' | 'accent'

const CHIP_TONE = {
  ok: 'success',
  warn: 'warning',
  neutral: 'neutral',
  accent: 'accent',
} as const

export function StatusChip({
  tone,
  children,
}: {
  tone: Tone
  children: React.ReactNode
}) {
  return (
    <Chip tone={CHIP_TONE[tone]} className="shrink-0">
      {children}
    </Chip>
  )
}

/** What a plan will do, read before the button is pressed. */
export function PlanPreview({
  steps,
  title = 'What happens when you continue',
}: {
  steps: readonly PlanStep[]
  title?: string
}) {
  if (steps.length === 0) return null
  return (
    <Section title={title}>
      <ol className="flex flex-col gap-2 rounded-md bg-card-highlight px-3 py-3">
        {steps.map((step, index) => {
          const { title: line, detail } = describeStep(step)
          return (
            <li
              // biome-ignore lint/suspicious/noArrayIndexKey: a plan is rebuilt whole; its order is its identity
              key={index}
              className="flex gap-3 font-sans text-[13px] text-ink"
            >
              <span className="w-4 shrink-0 text-right font-mono text-[11px] leading-5 tabular-nums text-ink-ghost">
                {index + 1}
              </span>
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="leading-5">{line}</span>
                {step.kind === 'add-workers' ? (
                  <span className="flex flex-col gap-0.5">
                    {step.workers.map((worker) => (
                      <span
                        key={worker}
                        className="text-[12px] leading-relaxed text-ink-faint"
                      >
                        <span className="font-mono text-ink">{worker}</span>
                        {' — '}
                        {step.why[worker]}
                      </span>
                    ))}
                  </span>
                ) : (
                  <span className="break-all font-mono text-[11px] text-ink-ghost">
                    {detail}
                  </span>
                )}
              </span>
            </li>
          )
        })}
      </ol>
    </Section>
  )
}

/**
 * Every action the wizard ran in this part of setup, live: the worker being
 * added and its compose phase, the secret stored, the setting written. The
 * log is `role="log"` so assistive tech hears each line as it lands.
 */
export function ActivityLog({
  entries,
  title = 'Activity',
}: {
  entries: readonly ActivityEntry[]
  title?: string
}) {
  const list = useRef<HTMLOListElement>(null)
  const last = entries[entries.length - 1]
  const moving = last ? `${last.id}:${last.status}` : ''
  // Keep the line that is moving in view: a worker being added is the thing
  // to watch, not the form above it.
  useEffect(() => {
    if (!moving) return
    list.current?.lastElementChild?.scrollIntoView({
      block: 'nearest',
      behavior: 'smooth',
    })
  }, [moving])
  if (entries.length === 0) return null
  return (
    <Section title={title}>
      <ol
        ref={list}
        role="log"
        aria-live="polite"
        className="flex flex-col gap-1 rounded-md bg-bg px-3 py-2.5"
      >
        {entries.map((entry) => (
          <li
            key={entry.id}
            className="onboarding-rise flex items-start gap-2.5 py-1"
          >
            <ActivityIcon status={entry.status} />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="flex items-baseline justify-between gap-3">
                <span
                  className={cn(
                    'font-sans text-[13px]',
                    entry.status === 'failed'
                      ? 'text-alert-strong'
                      : 'text-ink',
                  )}
                >
                  {entry.title}
                </span>
                {typeof entry.progress === 'number' ? (
                  <span className="shrink-0 font-mono text-[11px] tabular-nums text-ink-ghost">
                    {Math.round(entry.progress * 100)}%
                  </span>
                ) : null}
              </span>
              <span className="break-all font-mono text-[11px] text-ink-ghost">
                {entry.detail}
              </span>
              {entry.note ? (
                <span
                  className={cn(
                    'break-words font-sans text-[12px]',
                    entry.status === 'failed'
                      ? 'text-alert-strong'
                      : 'text-ink-faint',
                  )}
                >
                  {entry.note}
                </span>
              ) : null}
            </span>
          </li>
        ))}
      </ol>
    </Section>
  )
}

function ActivityIcon({ status }: { status: ActivityEntry['status'] }) {
  if (status === 'running') {
    return (
      <LoaderCircle
        aria-label="running"
        className="iii-ui-spin mt-0.5 size-4 shrink-0 text-accent"
      />
    )
  }
  if (status === 'failed') {
    return (
      <CircleAlert
        aria-label="failed"
        className="mt-0.5 size-4 shrink-0 text-alert"
      />
    )
  }
  return <Check aria-label="done" className="mt-0.5 size-4 shrink-0 text-ok" />
}

/**
 * The wizard's key chooser: the console's shared `KeyChoice`, then where the
 * key goes, so storing it is never a surprise.
 */
export function KeyField({
  envVar,
  detection,
  value,
  onChange,
  keysUrl,
  secretsReady,
}: {
  envVar: string
  detection: KeyDetection | null
  value: KeyInput | undefined
  onChange: (next: KeyInput) => void
  keysUrl?: string
  secretsReady: boolean
}) {
  return (
    <div className="flex flex-col gap-2 px-3 pb-3">
      <KeyChoice
        name={envVar}
        detection={detection}
        value={value}
        onChange={onChange}
        keysUrl={keysUrl}
      />
      <p className="font-sans text-[12px] leading-relaxed text-ink-faint">
        {secretsReady
          ? 'Stored encrypted by the secrets worker as '
          : 'The secrets worker will store it encrypted as '}
        <span className="font-mono text-ink">{secretRef(envVar)}</span>. Only
        that reference is written to configuration, so the key never lands in a
        file you commit.
      </p>
    </div>
  )
}

export { defaultKeyInput, keyInputReady }

/** A step: scrolling body over a footer that keeps its actions in reach. */
export function StepLayout({
  children,
  footer,
}: {
  children: React.ReactNode
  footer: React.ReactNode
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-6 @2xl:px-8">
        <div className="mx-auto flex max-w-[640px] flex-col gap-6">
          {children}
        </div>
      </div>
      <footer className="flex shrink-0 items-center justify-between gap-2 bg-panel-raised px-5 py-3 shadow-[0_-1px_0_var(--color-edge)] @2xl:px-8">
        {footer}
      </footer>
    </div>
  )
}
