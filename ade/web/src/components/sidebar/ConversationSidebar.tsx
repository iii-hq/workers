import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  CircleQuestionMark,
  Download,
  FolderPlus,
  Search,
  Settings,
  SquarePen,
} from 'lucide-react'
import { Fragment, useCallback, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { hashForSettingsLanding } from '@/hooks/use-hash-route'
import { hideSideChats } from '@/lib/chat-aside'
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
import { formatBinding, shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  hoverTitle,
  type KeybindingActionId,
} from '@/lib/keybindings/registry'
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
  /** Browse for a folder to remember as a project — the Add project row. */
  onAddProject?: () => void
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
  'group/row flex w-full min-w-0 items-center gap-2.5 rounded-xl px-2.5 text-left font-inter font-medium text-neutral-600 transition-colors duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-50 dark:focus-visible:ring-neutral-400'

/**
 * A row's shortcut, spelled plainly (⌘K, ⌃N) and shown only while the row
 * is hovered or focused: it slides in from the right so the hint reads as
 * an answer to the pointer, not as a second label.
 */
function ShortcutHint({ id }: { id: KeybindingActionId }) {
  const platform = shortcutPlatform()
  const binding = bindingsFor(id, platform)[0]
  if (!binding) return null
  const caps = formatBinding(binding, platform)
  return (
    <span
      aria-hidden
      className="ml-auto shrink-0 translate-x-1 pl-2 font-inter text-[11px] font-medium text-neutral-400 opacity-0 transition-[opacity,transform] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] group-hover/row:translate-x-0 group-hover/row:opacity-100 group-focus-visible/row:translate-x-0 group-focus-visible/row:opacity-100 dark:text-neutral-500"
    >
      {platform === 'mac' ? caps.join('') : caps.join('+')}
    </span>
  )
}

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
  onAddProject,
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

  /* Finding a chat by name is the palette's job (the Search row opens it);
     the list itself always shows the whole tree. */
  const roots = useMemo<ConvNode[]>(
    () => sortConversationRoots(buildConversationTree(visible), view.sort),
    [visible, view.sort],
  )

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
            title={hoverTitle('New chat', 'chat.new')}
            className={cn(
              NAV_ROW,
              narrow ? 'h-12 text-base' : 'h-9 text-[13px]',
            )}
          >
            {/* A compose mark, not a plus: the header's plus already means
                "new workspace", and the phone header spells new chat this
                way too. */}
            <SquarePen className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate">New chat</span>
            {narrow ? null : <ShortcutHint id="chat.new" />}
          </button>
        ) : null}
        {onAddProject ? (
          <button
            type="button"
            onClick={onAddProject}
            title={hoverTitle('Add project', 'project.add')}
            className={cn(
              NAV_ROW,
              narrow ? 'h-12 text-base' : 'h-9 text-[13px]',
            )}
          >
            <FolderPlus className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate">Add project</span>
            {narrow ? null : <ShortcutHint id="project.add" />}
          </button>
        ) : null}
        {/* Search is the console's one search: the ⌘K palette, which lists
            every chat by name beside pages and commands. */}
        <button
          type="button"
          onClick={() => requestPaletteOpen()}
          aria-label="Search and commands"
          title={hoverTitle('Search and commands', 'palette.toggle')}
          className={cn(NAV_ROW, narrow ? 'h-12 text-base' : 'h-9 text-[13px]')}
        >
          <Search className="size-4 shrink-0" aria-hidden />
          <span className="min-w-0 flex-1 truncate">Search</span>
          {narrow ? null : <ShortcutHint id="palette.toggle" />}
        </button>
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
            {started.length === 0 ? (
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
              // 14px/500 titles on 36px rows, 12px timestamps: the list
              // reads at the sidebar's own size, not the tree recipe's 13px.
              '[--iii-ui-tree-row:2.25rem] [&_.iii-ui-tree-item]:rounded-xl [&_.iii-ui-tree-item]:text-sm [&_.iii-ui-tree-item__meta]:text-xs',
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
          onClick={() => requestPaletteOpen({ query: 'shortcuts' })}
          aria-label="Help and keyboard shortcuts"
          title="Help and keyboard shortcuts"
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
          // onboarding-settings: tour anchor (workers/onboarding). Do not remove.
          className={cn(
            'onboarding-settings',
            QUIET_ACTION,
            narrow ? 'size-12' : 'size-8',
          )}
        >
          <Settings className="size-4" aria-hidden />
        </button>
      </div>
    </div>
  )
}
