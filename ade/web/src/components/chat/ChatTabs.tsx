import { History, Plus, X } from 'lucide-react'
import type { KeyboardEvent } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { Hint } from '@/components/ui/Hint'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'

/** How many chats the history menu lists beyond the open tabs. */
const HISTORY_LIMIT = 12

const ICON_BUTTON =
  'flex size-7 shrink-0 items-center justify-center rounded-md text-ink-faint transition-colors duration-[var(--motion-duration-fast)] hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus'

function relative(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000))
  if (s < 60) return 'now'
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  return `${Math.floor(s / 86400)}d`
}

function StatusMark({ conversation }: { conversation: Conversation }) {
  if (conversation.status === 'working') {
    return <StatusDot tone="accent" pulse title="working" />
  }
  if (conversation.status === 'error') {
    return (
      <StatusDot tone="alert" title={conversation.statusReason ?? 'error'} />
    )
  }
  return null
}

/**
 * The chat header's open chats, an editor's tab row: switch with a click,
 * close with the ✕ that shows on hover (or a middle click), + for a new
 * chat, and the history menu for one that isn't open. A chat that is
 * working leads its title with the pulsing dot, as in the sidebar.
 */
export function ChatTabs({
  tabs,
  activeId,
  recent,
  onSelect,
  onClose,
  onNew,
}: {
  /** The open chats, left to right. */
  tabs: readonly Conversation[]
  activeId: string | null
  /** Every chat, newest first, for the history menu (open ones are skipped). */
  recent: readonly Conversation[]
  onSelect: (id: string) => void
  onClose: (id: string) => void
  onNew: () => void
}) {
  const closable = tabs.length > 1
  const open = new Set(tabs.map((tab) => tab.id))
  const history = recent.filter((c) => !open.has(c.id)).slice(0, HISTORY_LIMIT)

  // Arrows step through the row, the way a tablist does.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    const index = tabs.findIndex((tab) => tab.id === activeId)
    const next = tabs[index + (event.key === 'ArrowRight' ? 1 : -1)]
    if (!next) return
    event.preventDefault()
    onSelect(next.id)
    event.currentTarget
      .querySelector<HTMLElement>(`[data-chat-tab="${next.id}"]`)
      ?.focus()
  }

  return (
    <div className="chat-tabs flex min-w-0 flex-1 items-center gap-1">
      <div
        role="tablist"
        aria-label="Open chats"
        // Sized to its tabs (shrinking, then scrolling, when they run out of
        // room), so + rides right after the last tab, as in an editor.
        className="flex min-w-0 items-center gap-0.5 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        onKeyDown={onKeyDown}
      >
        {tabs.map((tab) => {
          const active = tab.id === activeId
          const title = tab.title || 'New chat'
          return (
            <div
              key={tab.id}
              className={cn(
                'chat-tab group/tab relative flex h-7 min-w-[4.5rem] max-w-52 shrink items-center rounded-md transition-colors duration-[var(--motion-duration-fast)]',
                active
                  ? 'bg-surface-selected text-ink'
                  : 'text-ink-faint hover:bg-surface-hover hover:text-ink',
              )}
            >
              <button
                type="button"
                role="tab"
                aria-selected={active}
                tabIndex={active ? 0 : -1}
                data-chat-tab={tab.id}
                title={title}
                onClick={() => onSelect(tab.id)}
                onMouseDown={(event) => {
                  // Middle click closes, like every tab row.
                  if (event.button === 1 && closable) {
                    event.preventDefault()
                    onClose(tab.id)
                  }
                }}
                className={cn(
                  'flex h-full min-w-0 flex-1 items-center gap-2 rounded-md px-2.5 text-left font-sans text-[13px] font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
                  closable && 'chat-tab-label',
                )}
              >
                <StatusMark conversation={tab} />
                <span className="min-w-0 truncate">{title}</span>
              </button>
              {closable ? (
                // The ✕ lies over the end of the title only while the tab is
                // hovered (or focused); the label fades out under it, so a
                // resting tab spends no width on it.
                <button
                  type="button"
                  aria-label={`close ${title}`}
                  onClick={() => onClose(tab.id)}
                  className="absolute inset-y-0 right-0 flex items-center pr-1 pl-1.5 opacity-0 transition-opacity duration-[var(--motion-duration-fast)] focus-visible:opacity-100 focus-visible:outline-none group-hover/tab:opacity-100 pointer-coarse:opacity-100 [&:focus-visible>span]:ring-2 [&:focus-visible>span]:ring-rule-focus"
                >
                  <span className="flex size-5 items-center justify-center rounded-sm text-ink-faint hover:bg-surface-hover hover:text-ink">
                    <X aria-hidden className="size-4" />
                  </span>
                </button>
              ) : null}
            </div>
          )
        })}
      </div>
      <Hint label="New chat">
        <button
          type="button"
          aria-label="new chat"
          onClick={onNew}
          className={ICON_BUTTON}
        >
          <Plus aria-hidden className="size-4" />
        </button>
      </Hint>
      <DropdownMenu>
        <Hint label="Recent chats">
          {/* History keeps the row's far end, away from the tabs. */}
          <DropdownMenuTrigger
            aria-label="recent chats"
            className={cn(ICON_BUTTON, 'ml-auto')}
          >
            <History aria-hidden className="size-4" />
          </DropdownMenuTrigger>
        </Hint>
        <DropdownMenuContent align="end" sideOffset={6} className="w-72 p-1.5">
          <DropdownMenuGroup>
            <DropdownMenuLabel className="px-2.5 text-xs text-ink-faint">
              Recent chats
            </DropdownMenuLabel>
            {history.length === 0 ? (
              <div className="px-2.5 py-2 text-sm text-ink-faint">
                Every recent chat is open.
              </div>
            ) : (
              history.map((conversation) => (
                <DropdownMenuItem
                  key={conversation.id}
                  className="gap-2.5 rounded-lg px-2.5 py-2 text-sm"
                  onClick={() => onSelect(conversation.id)}
                >
                  <StatusMark conversation={conversation} />
                  <span className="min-w-0 flex-1 truncate text-ink">
                    {conversation.title || 'New chat'}
                  </span>
                  <span className="shrink-0 text-xs text-ink-faint tabular-nums">
                    {relative(conversation.updatedAt)}
                  </span>
                </DropdownMenuItem>
              ))
            )}
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
