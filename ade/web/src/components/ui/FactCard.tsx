import type { LucideIcon } from 'lucide-react'
import type { ComponentProps, CSSProperties, ReactNode } from 'react'
import { cn } from '@/lib/utils'

/**
 * The fact card: one recipe for every "a thing and its numbers" surface in
 * the console — the span hover card, the context-usage popover, technical
 * details under a notice, a trigger's subscription, error metadata.
 *
 * Anatomy: an optional header (a tinted icon chip in the subject's colour,
 * then the name), an optional status line (a semantic dot and word), then
 * a `FactList` of label/value rows and, when there is a share to show, a
 * `FactMeter`. Labels are Inter 14px ink-faint; values are Inter 14px ink,
 * or Geist Mono 13px tabular for machine values (ids, durations, paths) —
 * 13px mono sits on the same optical size as 14px Inter.
 *
 * `accent` is any CSS colour (a hex or a `var(--…)` token). It is the only
 * colour the card adds: the chip tint, the icon and the meter fill. Status
 * keeps the semantic palette (`ok` / `alert` / `warn`), never the accent.
 */

export const factLabelClassName = 'font-inter text-sm text-ink-faint'
export const factValueClassName = 'font-inter text-sm text-ink'
export const factMonoValueClassName =
  'font-mono text-[13px] text-ink tabular-nums'

type AccentStyle = CSSProperties & { '--fact-accent'?: string }

function accentStyle(accent: string | undefined): AccentStyle | undefined {
  return accent ? { '--fact-accent': accent } : undefined
}

export interface FactCardProps extends ComponentProps<'div'> {
  /** The subject's colour: a worker hue, a tone token. Tints the chip. */
  accent?: string
}

/** The floating shell. Popover-styled; stacks header, status, list, meter. */
export function FactCard({
  accent,
  className,
  style,
  ...props
}: FactCardProps) {
  return (
    <div
      data-slot="fact-card"
      className={cn(
        'flex min-w-0 flex-col gap-2 rounded-lg bg-popover px-3 py-2.5 font-inter text-sm text-ink shadow-md ring-1 ring-foreground/10',
        className,
      )}
      style={{ ...accentStyle(accent), ...style }}
      {...props}
    />
  )
}

export interface FactCardHeaderProps {
  icon?: LucideIcon
  /** Overrides the card's accent for this header alone. */
  accent?: string
  /** The subject's name. */
  title: ReactNode
  /** Machine names (span names, ids) set in mono; defaults to Inter. */
  mono?: boolean
  /** A short line under the title: the model, the worker, a path. */
  description?: ReactNode
  /** Anything on the trailing edge: a close button, a count. */
  trailing?: ReactNode
  className?: string
}

/** The icon chip and the name. */
export function FactCardHeader({
  icon: Icon,
  accent,
  title,
  mono = false,
  description,
  trailing,
  className,
}: FactCardHeaderProps) {
  return (
    <div
      data-slot="fact-card-header"
      className={cn('flex min-w-0 items-start gap-2', className)}
      style={accentStyle(accent)}
    >
      {Icon ? (
        <span
          aria-hidden
          className="mt-px flex size-6 shrink-0 items-center justify-center rounded-md bg-[color-mix(in_oklab,var(--fact-accent,var(--color-ink))_14%,transparent)] text-(--fact-accent,var(--color-ink))"
        >
          <Icon className="size-4" />
        </span>
      ) : null}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div
          className={cn(
            'min-w-0 leading-snug font-medium text-ink',
            mono ? 'font-mono text-[13px] break-all' : 'text-sm break-words',
          )}
        >
          {title}
        </div>
        {description ? (
          <div className="min-w-0 truncate text-xs text-ink-faint">
            {description}
          </div>
        ) : null}
      </div>
      {trailing ? <div className="shrink-0">{trailing}</div> : null}
    </div>
  )
}

export type FactTone = 'ok' | 'alert' | 'warn' | 'pending' | 'unset'

