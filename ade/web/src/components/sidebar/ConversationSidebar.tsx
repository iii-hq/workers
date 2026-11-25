import uiClasses from '@iii-dev/console-ui/ui-classes'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  ChevronRight,
  Download,
  FolderKanban,
  Plus,
  Search,
  SquarePen,
} from 'lucide-react'
import {
  type CSSProperties,
  type DragEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  measureRowSlot,
  observeNonEmptyRect,
  rowsWithSpacers,
  windowedRowIndexes,
} from '@/components/chat/transcript-window'
import { UsageFooter } from '@/components/chat/UsageFooter'
import { ProjectMenu } from '@/components/projects/ProjectMenu'
import { Button } from '@/components/ui/Button'
import { Hint } from '@/components/ui/Hint'
import { navRowVariants } from '@/components/ui/nav-row'
import type { ChatProject } from '@/lib/backend/projects'
import { hideSideChats } from '@/lib/chat-aside'
import type { ConversationDetailsDraft } from '@/lib/conversation-details'
import {
  type ConversationGrouping,
  capProjectRoots,
  countConversations,
  groupConversationRoots,
  NO_PROJECT_GROUP_KEY,
  PINNED_GROUP_KEY,
  PROJECT_ROOTS_SHOWN,
} from '@/lib/conversation-groups'
import {
  buildConversationTree,
  type ConvNode,
  flattenConversationTree,
  isInactiveToggle,
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
import { NEUTRAL_SURFACE } from '@/lib/neutral-surface'
import { requestPaletteOpen } from '@/lib/palette/open-request'
import { usePinnedConversations } from '@/lib/pinned-conversations'
import { cn } from '@/lib/utils'
import type { Conversation } from '@/types/chat'
import { ConversationFilterMenu } from './ConversationFilterMenu'
import { ConversationGroupHeader } from './ConversationGroupHeader'
import { ConversationRow } from './ConversationRow'
import './conversation-tree.css'

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
  /** Browse for a folder to remember in the folder catalogue — the Add folder row. */
  onAddProject?: () => void
  /** Import conversations from a file — a quiet action beside the list title. */
  onImport?: () => void
  /** Start a chat scoped to a project folder — a project header's plus. */
  onCreateInProject?: (path: string) => void
  onSelect: (id: string) => void
  onRename: (id: string, title: string) => void
  onRemove: (id: string) => void
  /**
   * The engine's chat projects (chat-projects §1.3.1); absent on an engine
   * without them, where Project groups by folder as before.
   */
  chatProjects?: readonly ChatProject[]
  /** Save "Conversation details" for a row. */
  onSaveDetails?: (id: string, draft: ConversationDetailsDraft) => Promise<void>
  /** "Move to project" in a row's menu. */
  onMoveToProject?: (id: string, projectId: string) => Promise<void>
  /** Show the details' "Let the agent keep it up to date" switch. */
  agentSwitch?: boolean
  /** "New project", next to New chat. */
  onNewProject?: () => void
  /** "Edit project…" from a project's menu. */
  onEditProject?: (projectId: string) => void
}

const TREE_COLLAPSED_KEY = 'iii-chat-tree-collapsed'
/** Parents whose inactive sub-agents the reader chose to show. */
const INACTIVE_SHOWN_KEY = 'iii-chat-tree-inactive-shown'
const GROUPS_COLLAPSED_KEY = 'iii-chat-groups-collapsed'
const LIST_VIEW_KEY = 'iii-chat-list-view'
/** The drag payload of a pinned chat being rearranged. */
const PIN_DRAG_MIME = 'application/x-iii-pinned-chat'

/** Rows kept mounted past each edge of the list's viewport. Rows are cheap. */
const LIST_OVERSCAN_ROWS = 12
/** `.iii-ui-tree`'s gap: a spacer is a flex child, so it brings one itself. */
const LIST_GAP_PX = 1

