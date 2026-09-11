import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Bot, ChevronRight, MessageSquare, X } from 'lucide-react'
import type { CSSProperties } from 'react'
import { useEffect, useRef, useState } from 'react'
import { SUBAGENT_ICON_COMPONENTS } from '@/components/chat/ActiveSubagentChips'
import { StatusDot } from '@/components/ui/StatusDot'
import { TriggerIcon } from '@/components/ui/TriggerIcon'
import type { Conversation, SubagentColor } from '@/types/chat'

interface ConversationRowProps {
  conversation: Conversation
  active: boolean
  onSelect: () => void
  onRename: (title: string) => void
  onRemove: () => void
  /** Tree nesting level; 0 = root. Drives the shared tree indent. */
  depth?: number
  /** True when this row has nested sub-agent rows (renders a toggle caret). */
  hasChildren?: boolean
  /** True when this row's subtree is currently collapsed. */
  treeCollapsed?: boolean
  /** Toggle this row's subtree (only wired when `hasChildren`). */
  onToggleTree?: () => void
}

function formatRelative(ts: number): string {
  const delta = Date.now() - ts
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h`
  return `${Math.floor(delta / 86_400_000)}d`
}

interface RowGlyph {
  Icon: typeof Bot
  /** Glyph tone; `neutral`/absent keeps the ghost ink. */
  color?: SubagentColor
  /** Native tooltip naming the session's origin or profile. */
  title?: string
}

/**
 * Which mark opens the row. A selected agent profile owns the glyph at
 * every depth; a spawned child shows its origin (trigger bolt or the
 * spawning agent's appearance); a plain chat keeps a quiet message mark so
 * every label starts on the same column.
 */
function resolveGlyph(conversation: Conversation, depth: number): RowGlyph {
  const profile = conversation.agentProfile
  if (profile) {
    return {
      Icon: (profile.icon && SUBAGENT_ICON_COMPONENTS[profile.icon]) || Bot,
      color: profile.color,
      title: profile.name,
    }
  }
  if (depth > 0 && conversation.spawnedBy === 'trigger') {
    return { Icon: TriggerIcon, title: 'spawned by a trigger' }
  }
  if (depth > 0 && conversation.spawnedBy === 'agent') {
    const appearance = conversation.subagentAppearance
    return {
      Icon:
        (appearance?.icon && SUBAGENT_ICON_COMPONENTS[appearance.icon]) || Bot,
      color: appearance?.color,
      title: appearance?.name ?? 'spawned by an agent',
    }
  }
  return { Icon: MessageSquare }
}

export function ConversationRow({
  conversation,
  active,
  onSelect,
  onRename,
  onRemove,
  depth = 0,
  hasChildren = false,
  treeCollapsed = false,
  onToggleTree,
}: ConversationRowProps) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(conversation.title)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus()
      inputRef.current?.select()
    }
  }, [editing])

  useEffect(() => {
    if (!editing) setDraft(conversation.title)
  }, [conversation.title, editing])

  const commit = () => {
    setEditing(false)
    const next = draft.trim()
    if (next && next !== conversation.title) onRename(next)
  }

  const glyph = resolveGlyph(conversation, depth)

  return (
    // biome-ignore lint/a11y/useSemanticElements: row hosts nested caret/delete <button>s; using a real <button> here would nest interactive elements.
    <div
      role="button"
      tabIndex={editing ? -1 : 0}
      aria-current={active ? 'page' : undefined}
      aria-label={`open ${conversation.title}`}
      className={uiClasses.treeItem}
      style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
      onClick={() => !editing && onSelect()}
      onDoubleClick={() => setEditing(true)}
      onKeyDown={(e) => {
        if (editing) return
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSelect()
        } else if (e.key === 'F2') {
          e.preventDefault()
          setEditing(true)
        }
      }}
    >
      <span
        className={uiClasses.treeItemIcon}
        data-color={glyph.color}
        title={glyph.title}
      >
        <glyph.Icon aria-hidden />
      </span>
      {editing ? (
        <input
          name="conversation-title"
          aria-label="conversation title"
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.currentTarget.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commit()
            else if (e.key === 'Escape') {
              setEditing(false)
              setDraft(conversation.title)
            }
          }}
          className="min-w-0 flex-1 rounded-xs bg-surface px-1 py-0.5 font-sans text-base font-medium text-ink outline-none sm:text-[13px]"
        />
      ) : (
        <span className={uiClasses.treeItemLabel}>{conversation.title}</span>
      )}
      {hasChildren ? (
        <button
          type="button"
          className={uiClasses.treeItemCaret}
          aria-expanded={!treeCollapsed}
          aria-label={
            treeCollapsed ? 'expand sub-agents' : 'collapse sub-agents'
          }
          onClick={(e) => {
            e.stopPropagation()
            onToggleTree?.()
          }}
        >
          <ChevronRight aria-hidden />
        </button>
      ) : null}
      <span className={uiClasses.treeItemTrailing}>
        {conversation.status === 'working' ? (
          <StatusDot tone="accent" pulse title="working" />
        ) : conversation.status === 'error' ? (
          <StatusDot
            tone="alert"
            title={conversation.statusReason ?? 'error'}
          />
        ) : null}
        <span className={uiClasses.treeItemMeta}>
          {formatRelative(conversation.updatedAt)}
        </span>
        <button
          type="button"
          className={uiClasses.treeItemAction}
          data-tone="alert"
          aria-label={`delete ${conversation.title}`}
          onClick={(e) => {
            e.stopPropagation()
            onRemove()
          }}
        >
          <X aria-hidden />
        </button>
      </span>
    </div>
  )
}
