import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  CircleQuestionMark,
  Download,
  Search,
  Settings,
  SquarePen,
} from 'lucide-react'
import { Fragment, useCallback, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { hashForSettingsLanding } from '@/hooks/use-hash-route'
import { hideSideChats } from '@/lib/chat-aside'
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
import { requestPaletteOpen } from '@/lib/palette/open-request'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ConversationFilterMenu } from './ConversationFilterMenu'
import { ConversationGroupHeader } from './ConversationGroupHeader'
import { ConversationRow } from './ConversationRow'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

interface ConversationSidebarProps {
  conversations: Conversation[]
  activeId: string | null
  /** Touch-sized rows and always-visible actions when the list is the whole narrow page. */
  narrow?: boolean
  /** Start a conversation — the New chat row at the top of the sidebar. */
  onCreate?: () => void
  /** Import conversations from a file — a quiet action beside the list title. */
  onImport?: () => void
  onSelect: (id: string) => void
  onRename: (id: string, title: string) => void
  onRemove: (id: string) => void
}

const TREE_COLLAPSED_KEY = 'iii-chat-tree-collapsed'
const GROUPS_COLLAPSED_KEY = 'iii-chat-groups-collapsed'
const LIST_VIEW_KEY = 'iii-chat-list-view'
/** The grouping-only key this view record replaced; read as a fallback. */
const LEGACY_GROUPING_KEY = 'iii-chat-grouping'

/**
 * A navigation row at the top of the sidebar (New chat, Search): an icon
 * and a label on a rounded-xl wash that only paints on hover or focus.
 */
const NAV_ROW =
  'flex w-full min-w-0 items-center gap-2.5 rounded-xl px-2.5 text-left font-inter font-medium text-neutral-600 transition-colors duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-50 dark:focus-visible:ring-neutral-400'

/** The quiet icon buttons beside the list title and along the bottom row. */
const QUIET_ACTION =
  'inline-flex shrink-0 items-center justify-center rounded-xl text-neutral-500 transition-colors duration-[var(--motion-duration-control)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-50 dark:focus-visible:ring-neutral-400'

/**
 * The sidebar reads the console tokens through the Tailwind neutral ramp:
 * every shared recipe inside it (tree rows, the filter menu trigger, the
 * group headers) keeps its own classes and simply resolves to neutral.
 */
