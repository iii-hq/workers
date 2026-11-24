import { cn } from '@/lib/utils'

export interface MemoryListItem {
  key: string
  text: string
  /** When the fact was last stated, `YYYY-MM-DD`. */
  date: string | null
  pinned?: boolean
}

/**
 * Recalled memories as a reader takes them in: the fact in sans, the day it
 * was last stated in mono at the right edge, pinned ones marked there too.
 * The one list behind the reply's memory chip and the timeline's memory
 * note, so both read the same.
 */
export function MemoryList({
  items,
  className,
}: {
  items: MemoryListItem[]
  className?: string
}) {
  // Below 24rem the date drops under its fact instead of squeezing it.
  return (
    <div className={cn('@container', className)}>
      <ul data-memory-list className="flex flex-col gap-3">
        {items.map((item) => (
          <li
            key={item.key}
            className="grid grid-cols-1 items-baseline gap-x-4 @sm:grid-cols-[minmax(0,1fr)_auto]"
          >
            <p className="text-pretty wrap-break-word font-sans text-[13px] leading-5 text-ink">
              {item.text}
            </p>
            <span className="font-mono text-meta leading-5 tabular-nums text-ink-ghost">
              {item.pinned ? (
                <span data-pinned>pinned{item.date ? ' · ' : ''}</span>
              ) : null}
              {item.date ? <time dateTime={item.date}>{item.date}</time> : null}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}
