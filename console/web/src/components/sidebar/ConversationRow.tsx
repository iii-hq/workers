import { Bot, ChevronDown, ChevronRight, Trash2, Zap } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'

interface ConversationRowProps {
  conversation: Conversation
  active: boolean
  /** Larger row actions when the sidebar is the whole narrow page. */
  narrow?: boolean
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
  narrow = false,
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
        'group relative flex cursor-pointer items-center gap-2 rounded-sm pr-1 pl-2',
        narrow ? 'min-h-12 py-1.5' : 'py-1',
        active ? 'bg-surface-selected' : 'hover:bg-surface-hover',
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
            className={cn(
              'relative flex shrink-0 items-center justify-center text-ink-faint hover:text-ink',
              narrow ? 'size-10' : 'size-4',
            )}
          >
            <span
              className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            {treeCollapsed ? (
              <ChevronRight className="size-4 shrink-0" aria-hidden />
            ) : (
              <ChevronDown className="size-4 shrink-0" aria-hidden />
            )}
          </button>
        ) : (
          <span
            aria-hidden
            className={cn('shrink-0', narrow ? 'size-10' : 'size-4')}
          />
        )
      ) : null}
      {/* Sub-agent origin: ⚡ a trigger reaction spawned it, 🤖 an agent's
          direct harness::spawn did. Absent on roots and pre-stamp sessions. */}
      {depth > 0 && conversation.spawnedBy ? (
        <span
          className="flex items-center shrink-0 text-ink-ghost"
          title={
            conversation.spawnedBy === 'trigger'
              ? 'spawned by a trigger'
              : 'spawned by an agent'
          }
        >
          {conversation.spawnedBy === 'trigger' ? (
            <Zap
              aria-label="spawned by a trigger"
              className="size-4 shrink-0"
            />
          ) : (
            <Bot aria-label="spawned by an agent" className="size-4 shrink-0" />
          )}
        </span>
      ) : null}
      <div className="flex-1 min-w-0">
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
            className="w-full rounded-xs bg-surface px-1 py-1 font-sans text-base text-ink outline-none sm:text-[13px]"
          />
        ) : (
          <div
            className={cn(
              'truncate font-sans text-base sm:text-[13px]',
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
      <span className="shrink-0 font-sans text-sm text-ink-ghost tabular-nums sm:text-[11px]">
        {formatRelative(conversation.updatedAt)}
      </span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation()
          onRemove()
        }}
        aria-label={`delete ${conversation.title}`}
        className={cn(
          'relative flex shrink-0 items-center justify-center text-ink-faint hover:text-alert focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          narrow
            ? 'size-10 opacity-100'
            : 'size-7 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100',
        )}
      >
        <span
          className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
          aria-hidden="true"
        />
        <Trash2 className="size-4 shrink-0" aria-hidden />
      </button>
    </div>
  )
}