const NEUTRAL_TOKENS =
  '[--color-ink:var(--color-neutral-900)] [--color-ink-faint:var(--color-neutral-500)] [--color-ink-ghost:var(--color-neutral-400)] [--color-surface:var(--color-neutral-100)] [--color-surface-hover:var(--color-neutral-200)] [--color-surface-selected:var(--color-neutral-200)] dark:[--color-ink:var(--color-neutral-50)] dark:[--color-ink-faint:var(--color-neutral-400)] dark:[--color-ink-ghost:var(--color-neutral-500)] dark:[--color-surface:var(--color-neutral-900)] dark:[--color-surface-hover:var(--color-neutral-800)] dark:[--color-surface-selected:var(--color-neutral-800)]'

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
  onCreate,
  onImport,
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

  /* A chat joins the list on its first send: an unsent "new chat" is a
     local draft with nothing to show yet. The kind filter runs on the flat
     list before the tree is built, judging each row by its root's kind, so
     hiding a kind hides whole subtrees and search matches inside them alike. */
  const started = useMemo(
    // A side chat (and what it spawned) lives in its conversation's side
    // panel until it is kept.
    () => hideSideChats(conversations.filter((c) => !c.draft)),
    [conversations],
  )
  const visible = useMemo(
    () => filterConversationsByKind(started, view.kinds),
    [started, view.kinds],
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
  /* Once any row carries a sub-agent caret at its end, every row keeps that
     column so timestamps and actions line up down the list. */
  const caretColumn = sections.some((section) =>
    section.rows.some((row) => row.hasChildren),
  )

  /* The 8px gutter plus the nav row's own 10px inset puts every glyph, the
     heading and the empty copy on one 18px column. */
  return (
    <div
      className={cn('flex min-h-0 flex-1 flex-col font-inter', NEUTRAL_TOKENS)}
    >
      <nav
        aria-label="Conversation actions"
        className="flex flex-col gap-0.5 px-2 pt-1"
      >
        {onCreate ? (
          <button
            type="button"
            onClick={onCreate}
            className={cn(
              NAV_ROW,
              narrow ? 'h-12 text-base' : 'h-9 text-[13px]',
            )}
          >
            {/* A compose mark, not a plus: the header's plus already means
                "new workspace", and the phone header spells new chat this
                way too. */}
            <SquarePen className="size-4 shrink-0" aria-hidden />
            New chat
          </button>
        ) : null}
        {/* Search is a row like the others; the field is the row. */}
        <div className="relative">
          <Search
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-neutral-500 dark:text-neutral-400"
            aria-hidden
          />
          <input
            name="conversation-search"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder="Search"
            aria-label="search conversations"
            data-conversation-search=""
            className={cn(
              NAV_ROW,
              'pl-9 placeholder:text-neutral-600 focus:bg-neutral-100 dark:placeholder:text-neutral-400 dark:focus:bg-neutral-900',
              narrow ? 'h-12 text-base' : 'h-9 text-[13px]',
            )}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setQuery('')
            }}
          />
        </div>
      </nav>

      <div className="flex items-center justify-between gap-2 px-2 pt-4 pb-1 pl-[18px]">
        <h2 className="min-w-0 truncate font-inter text-base font-medium text-neutral-500 sm:text-[13px] dark:text-neutral-400">
          Conversations
        </h2>
        <div className="flex items-center gap-0.5">
          {onImport ? (
            <button
              type="button"
              onClick={onImport}
              aria-label="Import conversations"
              title="Import conversations"
              className={cn(QUIET_ACTION, narrow ? 'size-12' : 'size-7')}
            >
              <Download className="size-4" aria-hidden />
            </button>
          ) : null}
          <ConversationFilterMenu
            view={view}
            onViewChange={setView}
            narrow={narrow}
          />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-1">
        {sections.length === 0 ? (
          <div className="flex flex-col items-start gap-2 px-[10px] py-6 font-sans text-base text-ink-ghost sm:text-[13px]">
            {query.trim() ? (
              'No matches.'
            ) : started.length === 0 ? (
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
          <div
            className={cn(
              uiClasses.tree,
              '[--iii-ui-tree-row:2.25rem] [&_.iii-ui-tree-item]:rounded-xl',
            )}
            data-narrow={narrow || undefined}
          >
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
                    caretColumn={caretColumn}
                    kind={
                      tagKinds ? effectiveConversationKind(c, byId) : undefined
                    }
                    treeCollapsed={collapsedNodes.has(c.id)}
                    onToggleTree={() => toggleNode(c.id)}
                    onSelect={() => onSelect(c.id)}
                    onRename={(title) => onRename(c.id, title)}
                    onRemove={() => onRemove(c.id)}
                  />
                ))}
              </Fragment>
            ))}
          </div>
        )}
      </div>

      {/* The bottom row: help on the left of settings, nothing else. */}
      <div className="flex shrink-0 items-center justify-end gap-1 px-2 py-2">
        <button
          type="button"
          onClick={() => requestPaletteOpen()}
          aria-label="Help: search and commands"
          title="Help: search and commands"
          className={cn(QUIET_ACTION, narrow ? 'size-12' : 'size-8')}
        >
          <CircleQuestionMark className="size-4" aria-hidden />
        </button>
        <button
          type="button"
          onClick={() => {
            window.location.hash = hashForSettingsLanding(narrow)
          }}
          aria-label="ADE settings"
          title="ADE settings"
          className={cn(QUIET_ACTION, narrow ? 'size-12' : 'size-8')}
        >
          <Settings className="size-4" aria-hidden />
        </button>
      </div>
    </div>
  )
}
