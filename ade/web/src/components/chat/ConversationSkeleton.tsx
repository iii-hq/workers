import { Skeleton } from '@/components/shadcn/skeleton'
import { cn } from '@/lib/utils'

/**
 * A conversation while its history loads, drawn as the shape it is about to
 * take — a user bubble on the right, the assistant's reply as lines of prose
 * and a call card, then the next exchange — in the transcript's own 720px
 * column, so nothing jumps when the real messages land. The words are for
 * assistive tech only; sighted readers see the shimmer.
 */
export function ConversationSkeleton({ className }: { className?: string }) {
  return (
    <div
      aria-busy="true"
      className={cn(
        'mx-auto flex w-full max-w-[720px] flex-col gap-y-8 px-4 pt-6',
        className,
      )}
    >
      <span className="sr-only">Loading conversation…</span>
      <Exchange question="w-[46%]" lines={['w-full', 'w-[92%]', 'w-[64%]']} />
      <Exchange question="w-[34%]" lines={['w-[96%]', 'w-[78%]']} card />
      <Exchange question="w-[52%]" lines={['w-full', 'w-[58%]']} />
    </div>
  )
}

/** A step stronger than shadcn's `bg-muted` (5.5% ink), which all but vanishes on the light pane. */
const BAR = 'bg-surface-hover'

function Exchange({
  question,
  lines,
  card = false,
}: {
  /** Width of the user bubble. */
  question: string
  /** Widths of the reply's lines. */
  lines: string[]
  /** A call-card placeholder under the reply. */
  card?: boolean
}) {
  return (
    <div aria-hidden className="flex flex-col gap-y-4">
      <Skeleton className={cn('ml-auto h-10 rounded-2xl', BAR, question)} />
      <div className="flex flex-col gap-y-2.5">
        {lines.map((width, index) => (
          <Skeleton
            className={cn('h-4', BAR, width)}
            key={`${width}-${index}`}
          />
        ))}
      </div>
      {card ? <Skeleton className={cn('h-12 w-full rounded-xl', BAR)} /> : null}
    </div>
  )
}
