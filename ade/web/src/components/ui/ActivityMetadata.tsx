import { Clock3 } from 'lucide-react'
import type * as React from 'react'
import { formatElapsed } from '@/lib/relative-time'
import { cn } from '@/lib/utils'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

export interface ActivityMetadataProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children'> {
  createdAt?: number | null
  identifier?: string | null
  now?: number
}

export function ActivityMetadata({
  createdAt,
  identifier,
  now = Date.now(),
  className,
  ...props
}: ActivityMetadataProps) {
  const createdAge = formatElapsed(createdAt, now)
  if (!createdAge && !identifier) return null

  return (
    <div
      className={cn(
        'flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 font-sans text-base text-ink-ghost sm:text-xs',
        className,
      )}
      {...props}
    >
      {createdAge ? (
        <span className="inline-flex shrink-0 items-center gap-1.5 tabular-nums">
          <Clock3 aria-hidden className="size-5 h-lh shrink-0 sm:size-4" />
          <span>
            {createdAge === 'just now' ? createdAge : `${createdAge} ago`}
          </span>
        </span>
      ) : null}
      {createdAge && identifier ? <span aria-hidden>·</span> : null}
      {identifier ? (
        <span className="min-w-0 truncate font-mono" title={identifier}>
          ID: {compactActivityId(identifier)}
        </span>
      ) : null}
    </div>
  )
}

export function compactActivityId(id: string): string {
  if (id.length <= 22) return id
  return `${id.slice(0, 11)}…${id.slice(-8)}`
}

export interface MetaRowItem {
  label: React.ReactNode
  value: React.ReactNode
}

export interface MetaRowProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Label/value pairs, values in mono; free-form children (chips) follow. */
  items?: readonly MetaRowItem[]
}

/** The wrapping metadata strip a function-trigger card opens with. */
export function MetaRow({
  items,
  children,
  className,
  ...props
}: MetaRowProps) {
  return (
    <div
      className={cn(
        'flex flex-wrap items-center gap-1.5 border-b border-rule-2 bg-paper-2 px-3 py-1.5',
        className,
      )}
      {...props}
    >
      {items?.map((item, index) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: position is the identity
          key={index}
          className="inline-flex items-center gap-1 font-sans text-xs text-ink-faint"
        >
          <span>{item.label}</span>
          <span className="font-mono text-ink">{item.value}</span>
        </span>
      ))}
      {children}
    </div>
  )
}

export interface ActionLineProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Leading 16 px icon (a Lucide element). */
  icon: React.ReactNode
  tone?: 'accent' | 'warn' | 'ink'
}

const ACTION_TONE = {
  accent: 'text-accent',
  warn: 'text-warn',
  ink: 'text-ink',
} as const

/** One action a card reports (`→ url`, `ƒ function`): icon in the tone, body in ink. */
export function ActionLine({
  icon,
  tone = 'accent',
  children,
  className,
  ...props
}: ActionLineProps) {
  return (
    <div
      className={cn(
        'flex items-start gap-2 border-b border-rule-2 bg-bg px-3 py-2',
        className,
      )}
      {...props}
    >
      <span
        aria-hidden
        className={cn(
          'flex h-lh shrink-0 items-center [&>svg]:size-4',
          ACTION_TONE[tone],
        )}
      >
        {icon}
      </span>
      <div className="min-w-0 break-all text-ink">{children}</div>
    </div>
  )
}
