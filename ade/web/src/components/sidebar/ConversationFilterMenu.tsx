import { SlidersVertical } from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { IconButton } from '@/components/ui/IconButton'
import {
  CONVERSATION_GROUPING_OPTIONS,
  isConversationGrouping,
} from '@/lib/conversation-groups'
import {
  CONVERSATION_KIND_OPTIONS,
  CONVERSATION_SORT_OPTIONS,
  type ConversationListView,
  DEFAULT_CONVERSATION_LIST_VIEW,
  describeConversationListView,
  isConversationSort,
  isDefaultConversationListView,
  kindsLabel,
  toggleKind,
} from '@/lib/conversation-view'
import { cn } from '@/lib/utils'

interface ConversationFilterMenuProps {
  view: ConversationListView
  onViewChange: (next: ConversationListView) => void
  /** Touch-sized menu rows when the list is the whole narrow page. */
  narrow?: boolean
}

interface SettingProps {
  view: ConversationListView
  onViewChange: (next: ConversationListView) => void
  /** Classes for each choice row (touch height when narrow). */
  className: string
}

/** A parent row: the setting's name, then its current value on the trailing edge. */
function Setting({ label, value }: { label: string; value: string }) {
  return (
    <>
      <span>{label}</span>
      <span className="ml-auto truncate text-ink-faint">{value}</span>
    </>
  )
}

function KindChoices({ view, onViewChange, className }: SettingProps) {
  return CONVERSATION_KIND_OPTIONS.map((option) => (
    <DropdownMenuCheckboxItem
      key={option.value}
      checked={view.kinds.includes(option.value)}
      onCheckedChange={(checked) =>
        onViewChange(toggleKind(view, option.value, checked === true))
      }
      /* Multi-select: the menu stays open across toggles. */
      onSelect={(e) => e.preventDefault()}
      className={className}
    >
      {option.label}
    </DropdownMenuCheckboxItem>
  ))
}

function GroupingChoices({ view, onViewChange, className }: SettingProps) {
  return (
    <DropdownMenuRadioGroup
      value={view.grouping}
      onValueChange={(value) => {
        if (isConversationGrouping(value))
          onViewChange({ ...view, grouping: value })
      }}
    >
      {CONVERSATION_GROUPING_OPTIONS.map((option) => (
        <DropdownMenuRadioItem
          key={option.value}
          value={option.value}
          className={className}
        >
          {option.label}
        </DropdownMenuRadioItem>
      ))}
    </DropdownMenuRadioGroup>
  )
}

function SortChoices({ view, onViewChange, className }: SettingProps) {
  return (
    <DropdownMenuRadioGroup
      value={view.sort}
      onValueChange={(value) => {
        if (isConversationSort(value)) onViewChange({ ...view, sort: value })
      }}
    >
      {CONVERSATION_SORT_OPTIONS.map((option) => (
        <DropdownMenuRadioItem
          key={option.value}
          value={option.value}
          className={className}
        >
          {option.label}
        </DropdownMenuRadioItem>
      ))}
    </DropdownMenuRadioGroup>
  )
}

/**
 * The sidebar's quiet filter affordance: one 16px glyph beside the heading
 * that opens the list's view options — Type (multi-select), Group by and
 * Sort by (single choice), then Clear filters. The trigger wears a dot
 * while any setting differs from its default, so a hidden kind is never a
 * surprise.
 *
 * In a wide pane each setting is a row that pushes into a submenu, showing
 * its current value. On a narrow page there is no room beside the menu for
 * a submenu (Radix flips it off-screen), so the same choices are laid out
 * inline as three labelled sections in one scrolling menu.
 */
export function ConversationFilterMenu({
  view,
  onViewChange,
  narrow = false,
}: ConversationFilterMenuProps) {
  const filtering = !isDefaultConversationListView(view)
  const grouping = CONVERSATION_GROUPING_OPTIONS.find(
    (option) => option.value === view.grouping,
  )
  const sort = CONVERSATION_SORT_OPTIONS.find(
    (option) => option.value === view.sort,
  )
  /* Tighter than the stock 28px item: these are short single-line choices,
     and the whole menu should read at a glance. Narrow rows stay 40px, tall
     enough for a thumb without stretching twelve rows past the viewport. */
  const rowClass = cn('py-1', narrow && 'min-h-10')
  const choices = { view, onViewChange, className: rowClass }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <IconButton
          label="Conversation view options"
          tooltip={describeConversationListView(view)}
          data-filtering={filtering || undefined}
          className={cn(
            'relative shrink-0',
            narrow && 'size-11',
            filtering && 'text-ink',
          )}
        >
          <SlidersVertical aria-hidden />
          {/* A badge, not a wash: a filled button in the header reads as a
              stuck hover. The dot says "something is hidden" and the ink
              glyph says it is on; the tooltip says what. */}
          {filtering ? (
            <span
              aria-hidden
              className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-ink"
            />
          ) : null}
        </IconButton>
      </DropdownMenuTrigger>

      <DropdownMenuContent
        align="end"
        className={cn(
          'min-w-[12rem]',
          narrow &&
            'max-h-[var(--radix-dropdown-menu-content-available-height)] w-[min(20rem,calc(100vw-1rem))] overflow-y-auto',
        )}
      >
        {narrow ? (
          <>
            <DropdownMenuLabel>Type</DropdownMenuLabel>
            <KindChoices {...choices} />
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Group by</DropdownMenuLabel>
            <GroupingChoices {...choices} />
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Sort by</DropdownMenuLabel>
            <SortChoices {...choices} />
          </>
        ) : (
          <>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger className={rowClass}>
                <Setting label="Type" value={kindsLabel(view.kinds)} />
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="min-w-[9rem]">
                <KindChoices {...choices} />
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            {/* What is listed, then how it is arranged: two questions, two blocks. */}
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <DropdownMenuSubTrigger className={rowClass}>
                <Setting
                  label="Group by"
                  value={grouping?.label ?? view.grouping}
                />
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="min-w-[9rem]">
                <GroupingChoices {...choices} />
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger className={rowClass}>
                <Setting label="Sort by" value={sort?.label ?? view.sort} />
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="min-w-[9rem]">
                <SortChoices {...choices} />
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          </>
        )}

        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={!filtering}
          onSelect={() => onViewChange(DEFAULT_CONVERSATION_LIST_VIEW)}
          className={rowClass}
        >
          Clear filters
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
