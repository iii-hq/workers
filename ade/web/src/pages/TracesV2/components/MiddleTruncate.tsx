import { useMemo } from 'react'
import { cn } from '@/lib/utils'
import { splitForMiddleTruncation } from '../lib/middleTruncate'

/**
 * A single-line label that drops its MIDDLE when it doesn't fit, Finder
 * style: `iii-dir…search_functions`. Only the head shrinks, ellipsizing at
 * its end; the tail never shrinks (a sub-pixel share of the squeeze would
 * already ellipsize it) but is capped at the box's width, so once the head
 * is gone a very narrow box still ends the tail in a clean ellipsis. When
 * the label fits, head and tail sit flush and read as one name. Type,
 * colour and weight come from `className` / the parent.
 */
export function MiddleTruncate({
  text,
  className,
  title,
}: {
  text: string
  className?: string
  title?: string
}) {
  const { head, tail } = useMemo(() => splitForMiddleTruncation(text), [text])
  return (
    <span
      className={cn(
        'flex min-w-0 overflow-hidden whitespace-nowrap',
        className,
      )}
      title={title}
    >
      <span className="min-w-0 overflow-hidden text-ellipsis">{head}</span>
      {tail ? (
        <span className="max-w-full shrink-0 overflow-hidden text-ellipsis">
          {tail}
        </span>
      ) : null}
    </span>
  )
}