/** The grouping-only key this view record replaced; read as a fallback. */
const LEGACY_GROUPING_KEY = 'iii-chat-grouping'
/**
 * Set once the reader picks a grouping on this device. Until then an engine
 * with chat projects groups by Project (chat-projects §1.3.1).
 */
const GROUPING_CHOSEN_KEY = 'iii-chat-grouping-chosen'

function loadGroupingChosen(): boolean {
  try {
    return window.localStorage.getItem(GROUPING_CHOSEN_KEY) === '1'
  } catch {
    return false
  }
}

function saveGroupingChosen(): void {
  try {
    window.localStorage.setItem(GROUPING_CHOSEN_KEY, '1')
  } catch {
    // best-effort persistence
  }
}

/**
 * A navigation row at the top of the sidebar (New chat, Search): an icon
 * and a label on a rounded-sm wash that only paints on hover or focus.
 */
const NAV_ROW = cn(
  navRowVariants(),
  'h-auto gap-2.5 rounded-sm px-2.5 font-inter font-medium',
)

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
      className="ml-auto shrink-0 translate-x-1 pl-2 font-inter text-meta font-medium text-ink-ghost opacity-0 transition-[opacity,transform] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] group-hover/row:translate-x-0 group-hover/row:opacity-100 group-focus-visible/row:translate-x-0 group-focus-visible/row:opacity-100"
    >
      {platform === 'mac' ? caps.join('') : caps.join('+')}
    </span>
  )
}