const TONE_DOT: Record<FactTone, string> = {
  ok: 'bg-ok',
  alert: 'bg-alert',
  warn: 'bg-warn',
  pending: 'bg-sky-500 pulse-dot',
  unset: 'bg-ink-faint',
}

const TONE_TEXT: Record<FactTone, string> = {
  ok: 'text-ok',
  alert: 'text-alert-strong',
  warn: 'text-warn-strong',
  pending: 'text-sky-600 dark:text-sky-400',
  unset: 'text-ink-faint',
}

export interface FactCardStatusProps {
  tone: FactTone
  label: ReactNode
  /** Trailing context after a middle dot: the worker, the provider. */
  meta?: ReactNode
  className?: string
}

/** A semantic dot, the status word, then the context after a middle dot. */
export function FactCardStatus({
  tone,
  label,
  meta,
  className,
}: FactCardStatusProps) {
  return (
    <div
      data-slot="fact-card-status"
      className={cn('flex min-w-0 items-center gap-1.5 text-xs', className)}
    >
      <span
        aria-hidden
        className={cn('size-1.5 shrink-0 rounded-full', TONE_DOT[tone])}
      />
      <span className={cn('shrink-0', TONE_TEXT[tone])}>{label}</span>
      {meta ? (
        <>
          <span aria-hidden className="text-ink-faint">
            ·
          </span>
          <span className="min-w-0 truncate text-ink-faint">{meta}</span>
        </>
      ) : null}
    </div>
  )
}

/** The label/value grid. Labels hug their column; values take the rest. */
export function FactList({ className, ...props }: ComponentProps<'dl'>) {
  return (
    <dl
      data-slot="fact-list"
      className={cn(
        'm-0 grid grid-cols-[auto_minmax(0,1fr)] items-baseline gap-x-4 gap-y-1',
        className,
      )}
      {...props}
    />
  )
}

export interface FactProps extends Omit<ComponentProps<'dd'>, 'children'> {
  label: ReactNode
  /** Machine values (durations, ids, paths) in mono with tabular figures. */
  mono?: boolean
  /** Right-align the value: numbers in a narrow card line up on the edge. */
  align?: 'start' | 'end'
  /** Quieter value: a secondary fact, an unset default. */
  muted?: boolean
  children: ReactNode
}

/** One row: the label, then the value. Extra props land on the `dd`. */
export function Fact({
  label,
  mono = false,
  align = 'start',
  muted = false,
  className,
  children,
  ...props
}: FactProps) {
  return (
    <>
      <dt className={cn(factLabelClassName, 'min-w-0')}>{label}</dt>
      <dd
        className={cn(
          'm-0 min-w-0 wrap-break-word',
          mono ? factMonoValueClassName : factValueClassName,
          muted && 'text-ink-faint',
          align === 'end' && 'text-right',
          className,
        )}
        {...props}
      >
        {children}
      </dd>
    </>
  )
}

export interface FactMeterProps {
  /** 0..100 */
  value: number
  /** Overrides the card's accent for the fill. */
  accent?: string
  /** Accessible name for the progressbar. */
  label: string
  /** Still growing: the fill pulses at full width. */
  pending?: boolean
  className?: string
}

/** A share of a whole: a hairline track with the accent as the fill. */
export function FactMeter({
  value,
  accent,
  label,
  pending = false,
  className,
}: FactMeterProps) {
  const clamped = Math.min(100, Math.max(0, value))
  return (
    <div
      data-slot="fact-meter"
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamped)}
      className={cn(
        'h-1 w-full overflow-hidden rounded-full bg-muted',
        className,
      )}
      style={accentStyle(accent)}
    >
      <div
        className={cn(
          'h-full rounded-full bg-(--fact-accent,var(--color-ink)) transition-[width] duration-(--motion-duration-panel)',
          pending && 'animate-pulse',
        )}
        style={{ width: pending ? '100%' : `${Math.max(1, clamped)}%` }}
      />
    </div>
  )
}
