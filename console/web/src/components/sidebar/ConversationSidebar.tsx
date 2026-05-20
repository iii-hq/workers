import { PanelLeftClose, PanelLeftOpen, Plus } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ConversationRow } from './ConversationRow'

interface ConversationSidebarProps {
  conversations: Conversation[]
  activeId: string | null
  collapsed?: boolean
  onToggleCollapsed?: () => void
  density?: 'route' | 'dock'
  onCreate: () => void
  onSelect: (id: string) => void
  onRename: (id: string, title: string) => void
  onRemove: (id: string) => void
}

export function ConversationSidebar({
  conversations,
  activeId,
  collapsed = false,
  onToggleCollapsed,
  density = 'route',
  onCreate,
  onSelect,
  onRename,
  onRemove,
}: ConversationSidebarProps) {
  if (collapsed) {
    return (
      <aside className="w-9 shrink-0 border-r border-rule flex flex-col items-center bg-bg gap-1 py-2">
        <button
          type="button"
          onClick={onToggleCollapsed}
          aria-label="expand conversations"
          title="expand conversations"
          className="flex items-center justify-center size-7 text-ink-faint hover:text-ink transition-colors"
        >
          <PanelLeftOpen className="size-4" />
        </button>
        <button
          type="button"
          onClick={onCreate}
          aria-label="new chat"
          title="new chat"
          className="flex items-center justify-center size-7 text-ink-faint hover:text-accent transition-colors"
        >
          <Plus className="size-4" />
        </button>
      </aside>
    )
  }

  const widthClass = density === 'dock' ? 'w-[220px]' : 'w-[260px]'

  return (
    <aside
      className={cn(
        'shrink-0 border-r border-rule flex flex-col bg-bg',
        widthClass,
      )}
    >
      <div className="px-3 py-3 border-b border-rule flex items-center gap-2">
        <Button
          type="button"
          variant="primary"
          size="sm"
          className="flex-1 justify-start"
          onClick={onCreate}
        >
          <span aria-hidden className="text-accent">
            $
          </span>
          new chat
        </Button>
        {onToggleCollapsed ? (
          <button
            type="button"
            onClick={onToggleCollapsed}
            aria-label="collapse conversations"
            title="collapse conversations"
            className="flex items-center justify-center size-7 text-ink-faint hover:text-ink transition-colors flex-shrink-0"
          >
            <PanelLeftClose className="size-4" />
          </button>
        ) : null}
      </div>

      <div className="px-3 py-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
          conversations
        </div>
      </div>

      <div className="flex-1 overflow-y-auto divide-y divide-rule-2">
        {conversations.length === 0 ? (
          <div className="px-3 py-6 font-mono text-[12px] text-ink-ghost lowercase">
            no conversations yet. start one above.
          </div>
        ) : (
          conversations.map((c) => (
            <ConversationRow
              key={c.id}
              conversation={c}
              active={c.id === activeId}
              onSelect={() => onSelect(c.id)}
              onRename={(title) => onRename(c.id, title)}
              onRemove={() => onRemove(c.id)}
            />
          ))
        )}
      </div>
    </aside>
  )
}
