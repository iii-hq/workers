import { Button } from '@/components/ui/Button'
import type { Conversation } from '@/types/chat'
import { ConversationRow } from './ConversationRow'

interface ConversationSidebarProps {
  conversations: Conversation[]
  activeId: string | null
  onCreate: () => void
  onSelect: (id: string) => void
  onRename: (id: string, title: string) => void
  onRemove: (id: string) => void
}

export function ConversationSidebar({
  conversations,
  activeId,
  onCreate,
  onSelect,
  onRename,
  onRemove,
}: ConversationSidebarProps) {
  return (
    <aside className="w-[260px] shrink-0 border-r border-rule flex flex-col bg-bg">
      <div className="px-3 py-3 border-b border-rule">
        <Button
          type="button"
          variant="primary"
          size="sm"
          className="w-full justify-start"
          onClick={onCreate}
        >
          <span aria-hidden className="text-accent">
            $
          </span>
          new chat
        </Button>
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