/** The quiet icon buttons beside the list title and along the bottom row. */
const QUIET_ACTION =
  'inline-flex shrink-0 items-center justify-center rounded-sm text-muted-foreground transition-colors duration-[var(--motion-duration-control)] hover:bg-surface-hover/70 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50'

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
  onCreateInProject,
  onSelect,
  onRename,
  onRemove,
  chatProjects,
  onSaveDetails,
  onMoveToProject,
  agentSwitch = false,
  onNewProject,
  onEditProject,
}: ConversationSidebarProps) {
  /* Per-node tree collapse state, persisted so a folded sub-agent subtree
 stays folded across reloads. Default expanded. */
  const [collapsedNodes, setCollapsedNodes] = useState<Set<string>>(() =>
    loadStringSet(TREE_COLLAPSED_KEY),
  )
  useEffect(() => {
    persistStringSet(TREE_COLLAPSED_KEY, collapsedNodes)
  }, [collapsedNodes])

  /* A parent lists only its live sub-agents; the inactive ones fold behind
     "Show N inactive", remembered per parent once opened. */
  const [inactiveShown, setInactiveShown] = useState<Set<string>>(() =>
    loadStringSet(INACTIVE_SHOWN_KEY),
  )
  useEffect(() => {
    persistStringSet(INACTIVE_SHOWN_KEY, inactiveShown)
  }, [inactiveShown])
  const toggleInactive = useCallback((parentId: string) => {
    setInactiveShown((prev) => {
      const next = new Set(prev)
      if (next.has(parentId)) next.delete(parentId)
      else next.add(parentId)
      return next
    })
  }, [])

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
  /* Project is the default grouping while the engine has projects and the
     reader has not picked one on this device. */
  const [groupingChosen, setGroupingChosen] = useState(loadGroupingChosen)
  const projectsMode = chatProjects !== undefined
  const defaultGrouping: ConversationGrouping =
    projectsMode && (chatProjects?.length ?? 0) > 0
      ? 'project'
      : DEFAULT_CONVERSATION_LIST_VIEW.grouping
  const grouping: ConversationGrouping = groupingChosen
    ? view.grouping
    : defaultGrouping
  const effectiveView = useMemo(() => ({ ...view, grouping }), [view, grouping])
  const viewDefaults = useMemo(
    () => ({ ...DEFAULT_CONVERSATION_LIST_VIEW, grouping: defaultGrouping }),
    [defaultGrouping],
  )
  const changeView = useCallback(
    (next: ConversationListView) => {
      if (next.grouping !== grouping) {
        saveGroupingChosen()
        setGroupingChosen(true)
      }
      setView(next)
    },
    [grouping],
  )
  /* Chat projects showing every root, past the first three. */
  const [projectsExpanded, setProjectsExpanded] = useState<Set<string>>(
    () => new Set(),
  )
  const toggleProjectExpanded = useCallback((key: string) => {
    setProjectsExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }, [])
  const { pinned, togglePin, movePin } = usePinnedConversations()

  /* Pinned chats rearrange by drag: the dragged id, and the row edge it
 would land on. A drag can end anywhere (outside the list, with Escape),
 and the window hears every end, so neither outlives it. */
  const [pinDrag, setPinDrag] = useState<string | null>(null)
  const [pinDrop, setPinDrop] = useState<{
    id: string
    place: 'before' | 'after'
  } | null>(null)
  useEffect(() => {
    if (!pinDrag) return
    const reset = () => {
      setPinDrag(null)
      setPinDrop(null)
    }
    window.addEventListener('dragend', reset)
    window.addEventListener('drop', reset)
    return () => {
      window.removeEventListener('dragend', reset)
      window.removeEventListener('drop', reset)
    }
  }, [pinDrag])
  /* What a keyboard move did, for a screen reader: the drag shows it. */
  const [pinAnnouncement, setPinAnnouncement] = useState('')
  const pinReorder = useCallback(
    (id: string, order: readonly ConvNode[]) => ({
      canMoveUp: order[0]?.conversation.id !== id,
      canMoveDown: order.at(-1)?.conversation.id !== id,
      onMove: (step: -1 | 1) => {
        const index = order.findIndex((n) => n.conversation.id === id)
        const target = order[index + step]
        if (index === -1 || !target) return
        movePin(id, target.conversation.id, step < 0 ? 'before' : 'after')
        setPinAnnouncement(
          `Moved ${order[index].conversation.title} to position ${index + step + 1} of ${order.length}`,
        )
      },
      dragging: pinDrag === id,
      dropEdge: pinDrop?.id === id ? pinDrop.place : undefined,
      onDragStart: (event: DragEvent<HTMLDivElement>) => {
        event.dataTransfer.effectAllowed = 'move'
        event.dataTransfer.setData(PIN_DRAG_MIME, id)
        setPinDrag(id)
      },
      onDragOver: (event: DragEvent<HTMLDivElement>) => {
        if (!pinDrag) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        const box = event.currentTarget.getBoundingClientRect()
        const place =
          event.clientY < box.top + box.height / 2 ? 'before' : 'after'
        setPinDrop((current) =>
          current?.id === id && current.place === place
            ? current
            : { id, place },
        )
      },
      onDrop: (event: DragEvent<HTMLDivElement>) => {
        event.preventDefault()
        const dragged = event.dataTransfer.getData(PIN_DRAG_MIME) || pinDrag
        if (dragged && pinDrop?.id === id) movePin(dragged, id, pinDrop.place)
        setPinDrag(null)
        setPinDrop(null)
      },
      onDragEnd: () => {
        setPinDrag(null)
        setPinDrop(null)
      },
    }),
    [pinDrag, pinDrop, movePin],
  )
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
      groupConversationRoots(
        roots,
        grouping,
        Date.now(),
        pinned,
        projectsMode ? chatProjects : undefined,
      ).map((group) => {
        const folded = collapsedGroups.has(group.key)
        // A chat project lists its 3 newest roots, then "N more".
        const capped = group.chatProject
          ? capProjectRoots(
              group.roots,
              projectsExpanded.has(group.key),
              activeId,
            )
          : { shown: group.roots, hidden: 0 }
        return {
          group,
          count: group.chatProject
            ? group.roots.length
            : countConversations(group.roots),
          hidden: folded ? 0 : capped.hidden,
          rows: folded
            ? []
            : flattenConversationTree(
                capped.shown,
                collapsedNodes,
                inactiveShown,
                activeId,
              ),
        }
      }),
    [
      roots,
      grouping,
      projectsMode,
      chatProjects,
      projectsExpanded,
      collapsedGroups,
      collapsedNodes,
      inactiveShown,
      activeId,
      pinned,
    ],
  )
  /* The flat list has one section and no header to collapse it by — unless
 pins split it, when both halves need naming. */
  const headers = grouping !== 'none' || pinned.size > 0
  /* Under chat projects every root row has its second line; elsewhere only
     a status line or a description earns one. */
  const secondLine: 'full' | 'status' =
    projectsMode && grouping === 'project' ? 'full' : 'status'
  /* Once any row carries a sub-agent caret at its end, every row keeps that
 column so timestamps and actions line up down the list. */
  const caretColumn = sections.some((section) =>
    section.rows.some((row) => !isInactiveToggle(row) && row.hasChildren),
  )

  /* Windowed: only the rows around the viewport are mounted, in normal flow,
 with a spacer for each run of the others. Sections are flattened for it, and
 a row is its own child of the list. The active row stays, as does the last
 one focused, which covers its menu and rename dialog: their focus reaches it
 through the portal. A dragged row stays too, or its dragend never fires. No
 ResizeObserver means no layout to window against (SSR, jsdom), so every row
 renders. */
  const windowed = typeof ResizeObserver !== 'undefined'
  const items = useMemo(
    () =>
      sections.flatMap(({ group, count, rows, hidden }) => {
        // A project folder heads its own chats; date buckets stay labels.
        const project =
          Boolean(group.chatProject) ||
          (group.key.startsWith('project:') &&
            group.key !== NO_PROJECT_GROUP_KEY)
        const folded = collapsedGroups.has(group.key)
        const expanded = projectsExpanded.has(group.key)
        return [
          ...(headers
            ? [
                {
                  kind: 'header' as const,
                  key: `header:${group.key}`,
                  group,
                  count,
                  project,
                },
              ]
            : []),
          ...rows.map((row, position) => ({
            kind: 'row' as const,
            // Not by section: a chat changing bucket keeps its menu or dialog.
            key: isInactiveToggle(row)
              ? `inactive:${row.parentId}`
              : row.conversation.id,
            row,
            group,
            project,
            stacked: project && position > 0,
          })),
          // An empty chat project still shows one row under its header.
          ...(group.chatProject && !folded && group.roots.length === 0
            ? [
                {
                  kind: 'empty' as const,
                  key: `empty:${group.key}`,
                  group,
                  project,
                },
              ]
            : []),
          // "N more" past a project's first three roots; "Show less" after.
          ...(group.chatProject &&
          !folded &&
          (hidden > 0 || (expanded && group.roots.length > PROJECT_ROOTS_SHOWN))
            ? [
                {
                  kind: 'more' as const,
                  key: `more:${group.key}`,
                  group,
                  project,
                  hidden,
                  expanded,
                },
              ]
            : []),
        ]
      }),
    [sections, headers, collapsedGroups, projectsExpanded],
  )
  const [focusedKey, setFocusedKey] = useState<string | null>(null)
  const heldIndexes = useMemo(() => {
    const held = new Set<number>()
    items.forEach((item, index) => {
      if (item.key === focusedKey) {
        // Its neighbors stay as tab stops, so the keyboard walk goes on from it.
        held.add(index - 1)
        held.add(index)
        held.add(index + 1)
      }
      if (item.kind !== 'row' || isInactiveToggle(item.row)) return
      const { id } = item.row.conversation
      if (id === activeId || id === pinDrag) held.add(index)
    })
    return held
  }, [items, focusedKey, activeId, pinDrag])
  const listRef = useRef<HTMLDivElement>(null)
  const getItemKey = useCallback(
    (index: number) => items[index]?.key ?? index,
    [items],
  )
  const estimateItem = useCallback(
    (index: number) => {
      const item = items[index]
      if (item?.kind === 'header') return item.project ? 49 : 41
      if (item?.kind === 'more' || item?.kind === 'empty')
        return narrow ? 45 : 37
      // A two-line row (chat projects) is 52px; measured once mounted.
      const twoLine =
        item?.kind === 'row' &&
        !isInactiveToggle(item.row) &&
        item.row.depth === 0 &&
        (secondLine === 'full' || Boolean(item.row.conversation.description))
      // A stacked row sits 1px up (conversation-tree.css), and its slot counts it.
      return (
        (twoLine ? (narrow ? 57 : 53) : narrow ? 45 : 37) -
        (item?.stacked ? 1 : 0)
      )
    },
    [items, narrow, secondLine],
  )
  // A changed list's range still comes from the old offset, so a section moving
  // up would unmount the rows in view. They stay for that commit, for the browser
  // to anchor on; the scroll it makes then moves the window to them.
  const viewedRowsRef = useRef<{
    items: typeof items
    keys: ReadonlySet<string>
  }>({ items, keys: new Set() })
  const carriedIndexes =
    viewedRowsRef.current.items === items
      ? []
      : items.flatMap((item, index) =>
          viewedRowsRef.current.keys.has(item.key) ? [index] : [],
        )
  const virtualizer = useVirtualizer<HTMLElement, HTMLDivElement>({
    enabled: windowed,
    count: items.length,
    getScrollElement: () => listRef.current,
    getItemKey,
    estimateSize: estimateItem,
    overscan: LIST_OVERSCAN_ROWS,
    initialRect: { width: 0, height: windowed ? window.innerHeight : 0 },
    observeElementRect: observeNonEmptyRect,
    measureElement: measureRowSlot,
    rangeExtractor: (range) =>
      windowedRowIndexes(range, {
        opening: false,
        frozen: null,
        pinned: [...heldIndexes, ...carriedIndexes],
      }),
  })
  // The browser's scroll anchoring already holds the rows in view when one above
  // them changes; correcting too would move them by twice that.
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = () => false
  const virtualItems = windowed ? virtualizer.getVirtualItems() : null
  useLayoutEffect(() => {
    const list = listRef.current
    if (windowed && list && viewedRowsRef.current.items !== items) {
      // The browser has anchored the carried rows by now. The library only hears
      // of that scroll from its event, and a render before it would mount the
      // window at the old offset again: a blank frame, and the rows coming back
      // without their anchor. Tell it now, and move the window in this commit.
      const before = virtualizer.range
      virtualizer.scrollOffset = list.scrollTop
      const after = virtualizer.calculateRange()
      if (
        after?.startIndex !== before?.startIndex ||
        after?.endIndex !== before?.endIndex
      ) {
        virtualizer.options.onChange?.(virtualizer, false)
      }
    }
    const range = virtualizer.range
    viewedRowsRef.current = {
      items,
      keys: new Set(
        range
          ? items
              .slice(range.startIndex, range.endIndex + 1)
              .map((item) => item.key)
          : [],
      ),
    }
  })

  const renderRow = ({
    row,
    group,
  }: Extract<(typeof items)[number], { kind: 'row' }>) => {
    if (isInactiveToggle(row)) {
      return (
        <InactiveToggle
          depth={row.depth}
          inactive={row.inactive}
          expanded={row.expanded}
          onToggle={() => toggleInactive(row.parentId)}
        />
      )
    }
    const { conversation: c, depth, hasChildren } = row
    return (
      <ConversationRow
        conversation={c}
        active={c.id === activeId}
        depth={depth}
        hasChildren={hasChildren}
        caretColumn={caretColumn}
        kind={tagKinds ? effectiveConversationKind(c, byId) : undefined}
        treeCollapsed={collapsedNodes.has(c.id)}
        onToggleTree={() => toggleNode(c.id)}
        onSelect={() => onSelect(c.id)}
        onRename={(title) => onRename(c.id, title)}
        onRemove={() => onRemove(c.id)}
        onSaveDetails={
          onSaveDetails ? (draft) => onSaveDetails(c.id, draft) : undefined
        }
        projects={projectsMode ? chatProjects : undefined}
        onMoveToProject={
          onMoveToProject && projectsMode
            ? (projectId) => onMoveToProject(c.id, projectId)
            : undefined
        }
        agentSwitch={agentSwitch}
        conversations={conversations}
        secondLine={secondLine}
        folderTail={group.noProject === true}
        pinned={pinned.has(c.id)}
        onTogglePin={() => togglePin(c.id)}
        reorder={
          group.key === PINNED_GROUP_KEY &&
          depth === 0 &&
          group.roots.length > 1
            ? pinReorder(c.id, group.roots)
            : undefined
        }
      />
    )
  }

  const renderItem = (item: (typeof items)[number], index: number) => (
    <div
      key={item.key}
      ref={windowed ? virtualizer.measureElement : undefined}
      data-index={index}
      className={
        item.kind === 'row' && item.project
          ? 'conversation-project-rows'
          : undefined
      }
      data-stacked={item.kind === 'row' && item.stacked ? '' : undefined}
      onFocusCapture={() => setFocusedKey(item.key)}
    >
      {item.kind === 'empty' ? (
        <div className={cn(uiClasses.treeItem, 'conversation-project-empty')}>
          <span className={uiClasses.treeItemLabel}>No conversations yet</span>
        </div>
      ) : item.kind === 'more' ? (
        <button
          type="button"
          className={cn(uiClasses.treeItem, 'conversation-project-more')}
          aria-expanded={item.expanded}
          onClick={() => toggleProjectExpanded(item.group.key)}
        >
          <span className={uiClasses.treeItemLabel}>
            {item.expanded ? 'Show less' : `${item.hidden} more`}
          </span>
        </button>
      ) : item.kind === 'header' ? (
        <ConversationGroupHeader
          label={item.group.label}
          title={item.group.title}
          count={item.count}
          collapsed={collapsedGroups.has(item.group.key)}
          onToggle={() => toggleGroup(item.group.key)}
          narrow={narrow}
          project={item.project}
          chatProject={item.group.chatProject}
          onCreateIn={onCreateInProject}
          menu={
            item.group.chatProject ? (
              <ProjectMenu
                project={item.group.chatProject}
                // As the list counts a project: its conversations, not the
                // subagents nested under them (spec §1.3.2, P11).
                conversationCount={item.group.roots.length}
                narrow={narrow}
                onNewConversation={() => {
                  const folder = item.group.chatProject?.folders[0]
                  if (folder) onCreateInProject?.(folder)
                }}
                onEdit={() =>
                  item.group.chatProject &&
                  onEditProject?.(item.group.chatProject.id)
                }
              />
            ) : undefined
          }
          onCreate={
            item.project && item.group.title && onCreateInProject
              ? () => onCreateInProject(item.group.title as string)
              : undefined
          }
        />
      ) : (
        renderRow(item)
      )}
    </div>
  )

  /* The 8px gutter plus the nav row's own 10px inset puts every glyph, the
 heading and the empty copy on one 18px column. */
  return (
    <div
      className={cn('flex min-h-0 flex-1 flex-col font-inter', NEUTRAL_SURFACE)}
    >
      <nav
        aria-label="Chat actions"
        className="flex flex-col gap-0.5 px-2 pt-1"
      >
        {onCreate ? (
          <button
            type="button"
            onClick={onCreate}
            title={hoverTitle('New chat', 'chat.new')}
            className={cn(NAV_ROW, narrow ? 'h-12 text-base' : 'h-9 text-sm')}
          >
            {/* A compose mark, not a plus: the header's plus already means
                "new workspace", and the phone header spells new chat this
 way too. */}
            <SquarePen className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate">New chat</span>
            {narrow ? null : <ShortcutHint id="chat.new" />}
          </button>
        ) : null}
        {onNewProject && projectsMode ? (
          <button
            type="button"
            onClick={onNewProject}
            className={cn(NAV_ROW, narrow ? 'h-12 text-base' : 'h-9 text-sm')}
          >
            <FolderKanban className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate">New project</span>
          </button>
        ) : null}
        {onAddProject ? (
          <button
            type="button"
            onClick={onAddProject}
            title={hoverTitle('Add folder', 'project.add')}
            className={cn(NAV_ROW, narrow ? 'h-12 text-base' : 'h-9 text-sm')}
          >
            <Plus className="size-4 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate">Add folder</span>
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
          className={cn(NAV_ROW, narrow ? 'h-12 text-base' : 'h-9 text-sm')}
        >
          <Search className="size-4 shrink-0" aria-hidden />
          <span className="min-w-0 flex-1 truncate">Search</span>
          {narrow ? null : <ShortcutHint id="palette.toggle" />}
        </button>
      </nav>

      <div className="flex items-center justify-between gap-2 px-2 pt-4 pb-1 pl-[18px]">
        <h2 className="min-w-0 truncate font-inter text-base font-medium text-muted-foreground sm:text-sm">
          Chats
        </h2>
        <div className="flex items-center gap-0.5">
          {onImport ? (
            <Hint label="Import chats" disabled={narrow}>
              <button
                type="button"
                onClick={onImport}
                aria-label="Import chats"
                className={cn(QUIET_ACTION, narrow ? 'size-12' : 'size-7')}
              >
                <Download className="size-4" aria-hidden />
              </button>
            </Hint>
          ) : null}
          <ConversationFilterMenu
            view={effectiveView}
            onViewChange={changeView}
            narrow={narrow}
            defaults={viewDefaults}
          />
        </div>
      </div>

      <div role="status" className="sr-only">
        {pinAnnouncement}
      </div>
      <div
        ref={listRef}
        className="conversation-list-scroll min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 py-1"
      >
        {sections.length === 0 ? (
          <div className="flex flex-col items-start gap-2 px-[10px] py-6 font-inter text-base text-ink-ghost sm:text-sm">
            {started.length === 0 ? (
              'No chats yet. Start one with New chat.'
            ) : (
              <>
                {/* Every chat is hidden by the kind filter, not missing:
 say so, and put the way back one click away. */}
                No chats match these filters.
                <Button
                  variant="ghost"
                  size="sm"
                  className="-ml-3 h-7 px-3 text-xs"
                  onClick={() => changeView(viewDefaults)}
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
              // 14px titles on the nav rows' 36px box, 12px timestamps and
              // the shared neutral-900 selected wash (conversation-tree.css).
              'conversation-tree',
            )}
            data-narrow={narrow || undefined}
          >
            {virtualItems
              ? rowsWithSpacers(
                  virtualItems,
                  virtualizer.getTotalSize(),
                  (index) => renderItem(items[index], index),
                  (key, slot) => (
                    <div
                      key={key}
                      aria-hidden="true"
                      data-list-spacer=""
                      style={{ height: Math.max(0, slot - LIST_GAP_PX) }}
                    />
                  ),
                )
              : items.map(renderItem)}
          </div>
        )}
      </div>
      {/* Claude Code accounts' week (chat-projects §4.3.3): whenever the
          list shows, on an engine with the whole feature. */}
      <UsageFooter />
    </div>
  )
}

/**
 * The fold under a parent's live sub-agents: how many are inactive (done
 * for now; an observation or the main agent can start them again), and the
 * way to see them. Sits at the sub-agents' indent with a disclosure chevron,
 * quiet until pointed at.
 */
function InactiveToggle({
  depth,
  inactive,
  expanded,
  onToggle,
}: {
  depth: number
  inactive: number
  expanded: boolean
  onToggle: () => void
}) {
  return (
    <button
      type="button"
      className={cn(uiClasses.treeItem, 'conversation-inactive-toggle')}
      style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
      aria-expanded={expanded}
      onClick={onToggle}
    >
      {/* A disclosure chevron in the glyph column, like the parent's caret:
 right while folded, down once the inactive rows show under it. */}
      <span className={uiClasses.treeItemIcon}>
        <ChevronRight aria-hidden />
      </span>
      <span className={uiClasses.treeItemLabel}>
        {expanded ? 'Hide inactive' : `Show ${inactive} inactive`}
      </span>
    </button>
  )
}
