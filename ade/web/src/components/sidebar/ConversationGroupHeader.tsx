import { ChevronRight } from 'lucide-react'
import { cn } from '@/lib/utils'

interface ConversationGroupHeaderProps {
  label: string
  /** Full project path behind a shortened label. */
  title?: string
  /** Sessions in the section, sub-agents included. */
  count: number
  collapsed: boolean
  onToggle: () => void
  /** Touch-sized header when the list is the whole narrow page. */
  narrow?: boolean
}

/**
 * A section label in the conversation tree: quiet ghost ink, the disclosure
 * caret right after the label the way a tree row carries it, and the section
 * count on the trailing edge. Reads as chrome rather than as another row — it
 * is shorter than the 28px row and never takes the selected wash.
 */
export function ConversationGroupHeader({
  label,
  title,
  count,
  collapsed,
  onToggle,
  narrow = false,
}: ConversationGroupHeaderProps) {
  return (
    <button
      type="button"
      aria-expanded={!collapsed}
      title={title}
      onClick={onToggle}
      className={cn(
        'iii-ui-motion-control mt-2 flex w-full min-w-0 items-center rounded-sm border-0 bg-transparent pr-3 pl-[10px] text-left font-sans font-medium text-ink-ghost transition-colors first:mt-0 hover:bg-surface-hover hover:text-ink-faint focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ink/55',
        narrow ? 'min-h-11 text-[13px]' : 'min-h-6 text-[11px]',
      )}
    >
      <span className="truncate">{label}</span>
      <ChevronRight
        aria-hidden
        className={cn(
          'iii-ui-motion-control ml-0.5 size-4 shrink-0 transition-transform',
          !collapsed && 'rotate-90',
        )}
      />
      <span
        aria-hidden
        className="ml-auto pl-2 font-normal text-[11px] text-ink-ghost tabular-nums"
      >
        {count}
      </span>
    </button>
  )
}
