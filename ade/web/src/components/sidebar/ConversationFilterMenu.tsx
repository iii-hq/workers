import { SlidersVertical } from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { IconButton } from '@/components/ui/IconButton'
import {
  CONVERSATION_GROUPING_OPTIONS,
  type ConversationGrouping,
} from '@/lib/conversation-groups'
import { cn } from '@/lib/utils'

interface ConversationFilterMenuProps {
  grouping: ConversationGrouping
  onGroupingChange: (next: ConversationGrouping) => void
  /** Touch-sized menu rows when the list is the whole narrow page. */
  narrow?: boolean
}

/**
 * The sidebar's quiet filter affordance: one 16px glyph beside the heading
 * that opens the list's view options. Grouping is the only setting so far, so
 * it reads as a labelled section of checkable choices rather than a row that
 * pushes into a submenu — a binary choice should not cost two clicks.
 */
export function ConversationFilterMenu({
  grouping,
  onGroupingChange,
  narrow = false,
}: ConversationFilterMenuProps) {
  const active = CONVERSATION_GROUPING_OPTIONS.find(
    (option) => option.value === grouping,
  )

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <IconButton
          label="Conversation view options"
          tooltip={
            active ? `Grouped by ${active.label.toLowerCase()}` : 'View options'
          }
          className={cn('shrink-0', narrow && 'size-11')}
        >
          <SlidersVertical aria-hidden />
        </IconButton>
      </DropdownMenuTrigger>

      <DropdownMenuContent align="end" className="min-w-[13rem]">
        <DropdownMenuLabel>Group by</DropdownMenuLabel>
        {CONVERSATION_GROUPING_OPTIONS.map((option) => (
          <DropdownMenuCheckboxItem
            key={option.value}
            checked={option.value === grouping}
            /* Re-picking the active grouping must not clear it: the list is
               always grouped, so only a positive check changes anything. */
            onCheckedChange={(checked) => {
              if (checked) onGroupingChange(option.value)
            }}
            className={cn('flex-col items-start gap-0.5', narrow && 'min-h-11')}
          >
            <span className="text-ink">{option.label}</span>
            <span className="text-[11px] text-ink-ghost">
              {option.description}
            </span>
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
