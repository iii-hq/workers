import { ChevronDown, ChevronRight } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'

interface ConversationRowProps {
  conversation: Conversation
  active: boolean
  onSelect: () => void
  onRename: (title: string) => void
  onRemove: () => void
  /** Tree nesting level; 0 = root. Controls left indent. */
  depth?: number
  /** True when this row has nested sub-agent rows (renders a toggle caret). */
  hasChildren?: boolean
  /** True when this row's subtree is currently collapsed. */
  treeCollapsed?: boolean
  /** Toggle this row's subtree (only wired when `hasChildren`). */
  onToggleTree?: () => void
}

/** px of indent per tree level. */
const INDENT_STEP = 12

function formatRelative(ts: number): string {
  const delta = Date.now() - ts
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h`
  return `${Math.floor(delta / 86_400_000)}d`
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

  /* Only flat lists (every row depth 0, childless) render with no caret slot,
     so the common case looks exactly as before. Any tree row reserves the
     caret column so titles line up across siblings with and without carets. */
  const inTree = hasChildren || depth > 0

  return (
    // biome-ignore lint/a11y/useSemanticElements: row hosts a nested delete <button>; using a real <button> here would nest interactive elements.
    <div
      role="button"
      tabIndex={editing ? -1 : 0}
      aria-current={active ? 'page' : undefined}
      aria-label={`open ${conversation.title}`}
      className={cn(
        'group relative flex items-center gap-2 pl-3 pr-2 py-2 cursor-pointer transition-colors',
        active
          ? 'bg-panel border-l-2 border-l-accent pl-[10px]'
          : 'border-l-2 border-l-transparent hover:bg-paper-2',
      )}
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
      {depth > 0 ? (
        <span
          aria-hidden
          className="shrink-0"
          style={{ width: depth * INDENT_STEP }}
        />
      ) : null}
      {inTree ? (
        hasChildren ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onToggleTree?.()
            }}
            aria-label={
              treeCollapsed ? 'expand sub-agents' : 'collapse sub-agents'
            }
            aria-expanded={!treeCollapsed}
            className="flex items-center justify-center size-4 shrink-0 text-ink-faint hover:text-ink transition-colors"
          >
            {treeCollapsed ? (
              <ChevronRight className="size-3.5" />
            ) : (
              <ChevronDown className="size-3.5" />
            )}
          </button>
        ) : (
          <span aria-hidden className="size-4 shrink-0" />
        )
      ) : null}
      <div className="flex-1 min-w-0">
        {editing ? (
          <input
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
            className="w-full bg-bg border border-ink px-1 py-0.5 font-mono text-[13px] text-ink outline-none lowercase"
          />
        ) : (
          <div
            className={cn(
              'font-mono text-[13px] truncate lowercase',
              active ? 'text-ink' : 'text-ink-faint',
            )}
          >
            {conversation.title}
          </div>
        )}
      </div>
      {conversation.status === 'working' ? (
        <StatusDot tone="accent" pulse title="working" className="shrink-0" />
      ) : conversation.status === 'error' ? (
        <StatusDot
          tone="alert"
          title={conversation.statusReason ?? 'error'}
          className="shrink-0"
        />
      ) : null}
      <span className="font-mono text-[11px] text-ink-ghost tabular-nums shrink-0">
        {formatRelative(conversation.updatedAt)}
      </span>
      {inTree ? (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            window.location.hash = `#/timeline/${encodeURIComponent(conversation.id)}`
          }}
          aria-label={`open timeline for ${conversation.title}`}
          title="open family timeline"
          className="opacity-0 group-hover:opacity-100 text-ink-faint hover:text-accent transition-opacity transition-colors font-mono text-[11px] leading-none px-1"
        >
          ⇅
        </button>
      ) : null}
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation()
          onRemove()
        }}
        aria-label={`delete ${conversation.title}`}
        className="opacity-0 group-hover:opacity-100 text-ink-faint hover:text-accent transition-opacity transition-colors font-mono text-[14px] leading-none px-1"
      >
        ×
      </button>
    </div>
  )
}
