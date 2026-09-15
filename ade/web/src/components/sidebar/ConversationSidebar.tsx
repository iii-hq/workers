import uiClasses from '@iii-dev/console-ui/ui-classes'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Input } from '@/components/ui/Input'
import { StatusPanel } from '@/components/ui/StatusPanel'
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
  /** Touch-sized rows and always-visible actions when the list is the whole narrow page. */
  narrow?: boolean
  onSelect: (id: string) => void
  /** Resolves the reason when the store refused the new title (see the hook). */
  onRename: (id: string, title: string) => Promise<string | null>
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

  /* A refused rename is reverted by the hook, so without this line the row
     would just snap back to its old name with no reason given. */
  const [renameError, setRenameError] = useState<string | null>(null)
  const handleRename = useCallback(
    (id: string, title: string) => {
      setRenameError(null)
      void onRename(id, title).then((reason) => {
        if (reason) setRenameError(reason)
      })
    },
    [onRename],
  )

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

  /* The 8px gutter plus the tree row's own 10px inset puts every glyph,
     the heading and the empty copy on one 18px column. */
  return (
    <>
      <div className="flex flex-col gap-2 px-2 pt-2 pb-1">
        <h2 className="px-[10px] font-sans text-base font-medium text-ink-faint sm:text-[13px]">
          Conversations
        </h2>
        <Input
          name="conversation-search"
          type="search"
          value={query}
          onChange={setQuery}
          placeholder="Search conversations"
          aria-label="search conversations"
          data-conversation-search=""
          className="h-12 font-sans text-base normal-case sm:h-8 sm:text-[13px]"
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('')
          }}
        />
      </div>

      {renameError ? (
        <div role="alert" className="px-2 pb-1">
          <StatusPanel
            variant="alert"
            headline="Title not saved"
            detail={renameError}
            className="px-2.5 py-2"
          />
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-1">
        {rows.length === 0 ? (
          <div className="px-[10px] py-6 font-sans text-base text-ink-ghost sm:text-[13px]">
            {query.trim()
              ? 'No matches.'
              : 'No conversations yet. Start one above.'}
          </div>
        ) : (
          <div className={uiClasses.tree} data-narrow={narrow || undefined}>
            {rows.map(({ conversation: c, depth, hasChildren }) => (
              <ConversationRow
                key={c.id}
                conversation={c}
                active={c.id === activeId}
                depth={depth}
                hasChildren={hasChildren}
                treeCollapsed={collapsedNodes.has(c.id)}
                onToggleTree={() => toggleNode(c.id)}
                onSelect={() => onSelect(c.id)}
                onRename={(title) => handleRename(c.id, title)}
                onRemove={() => onRemove(c.id)}
              />
            ))}
          </div>
        )}
      </div>
    </>
  )
}
