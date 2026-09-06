import { ArrowDown, ArrowRight, Braces, Check, Copy } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { CardHighlight } from '@/components/ui/Surface'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'

/*
 * Shared vocabulary for trigger detail surfaces (registration details and
 * fired/retired activity). Every piece follows the function-call card chrome
 * it sits inside: `label-caps-sm` eyebrows in mono, sans copy, machine values
 * in mono, one 16 px Lucide glyph on a 32 px neutral tile, and accent reserved
 * for live activity.
 */

/** The card chrome's `function` / `request` / `response` eyebrow recipe. */
export const TRIGGER_EYEBROW_CLASS =
  'font-mono text-[11px] font-medium tracking-[0.06em] text-ink-faint uppercase'

export function TriggerEyebrow({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) {
  return <div className={cn(TRIGGER_EYEBROW_CLASS, className)}>{children}</div>
}

export type TriggerGlyphTone = 'neutral' | 'accent' | 'warn'

const glyphTone: Record<TriggerGlyphTone, string> = {
  neutral: 'bg-surface [&_svg]:stroke-ink-faint',
  accent: 'bg-accent-muted [&_svg]:stroke-accent',
  warn: 'bg-warn-muted [&_svg]:stroke-warn',
}

/** 32 px tile holding one 16 px glyph. Neutral by default; `accent` marks
 * live activity and `warn` a failed or skipped outcome. */
export function TriggerGlyph({
  tone = 'neutral',
  className,
  children,
}: {
  tone?: TriggerGlyphTone
  className?: string
  children: ReactNode
}) {
  return (
    <div
      aria-hidden
      className={cn(
        'flex size-8 shrink-0 items-center justify-center rounded-md [&_svg]:size-4 [&_svg]:shrink-0',
        glyphTone[tone],
        className,
      )}
    >
      {children}
    </div>
  )
}

export function TriggerTrace({
  when,
  then,
}: {
  when: ReactNode
  then: ReactNode
}) {
  return (
    <CardHighlight data-trigger-execution-trace>
      <div className="grid min-w-0 @xl:grid-cols-[minmax(0,1fr)_3rem_minmax(0,1fr)] @xl:items-stretch">
        {when}
        <TriggerTraceConnector />
        {then}
      </div>
    </CardHighlight>
  )
}

export function TriggerTraceNode({
  kind,
  icon,
  label,
  title,
  mono = false,
  children,
}: {
  kind: 'when' | 'then'
  icon: ReactNode
  label: string
  title: string
  /** The title is a machine identifier (trigger type, function id). */
  mono?: boolean
  children?: ReactNode
}) {
  return (
    <div
      className="flex min-w-0 items-start gap-3 p-3 @xl:p-4"
      data-trigger-flow-card={kind}
    >
      <TriggerGlyph>{icon}</TriggerGlyph>
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="flex min-w-0 flex-col gap-0.5">
          <TriggerEyebrow>{label}</TriggerEyebrow>
          <div
            className={cn(
              'min-w-0 break-all text-ink',
              mono
                ? 'font-mono text-[13px]'
                : 'font-sans text-base font-medium sm:text-sm',
            )}
          >
            {title}
          </div>
        </div>
        {children}
      </div>
    </div>
  )
}

export interface TriggerStat {
  label: string
  value: string
  /** Machine-produced value (id, path): mono, and truncated with the full
   * value in `title` instead of orphaning its tail on a narrow pane. */
  mono?: boolean
}

export function TriggerStats({ items }: { items: readonly TriggerStat[] }) {
  return (
    <dl className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
      {items.map((item, index) => (
        <div
          key={item.label}
          className="flex min-w-0 items-baseline gap-1.5"
          data-trigger-activity-stat={item.label.toLowerCase()}
        >
          {index > 0 ? (
            <span aria-hidden className="shrink-0 text-ink-ghost">
              ·
            </span>
          ) : null}
          <dt className={cn('shrink-0', TRIGGER_EYEBROW_CLASS)}>
            {item.label}
          </dt>
          <dd
            className={cn(
              'min-w-0 text-ink tabular-nums',
              item.mono
                ? 'truncate font-mono text-[13px]'
                : 'wrap-anywhere font-sans text-base sm:text-sm',
            )}
            title={item.mono ? item.value : undefined}
          >
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  )
}

/** Sits on the WHEN/THEN tiles' axis: under the tile when stacked, level
 * with the tile centre in the wide two-column layout. */
function TriggerTraceConnector() {
  return (
    <div
      aria-hidden
      className="flex min-w-0 items-center px-3 @xl:items-start @xl:px-0 @xl:pt-4"
    >
      <div className="flex size-8 shrink-0 items-center justify-center @xl:hidden">
        <ArrowDown className="size-4 shrink-0 stroke-ink-ghost" />
      </div>
      <div className="hidden h-8 min-w-0 flex-1 items-center @xl:flex">
        <span className="h-px min-w-0 flex-1 border-t border-dashed border-edge" />
        <ArrowRight className="size-4 shrink-0 stroke-ink-ghost" />
        <span className="h-px min-w-0 flex-1 border-t border-dashed border-edge" />
      </div>
    </div>
  )
}

export function TriggerJsonPane({
  label,
  value,
  variant = 'panel',
}: {
  label: string
  value: unknown
  variant?: 'panel' | 'secondary'
}) {
  const json = safeJson(value)
  const secondary = variant === 'secondary'
  return (
    <div
      className={cn(
        'min-w-0 overflow-hidden',
        secondary
          ? 'border-t border-edge'
          : 'rounded-md border border-edge bg-surface',
      )}
      data-function-pane={label.toLowerCase().replaceAll(' ', '-')}
    >
      <TriggerPaneHeader label={label} copyText={json} secondary={secondary} />
      <div
        className={cn(
          'max-h-64 overflow-auto',
          secondary && 'rounded-md bg-bg',
        )}
      >
        <JsonHighlight code={json} wrap />
      </div>
    </div>
  )
}

function TriggerPaneHeader({
  label,
  copyText,
  secondary = false,
}: {
  label: string
  copyText: string
  secondary?: boolean
}) {
  const [copied, setCopied] = useState(false)
  return (
    <div
      className={cn(
        'flex min-w-0 items-center gap-2 font-sans text-base font-medium text-ink-faint sm:text-sm',
        secondary ? 'py-3' : 'border-b border-edge bg-surface px-3 py-2',
      )}
    >
      <Braces
        aria-hidden
        className="size-5 shrink-0 stroke-ink-ghost sm:size-4"
      />
      <div className="min-w-0 flex-1 truncate">{label}</div>
      <button
        type="button"
        onClick={() => {
          void copyTextToClipboard(copyText).then((ok) => {
            if (!ok) return
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1200)
          })
        }}
        className="relative flex shrink-0 cursor-pointer items-center gap-1.5 font-sans text-base text-ink-ghost hover:text-ink sm:text-xs"
        aria-label={copied ? 'copied' : `copy ${label}`}
        title={copied ? 'copied' : 'copy'}
      >
        <span
          aria-hidden
          className="pointer-fine:hidden absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2"
        />
        {copied ? (
          <Check aria-hidden className="size-5 shrink-0 sm:size-4" />
        ) : (
          <Copy aria-hidden className="size-5 shrink-0 sm:size-4" />
        )}
        <span>{copied ? 'Copied' : 'Copy'}</span>
      </button>
    </div>
  )
}

const safeJson = (value: unknown): string => {
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return '[unserializable value]'
  }
}
