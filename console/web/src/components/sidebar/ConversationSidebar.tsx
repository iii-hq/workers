import { PanelLeftClose, PanelLeftOpen, Plus } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import {
  clampSidebarWidth,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from '@/hooks/use-sidebar-width'
import { filterConversations } from '@/lib/conversation-filter'
import {
  buildConversationTree,
  flattenConversationTree,
} from '@/lib/conversation-tree'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ConversationRow } from './ConversationRow'

interface ConversationSidebarProps {
  conversations: Conversation[]
  activeId: string | null
  collapsed?: boolean
  onToggleCollapsed?: () => void
  /** Current panel width (px). Optional — defaults to the standard width. */
  width?: number
  /** Wire to enable drag-resize; omit to render a fixed-width panel. */
  onWidthChange?: (next: number) => void
  /**
   * Drill-in mode (narrow hosts): the list fills the pane instead of
   * rendering as a fixed-width rail. Pair with omitted collapse/resize
   * wiring — neither affordance makes sense when the list IS the page.
   */
  narrow?: boolean
  onCreate: () => void
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
  collapsed = false,
  onToggleCollapsed,
  width = SIDEBAR_DEFAULT_WIDTH,
  onWidthChange,
  narrow = false,
  onCreate,
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

  /* Drag-resize, lifted from ChatDock: anchored left, drag right = wider.
     Only active when a width setter is wired. */
  const [isResizing, setIsResizing] = useState(false)
  const resizeStartRef = useRef({ x: 0, width })

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      resizeStartRef.current = { x: e.clientX, width }
      setIsResizing(true)
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
    },
    [width],
  )

  useEffect(() => {
    if (!isResizing || !onWidthChange) return
    const onMove = (e: MouseEvent) => {
      const dx = e.clientX - resizeStartRef.current.x
      onWidthChange(clampSidebarWidth(resizeStartRef.current.width + dx))
    }
    const onUp = () => {
      setIsResizing(false)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
    return () => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
    }
  }, [isResizing, onWidthChange])

  const handleReset = useCallback(() => {
    onWidthChange?.(SIDEBAR_DEFAULT_WIDTH)
  }, [onWidthChange])

  if (collapsed) {
    return (
      <aside className="flex w-9 shrink-0 flex-col items-center gap-1 bg-sidebar py-2">
        <button
          type="button"
          onClick={onToggleCollapsed}
          aria-label="expand conversations"
          title="expand conversations"
          className="relative flex size-7 items-center justify-center text-ink-faint hover:text-ink"
        >
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <PanelLeftOpen className="size-4 shrink-0" aria-hidden />
        </button>
        <button
          type="button"
          onClick={onCreate}
          aria-label="new chat"
          title="new chat"
          className="relative flex size-7 items-center justify-center text-ink-faint hover:text-accent"
        >
          <span
            className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          />
          <Plus className="size-4 shrink-0" aria-hidden />
        </button>
      </aside>
    )
  }

  return (
    <>
      <aside
        style={narrow ? undefined : { width }}
        className={cn(
          'shrink-0 flex flex-col bg-sidebar',
          narrow && 'flex-1 min-w-0',
          isResizing && 'select-none',
        )}
      >
        <div className="px-3 py-3 flex items-center gap-2">
          <Button
            type="button"
            variant="primary"
            size="md"
            className="h-12 flex-1 justify-center px-3 font-sans text-base normal-case sm:h-9 sm:justify-start sm:text-sm"
            onClick={onCreate}
          >
            <Plus className="size-4 shrink-0" aria-hidden />
            New chat
          </Button>
          {onToggleCollapsed ? (
            <button
              type="button"
              onClick={onToggleCollapsed}
              aria-label="collapse conversations"
              title="collapse conversations"
              className="relative flex size-10 shrink-0 items-center justify-center text-ink-faint hover:text-ink sm:size-7"
            >
              <span
                className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                aria-hidden="true"
              />
              <PanelLeftClose className="size-4 shrink-0" aria-hidden />
            </button>
          ) : null}
        </div>

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
      </aside>
      {onWidthChange ? (
        // biome-ignore lint/a11y/useSemanticElements: drag handle is not a standard input; semantic separator + tabIndex is enough
        <div
          role="separator"
          aria-orientation="vertical"
          aria-valuenow={width}
          aria-valuemin={SIDEBAR_MIN_WIDTH}
          aria-valuemax={SIDEBAR_MAX_WIDTH}
          aria-label="resize conversations"
          tabIndex={0}
          onMouseDown={handleMouseDown}
          onDoubleClick={handleReset}
          className={cn(
            'w-[3px] flex-shrink-0 cursor-col-resize bg-transparent hover:bg-accent/60 active:bg-accent',
            isResizing && 'bg-accent',
          )}
        />
      ) : null}
    </>
  )
}
