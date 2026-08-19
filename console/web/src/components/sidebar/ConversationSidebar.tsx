import { useCallback, useEffect, useMemo, useState } from 'react'
import { Input } from '@/components/ui/Input'
import { filterConversations } from '@/lib/conversation-filter'
import {
  buildConversationTree,
  flattenConversationTree,
} from '@/lib/conversation-tree'
import type { Conversation } from '@/types/chat'
import { ConversationRow } from './ConversationRow'

interface ConversationSidebarProps {
  conversations: Conversation[]
  activeId: string | null
  /** Larger row actions when the list is the whole narrow page. */
  narrow?: boolean
  onSelect: (id: string) => void
  onRename: (id: string, title: string) => void
  onRemove: (id: string) => void
}

const TREE_COLLAPSED_KEY = 'iii-chat-tree-collapsed'

function loadCollapsedNodes(): Set<string> {
  if (typeof window === 'undefined') return new Set()
  try {
    const raw = window.localStorage.getItem(TREE_COLLAPSED_KEY)
    if (!raw) return new Set()
    const arr = JSON.parse(raw)
    return Array.isArray(arr)
      ? new Set(arr.filter((x): x is string => typeof x === 'string'))
      : new Set()
  } catch {
    return new Set()
  }
}

function persistCollapsedNodes(set: Set<string>): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(TREE_COLLAPSED_KEY, JSON.stringify([...set]))
  } catch {
    // best-effort persistence
  }
}

export function ConversationSidebar({
  conversations,
  activeId,
  narrow = false,
  onSelect,
  onRename,
  onRemove,
}: ConversationSidebarProps) {
  /* Per-node tree collapse state, persisted so a folded sub-agent subtree
     stays folded across reloads. Default expanded. */
  const [collapsedNodes, setCollapsedNodes] =
    useState<Set<string>>(loadCollapsedNodes)
  useEffect(() => {
    persistCollapsedNodes(collapsedNodes)
  }, [collapsedNodes])

  const toggleNode = useCallback((id: string) => {
    setCollapsedNodes((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const [query, setQuery] = useState('')

  const rows = useMemo(() => {
    const q = query.trim()
    if (q) {
      /* Flat matches while searching: a matched child under an unmatched
         parent has no tree context to render, so no indent/caret. */
      return filterConversations(conversations, q).map((conversation) => ({
        conversation,
        depth: 0,
        hasChildren: false,
      }))
    }
    return flattenConversationTree(
      buildConversationTree(conversations),
      collapsedNodes,
    )
  }, [conversations, collapsedNodes, query])

  return (
    <>
      <div className="space-y-2 px-3 py-2">
        <h2 className="font-sans text-base font-medium text-ink sm:text-sm">
          Conversations
        </h2>
        <Input
          name="conversation-search"
          type="search"
          value={query}
          onChange={setQuery}
          placeholder="Search conversations"
          aria-label="search conversations"
          className="h-12 font-sans text-base normal-case sm:h-9 sm:text-sm"
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('')
          }}
        />
      </div>

      <div className="flex-1 space-y-px overflow-y-auto px-2 py-1">
        {rows.length === 0 ? (
          <div className="px-3 py-6 font-sans text-base text-ink-ghost sm:text-sm">
            {query.trim()
              ? 'No matches.'
              : 'No conversations yet. Start one above.'}
          </div>
        ) : (
          rows.map(({ conversation: c, depth, hasChildren }) => (
            <ConversationRow
              key={c.id}
              conversation={c}
              active={c.id === activeId}
              narrow={narrow}
              depth={depth}
              hasChildren={hasChildren}
              treeCollapsed={collapsedNodes.has(c.id)}
              onToggleTree={() => toggleNode(c.id)}
              onSelect={() => onSelect(c.id)}
              onRename={(title) => onRename(c.id, title)}
              onRemove={() => onRemove(c.id)}
            />
          ))
        )}
      </div>
    </>
  )
}
