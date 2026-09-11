import { Clock3 } from 'lucide-react'
import type * as React from 'react'
import { formatElapsed } from '@/lib/relative-time'
import { cn } from '@/lib/utils'

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
