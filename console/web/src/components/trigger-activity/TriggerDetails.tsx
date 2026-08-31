import { ArrowDown, ArrowRight, Braces, Check, Copy } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { CardHighlight } from '@/components/ui/Surface'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'

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
  children,
}: {
  kind: 'when' | 'then'
  icon: ReactNode
  label: string
  title: string
  children?: ReactNode
}) {
  return (
    <div
      className="flex min-w-0 items-start gap-3 p-3 @xl:p-4"
      data-trigger-flow-card={kind}
    >
      <div className="flex size-10 shrink-0 items-center justify-center rounded-full bg-accent-muted [&_svg]:size-5 [&_svg]:shrink-0 [&_svg]:stroke-accent">
        {icon}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="font-mono text-base tracking-wide text-ink-ghost uppercase sm:text-xs">
          {label}
        </div>
        <div className="font-sans text-base font-medium break-all text-ink sm:text-sm">
          {title}
        </div>
        {children}
      </div>
    </div>
  )
}

export function TriggerStats({
  items,
}: {
  items: readonly { label: string; value: string }[]
}) {
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
          <dt className="font-mono text-base font-medium tracking-wide text-ink-faint uppercase sm:text-xs">
            {item.label}
          </dt>
          <dd className="min-w-0 font-sans text-base wrap-break-word text-ink-ghost tabular-nums sm:text-sm">
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  )
}

function TriggerTraceConnector() {
  return (
    <div
      aria-hidden
      className="flex h-8 min-w-0 items-center px-3 @xl:h-auto @xl:px-0"
    >
      <div className="flex size-10 shrink-0 items-center justify-center @xl:hidden">
        <ArrowDown className="size-4 shrink-0 stroke-ink-ghost" />
      </div>
      <div className="hidden min-w-0 flex-1 items-center @xl:flex">
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
