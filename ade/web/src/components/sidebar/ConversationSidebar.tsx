import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Fragment, useCallback, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { filterConversations } from '@/lib/conversation-filter'
import {
  countConversations,
  groupConversationRoots,
} from '@/lib/conversation-groups'
import {
  buildConversationTree,
  type ConvNode,
  flattenConversationTree,
} from '@/lib/conversation-tree'
import {
  type ConversationListView,
  DEFAULT_CONVERSATION_LIST_VIEW,
  effectiveConversationKind,
  filterConversationsByKind,
  parseConversationListView,
  sortConversationRoots,
} from '@/lib/conversation-view'
import type { Conversation } from '@/types/chat'
import { ConversationFilterMenu } from './ConversationFilterMenu'
import { ConversationGroupHeader } from './ConversationGroupHeader'
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
const GROUPS_COLLAPSED_KEY = 'iii-chat-groups-collapsed'
const LIST_VIEW_KEY = 'iii-chat-list-view'
/** The grouping-only key this view record replaced; read as a fallback. */
const LEGACY_GROUPING_KEY = 'iii-chat-grouping'

function loadStringSet(storageKey: string): Set<string> {
  if (typeof window === 'undefined') return new Set()
  try {
    const raw = window.localStorage.getItem(storageKey)
    if (!raw) return new Set()
    const arr = JSON.parse(raw)
    return Array.isArray(arr)
      ? new Set(arr.filter((x): x is string => typeof x === 'string'))
      : new Set()
  } catch {
    return new Set()
  }
}

function persistStringSet(storageKey: string, set: Set<string>): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(storageKey, JSON.stringify([...set]))
  } catch {
    // best-effort persistence
  }
}

function loadListView(): ConversationListView {
  if (typeof window === 'undefined') return DEFAULT_CONVERSATION_LIST_VIEW
  try {
    const raw = window.localStorage.getItem(LIST_VIEW_KEY)
    return parseConversationListView(
      raw ? JSON.parse(raw) : undefined,
      window.localStorage.getItem(LEGACY_GROUPING_KEY),
    )
  } catch {
    return DEFAULT_CONVERSATION_LIST_VIEW
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
  const [collapsedNodes, setCollapsedNodes] = useState<Set<string>>(() =>
    loadStringSet(TREE_COLLAPSED_KEY),
  )
  useEffect(() => {
    persistStringSet(TREE_COLLAPSED_KEY, collapsedNodes)
  }, [collapsedNodes])

  /* Section collapse state. Group keys carry their mode, so one set covers
     both groupings without either overwriting the other. */
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() =>
    loadStringSet(GROUPS_COLLAPSED_KEY),
  )
  useEffect(() => {
    persistStringSet(GROUPS_COLLAPSED_KEY, collapsedGroups)
  }, [collapsedGroups])

  /* Kinds shown, grouping and order — one record, one Clear filters. */
  const [view, setView] = useState<ConversationListView>(loadListView)
  useEffect(() => {
    if (typeof window === 'undefined') return
    try {
      window.localStorage.setItem(LIST_VIEW_KEY, JSON.stringify(view))
    } catch {
      // best-effort persistence
    }
  }, [view])

  const toggleNode = useCallback((id: string) => {
    setCollapsedNodes((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  const toggleGroup = useCallback((key: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
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

  /* The kind filter runs on the flat list before the tree is built, judging
     each row by its root's kind, so hiding a kind hides whole subtrees and
     search matches inside them alike. */
  const visible = useMemo(
    () => filterConversationsByKind(conversations, view.kinds),
    [conversations, view.kinds],
  )
  const byId = useMemo(
    () => new Map(conversations.map((c) => [c.id, c])),
    [conversations],
  )
  /* A row wears its kind only when the list mixes kinds; with one kind on
     show the tag would just repeat the filter. */
  const tagKinds = view.kinds.length > 1

  const roots = useMemo<ConvNode[]>(() => {
    const q = query.trim()
    const nodes: ConvNode[] = q
      ? /* Flat matches while searching: a matched child under an unmatched
           parent has no tree context to render, so no indent/caret. Sections
           still apply — they are what tells a match from last month apart
           from one from this morning. */
        filterConversations(visible, q).map((conversation) => ({
          conversation,
          children: [],
          depth: 0,
        }))
      : buildConversationTree(visible)
    return sortConversationRoots(nodes, view.sort)
  }, [visible, query, view.sort])

  /* `Date.now()` is read here rather than held in state: the bucket a chat
     falls into is only ever read beside its own relative timestamp, which the
     row computes the same way, and both refresh on the next conversation
     update. */
  const sections = useMemo(
    () =>
      groupConversationRoots(roots, view.grouping, Date.now()).map((group) => ({
        group,
        count: countConversations(group.roots),
        rows: collapsedGroups.has(group.key)
          ? []
          : flattenConversationTree(group.roots, collapsedNodes),
      })),
    [roots, view.grouping, collapsedGroups, collapsedNodes],
  )
  /* The flat list has one section and no header to collapse it by. */
  const headers = view.grouping !== 'none'

  /* The 8px gutter plus the tree row's own 10px inset puts every glyph,
     the heading and the empty copy on one 18px column. */
  return (
    <>
      <div className="flex flex-col gap-2 px-2 pt-2 pb-1">
        <div className="flex items-center justify-between gap-2 pr-0.5 pl-[10px]">
          <h2 className="min-w-0 truncate font-sans text-base font-medium text-ink-faint sm:text-[13px]">
            Conversations
          </h2>
          <ConversationFilterMenu
            view={view}
            onViewChange={setView}
            narrow={narrow}
          />
        </div>
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
        {sections.length === 0 ? (
          <div className="flex flex-col items-start gap-2 px-[10px] py-6 font-sans text-base text-ink-ghost sm:text-[13px]">
            {query.trim() ? (
              'No matches.'
            ) : conversations.length === 0 ? (
              'No conversations yet. Start one above.'
            ) : (
              <>
                {/* Every chat is hidden by the kind filter, not missing:
                    say so, and put the way back one click away. */}
                No conversations match these filters.
                <Button
                  variant="ghost"
                  size="sm"
                  className="-ml-3 h-7 px-3 text-[12px]"
                  onClick={() => setView(DEFAULT_CONVERSATION_LIST_VIEW)}
                >
                  Clear filters
                </Button>
              </>
            )}
          </div>
        ) : (
          <div className={uiClasses.tree} data-narrow={narrow || undefined}>
            {sections.map(({ group, count, rows }) => (
              <Fragment key={group.key}>
                {headers ? (
                  <ConversationGroupHeader
                    label={group.label}
                    title={group.title}
                    count={count}
                    collapsed={collapsedGroups.has(group.key)}
                    onToggle={() => toggleGroup(group.key)}
                    narrow={narrow}
                  />
                ) : null}
                {rows.map(({ conversation: c, depth, hasChildren }) => (
                  <ConversationRow
                    key={c.id}
                    conversation={c}
                    active={c.id === activeId}
                    depth={depth}
                    hasChildren={hasChildren}
                    kind={
                      tagKinds ? effectiveConversationKind(c, byId) : undefined
                    }
                    treeCollapsed={collapsedNodes.has(c.id)}
                    onToggleTree={() => toggleNode(c.id)}
                    onSelect={() => onSelect(c.id)}
                    onRename={(title) => handleRename(c.id, title)}
                    onRemove={() => onRemove(c.id)}
                  />
                ))}
              </Fragment>
            ))}
          </div>
        )}
      </div>
    </>
  )
}
