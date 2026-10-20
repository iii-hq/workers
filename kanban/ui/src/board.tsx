import {
  Button,
  Card,
  Chip,
  EmptyState,
  type Host,
  IconButton,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  Skeleton,
  StatusDot,
  StatusPanel,
  Tabs,
  TabsList,
  TabsTrigger,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import {
  Bot,
  CircleDashed,
  Columns2,
  CornerDownRight,
  ListTree,
  Maximize2,
  MessageCircle,
  Plus,
  RefreshCw,
  TriangleAlert,
} from 'lucide-react'
import { type DragEvent, memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  applyRead,
  applyWrite,
  buildSwimlanes,
  coerceGroup,
  dropRefusal,
  flattenLanes,
  fullIndex,
  type GroupBy,
  knownLaneIds,
  laneChange,
  laneIdOf,
  type Swimlane,
  subtaskStats,
  type Tombstones,
  upsertSummary,
  type Write,
  type WriteKind,
} from './board-state'
import { FilterBar, ViewTabs } from './filter-bar'
import {
  ALL_VIEW,
  allLabels,
  applyFilters,
  coerceFilters,
  coerceViews,
  EMPTY_FILTERS,
  type Filters,
  isChangedSinceVisit,
  isEmpty,
  type MatchContext,
  type SavedView,
  sameFilters,
  validVisit,
} from './filters'
import { TicketList } from './list'
import { CreateTicketDialog } from './overlays'
import {
  type AgentProfile,
  api,
  type BoardColumn,
  type BoardData,
  noteOwnWrite,
  OWN_WRITES,
  PLAN_CHANGED_EVENT,
  PROJECTS_PAGE_ID,
  PriorityChip,
  RELEASES_PAGE_ID,
  StatusGlyph,
  statusLabel,
  subscribeChanges,
  Pills,
  summarizeTicket,
  TICKET_PAGE_ID,
  type Ticket,
  type TicketSummary,
  useOwnWritesVersion,
} from './shared'
import { cellKey, DragLine, type DropTarget, SwimlaneGrid, SwimlaneSections } from './swimlanes'
import { TicketScreen } from './ticket'
import { WORKER } from './worker'

type DragState = { id: string; from: string; height: number }

/** What the New ticket dialog opens with: a status, and the swimlane it was started from (its release, project or parent, and its name). */
type NewTicket = {
  status: string
  project?: string | null
  release?: string | null
  parent?: string | null
  lane?: string
}

/** A new ticket starts in the swimlane it was asked from. A subtask begins with its parent's project and release. */
function newIn(lane: Swimlane, status: string): NewTicket {
  if (lane.facet === 'release') return { status, release: lane.value, lane: lane.short }
  if (lane.facet === 'project') return { status, project: lane.value, lane: lane.short }
  const parent = lane.parent
  return {
    status,
    parent: lane.value,
    lane: lane.short,
    ...(parent ? { project: parent.project_id, release: parent.release_id } : {}),
  }
}

/** The writes of one card go one after the other, whichever pane made them: a second drop must not overtake the first. */
const moveTails = new Map<string, Promise<void>>()

/** A callback that keeps its identity and runs the latest closure: the memoised swimlane cells are not redrawn by it. */
function useLatest<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  const latest = useRef(fn)
  useLayoutEffect(() => {
    latest.current = fn
  })
  return useCallback((...args: A) => latest.current(...args), [])
}

/** Panel contexts (`panelContext.id`) a board already applied: the host keeps the last one per page for good. */
const handledContexts = new Set<number>()

/** Set by dragstart: only a drag that carries it is a ticket drag (not a file, a selection, another pane's card). */
const DRAG_TYPE = 'application/x-kanban-ticket'
const carriesTicket = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes(DRAG_TYPE)

const VIEWS_KEY = `${WORKER}:views`
/** Dispatched on window after the views are written, so the other panes of this document pick them up. */
const VIEWS_EVENT = 'kanban:views-changed'

/** Saved views are shared by every pane and tab, so the next list is built from storage, not from this pane's copy. */
function readViews(): SavedView[] | null {
  try {
    const raw = localStorage.getItem(VIEWS_KEY)
    return coerceViews(raw === null ? [] : JSON.parse(raw))
  } catch {
    return null
  }
}

/** Scroll offsets of the board, kept while a ticket is open in its place. */
type Scroll = {
  board: number
  strip: number
  lanes: Record<string, number>
  /** The swimlane grid (wide) or the sections (narrow). */
  swim: { top: number; left: number }
}

type TicketCardProps = {
  ticket: TicketSummary
  dragging: boolean
  changed: boolean
  now: number
  agent: AgentProfile | undefined
  /** The parent's key, when the board holds the parent. */
  parentKey: string | undefined
  subs: { done: number; total: number } | undefined
  /** The labels the filters ask for: they come first and show selected. */
  highlight: string[]
  priorities: string[]
  doneStatus: string
  canOpenPane: boolean
  onOpen: (key: string) => void
  onOpenHere: (key: string) => void
  onDragStart: (ticket: TicketSummary, event: DragEvent<HTMLElement>) => void
}

/** One card. Memoised: a drag over the swimlanes redraws the cells it crosses, not the thousands of cards around them. */
const TicketCard = memo(function TicketCard({
  ticket,
  dragging,
  changed,
  now,
  agent,
  parentKey,
  subs,
  highlight,
  priorities,
  doneStatus,
  canOpenPane,
  onOpen,
  onOpenHere,
  onDragStart,
}: TicketCardProps) {
  const assignee = agent?.name ?? ticket.assignee
  const labels = [...ticket.labels].sort((a, b) => Number(highlight.includes(b)) - Number(highlight.includes(a)))
  const rest = labels.slice(3)
  const rank = priorities.indexOf(ticket.priority)
  const notable = rank > 0 && rank >= priorities.length - 2
  // A parent in the done status with subtasks still open says so.
  const openSubs = subs && ticket.status === doneStatus ? subs.total - subs.done : 0
  const subsLabel = subs
    ? `${subs.done} of ${subs.total} subtasks done${openSubs > 0 ? `; done with ${openSubs} open` : ''}`
    : ''
  return (
    <Card
      aria-label={[
        `${ticket.key}: ${ticket.title}`,
        parentKey ? `subtask of ${parentKey}` : '',
        subsLabel,
        changed ? 'changed since your last visit' : '',
      ]
        .filter(Boolean)
        .join(', ')}
      className="kanban-card"
      data-changed={changed ? 'true' : undefined}
      data-dragging={dragging ? 'true' : undefined}
      data-ticket-card=""
      data-ticket-id={ticket.id}
      draggable
      interactive
      onClick={() => onOpen(ticket.key)}
      onDragStart={(event) => onDragStart(ticket, event)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onOpen(ticket.key)
        }
      }}
      role="button"
      tabIndex={0}
    >
      {parentKey ? (
        <span className="kanban-card__subof">
          <CornerDownRight size={16} />
          Subtask of <span className="kanban-card__key">{parentKey}</span>
        </span>
      ) : null}
      <div className="kanban-card__head">
        <div className="kanban-card__title">{ticket.title}</div>
        {canOpenPane ? (
          <IconButton
            className="kanban-card__here"
            label="Open in this tab"
            onClick={(event) => {
              event.stopPropagation()
              onOpenHere(ticket.key)
            }}
            onKeyDown={(event) => event.stopPropagation()}
            variant="ghost"
          >
            <Maximize2 size={16} />
          </IconButton>
        ) : null}
      </div>
      {labels.length > 0 ? (
        <div className="kanban-card__labels">
          {labels.slice(0, 3).map((label) => (
            <Chip className="kanban-card__label" key={label} selected={highlight.includes(label)}>
              {label}
            </Chip>
          ))}
          {rest.length > 0 ? (
            <Chip className="kanban-card__more" title={rest.join(', ')}>
              +{rest.length}
            </Chip>
          ) : null}
        </div>
      ) : null}
      <Pills pills={ticket.pills} />
      <div className="kanban-card__foot">
        <span className="kanban-card__key">{ticket.key}</span>
        {notable ? <PriorityChip priority={ticket.priority} priorities={priorities} /> : null}
        <span className="kanban-card__spacer" />
        {changed ? (
          <span className="kanban-card__changed" title="Changed since your last visit">
            <StatusDot tone="accent" />
            {formatRelative(ticket.updated_at, now)}
          </span>
        ) : null}
        {subs ? (
          <span aria-label={subsLabel} className="kanban-card__subs" role="img">
            <ListTree size={16} />
            {subs.done}/{subs.total}
            {openSubs > 0 ? <StatusDot tone="warn" /> : null}
          </span>
        ) : null}
        {ticket.comment_count > 0 ? (
          <span className="kanban-card__meta">
            <MessageCircle size={16} />
            {ticket.comment_count}
          </span>
        ) : null}
        <span
          aria-label={assignee ?? 'Unassigned'}
          className={`${uiClasses.treeItemIcon} kanban-card__assignee`}
          data-color={agent?.color ?? undefined}
          role="img"
          title={assignee ?? 'Unassigned'}
        >
          {assignee ? <Bot size={16} /> : <CircleDashed size={16} />}
        </span>
      </div>
    </Card>
  )
})

export function BoardPage({
  host,
  onRequestClose,
  paneId,
  tabId,
  commands,
  panelContext,
}: PageRenderProps & { host: Host }) {
  const [data, setData] = useState<BoardData | null>(null)
  // The latest board, readable between renders: every write is applied to it, not to a stale closure.
  const dataRef = useRef<BoardData | null>(null)
  const tombstones = useRef<Tombstones>(new Map())
  const [agents, setAgents] = useState<AgentProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  // The banner above the board: nothing saved by default; a move can be saved in part.
  const [notice, setNotice] = useState<{ headline: string; detail: string } | null>(null)
  const setActionError = useCallback(
    (detail: string | null) => setNotice(detail === null ? null : { headline: 'That change was not saved', detail }),
    [],
  )
  const [creating, setCreating] = useState<NewTicket | null>(null)
  const [inlineTicketId, setInlineTicketId] = useState<string | null>(null)
  const [drag, setDrag] = useState<DragState | null>(null)
  const [dropTarget, setDropTarget] = useState<DropTarget | null>(null)
  const { ref: narrowRef, narrow } = useContainerNarrow({ below: 760 })
  const paneKey = paneId || tabId || 'default'
  const [activeLane, setActiveLane] = usePaneState<string>(`kanban:lane:${paneKey}`, 'backlog')
  const dragRef = useRef<DragState | null>(null)
  const dragSeq = useRef(0)
  const boardEl = useRef<HTMLElement | null>(null)
  const scrollMemo = useRef<Scroll | null>(null)
  const setBoardEl = useCallback(
    (node: HTMLElement | null) => {
      boardEl.current = node
      narrowRef(node)
    },
    [narrowRef],
  )

  const [storedViews, setStoredViews] = usePaneState<SavedView[]>(VIEWS_KEY, [])
  const [activeViewId, setActiveViewId] = usePaneState<string>(`kanban:view:${paneKey}`, ALL_VIEW.id)
  const [storedFilters, setFilters] = usePaneState<Filters>(`kanban:filters:${paneKey}`, EMPTY_FILTERS)
  const [storedLayout, setLayout] = usePaneState<'board' | 'list'>(`kanban:layout:${paneKey}`, 'board')
  const [storedGroup, setGroup] = usePaneState<GroupBy>(`kanban:group:${paneKey}`, 'none')
  const [showEmpty, setShowEmpty] = usePaneState<boolean>(`kanban:swim-empty:${paneKey}`, false)
  // Swimlanes the user folded or unfolded; the rest follow their default.
  const [folded, setFolded] = usePaneState<Record<string, boolean>>(`kanban:swim-fold:${paneKey}`, {})
  // Swimlane cells opened up in place ("Show N more"); a visit to the board starts them closed again.
  const [openCells, setOpenCells] = useState<ReadonlySet<string>>(new Set())
  const [lastVisit, setLastVisit] = useState<string | null>(null)
  const visitRead = useRef(false)
  // Writes this tab made (any pane): they are not "changed since your last visit", later edits by others are.
  const ownVersion = useOwnWritesVersion()

  const canOpenPane = Boolean(host.panels && typeof host.panels.open === 'function')

  // The previous visit is read once per pane, when the board arrives; the new one is written when the
  // pane goes away, so every pane opened in a session compares against the same previous visit.
  const visitKey = data ? `kanban:last-visit:${data.id_prefix}` : null
  useEffect(() => {
    if (!visitKey) return
    if (!visitRead.current) {
      visitRead.current = true
      try {
        setLastVisit(validVisit(localStorage.getItem(visitKey)))
      } catch {
        undefined
      }
    }
    const stamp = () => {
      try {
        localStorage.setItem(visitKey, new Date().toISOString())
      } catch {
        undefined
      }
    }
    window.addEventListener('pagehide', stamp)
    return () => {
      window.removeEventListener('pagehide', stamp)
      stamp()
    }
  }, [visitKey])

  const userViews = useMemo(() => coerceViews(storedViews), [storedViews])
  const views = useMemo(() => [ALL_VIEW, ...userViews], [userViews])
  const activeView = views.find((view) => view.id === activeViewId) ?? ALL_VIEW
  const filters = useMemo(() => coerceFilters(storedFilters), [storedFilters])
  const layout = storedLayout === 'list' ? 'list' : 'board'
  const group: GroupBy = layout === 'board' ? coerceGroup(storedGroup) : 'none'
  const columns = data?.columns ?? []
  const projects = data?.projects ?? []
  const releases = data?.releases ?? []
  const doneStatus = data?.done_status || columns[columns.length - 1]?.id || ''
  const canceledStatus = data?.canceled_status ?? null
  const visibleLane = columns.find((column) => column.id === activeLane) ?? columns[0] ?? null
  // A narrow pane shows one lane, so New ticket starts in that lane (the lane header with its own add is gone).
  const laneNew = narrow && layout === 'board' ? visibleLane : null
  const newStatus = laneNew?.id ?? data?.default_status ?? 'backlog'
  const newLabel = laneNew ? `New ticket in ${laneNew.label}` : 'New ticket'
  const tickets = useMemo(() => (data?.columns ?? []).flatMap((column) => column.tickets), [data])
  // `now` is re-read whenever the board data changes.
  const ctx = useMemo<MatchContext>(
    () => ({ lastVisit, now: Date.now(), own: OWN_WRITES }),
    [lastVisit, data, ownVersion],
  )
  const shown = useMemo(() => applyFilters(tickets, filters, ctx), [tickets, filters, ctx])
  const shownIds = useMemo(() => new Set(shown.map((ticket) => ticket.id)), [shown])
  const filtering = !isEmpty(filters)
  // Subtasks and their parents are read off the whole board, not the filtered one: "2/12" does not change with a search.
  const subtasks = useMemo(
    () => subtaskStats(tickets, doneStatus, canceledStatus),
    [tickets, doneStatus, canceledStatus],
  )
  const keyById = useMemo(() => new Map(tickets.map((ticket) => [ticket.id, ticket.key])), [tickets])
  const lanes = useMemo(
    () =>
      group === 'none'
        ? []
        : buildSwimlanes({
            by: group,
            columns: data?.columns ?? [],
            shownIds,
            releases,
            projects,
            doneStatus,
            canceledStatus,
            showEmpty,
          }),
    [group, data, shownIds, releases, projects, doneStatus, canceledStatus, showEmpty],
  )
  const laneById = useMemo(() => new Map(flattenLanes(lanes).map((lane) => [lane.id, lane])), [lanes])
  // The ids the swimlanes stand for: a ticket that names any other sits in the "No …" lane, whatever it names.
  const known = useMemo(
    () => (group === 'none' ? new Set<string>() : knownLaneIds(group, tickets, releases, projects)),
    [group, tickets, releases, projects],
  )
  /** Why a card cannot be dropped in a swimlane (null = it can): the lane header and the drop both ask this. */
  const refuseDrop = useCallback(
    (ticket: TicketSummary, lane: Swimlane) =>
      group === 'none' ? null : dropRefusal(lane, ticket, laneIdOf(ticket, group, known), subtasks.has(ticket.id)),
    [group, known, subtasks],
  )
  const isFolded = useCallback(
    (lane: Swimlane) =>
      (folded && typeof folded === 'object' ? folded[lane.id] : undefined) ??
      (lane.state === 'planned' || lane.kind === 'group'),
    [folded],
  )
  const toggleLane = useCallback(
    (lane: Swimlane) => setFolded({ ...folded, [lane.id]: !isFolded(lane) }),
    [folded, isFolded, setFolded],
  )
  const edited = !sameFilters(filters, activeView.filters)
  const viewCounts = useMemo(
    () => Object.fromEntries(views.map((view) => [view.id, applyFilters(tickets, view.filters, ctx).length])),
    [views, tickets, ctx],
  )

  const selectView = (view: SavedView) => {
    setActiveViewId(view.id)
    setFilters(view.filters)
  }

  /** Write the list now (the other panes read it back from storage), then tell them. */
  const writeViews = (next: SavedView[]) => {
    try {
      localStorage.setItem(VIEWS_KEY, JSON.stringify(next))
    } catch {
      undefined
    }
    setStoredViews(next)
    window.dispatchEvent(new CustomEvent(VIEWS_EVENT))
  }

  const saveView = (name: string) => {
    const view: SavedView = { id: `v${Date.now().toString(36)}`, name, filters }
    writeViews([...(readViews() ?? userViews), view])
    setActiveViewId(view.id)
  }

  const deleteView = (id: string) => {
    writeViews((readViews() ?? userViews).filter((view) => view.id !== id))
    if (activeView.id === id) selectView(ALL_VIEW)
  }

  // Views saved or deleted by another pane of this document (event) or by another tab (storage).
  useEffect(() => {
    const sync = () => {
      const next = readViews()
      if (next) setStoredViews(next)
    }
    const onStorage = (event: StorageEvent) => {
      if (event.key === VIEWS_KEY || event.key === null) sync()
    }
    window.addEventListener(VIEWS_EVENT, sync)
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener(VIEWS_EVENT, sync)
      window.removeEventListener('storage', onStorage)
    }
  }, [setStoredViews])

  const publish = useCallback((next: BoardData) => {
    dataRef.current = next
    setData(next)
  }, [])

  // While a board read or a move is in flight, the tickets that arrive are also kept here, in order, so a
  // read that started before them can have them replayed over it. Cleared when nothing is in flight.
  const journal = useRef<Write[]>([])
  const pending = useRef(0)
  const begin = useCallback(() => {
    pending.current += 1
    return journal.current.length
  }, [])
  const end = useCallback(() => {
    pending.current -= 1
    if (pending.current === 0) journal.current = []
  }, [])

  // A card dropped into another swimlane is written in two calls (its release, project or parent, then its place).
  // Until the second lands, what the first one reports still carries the old status: keep the card where it was dropped.
  const held = useRef(new Map<string, { status: string; order: number }>())

  /** A ticket as the worker has it (event, reply, bulk result): applied unless the board is already past it. */
  const write = useCallback(
    (reported: TicketSummary, kind: WriteKind = {}) => {
      const hold = held.current.get(reported.id)
      const summary = hold && !kind.moved ? { ...reported, ...hold } : reported
      if (pending.current > 0) journal.current.push({ summary, kind })
      const current = dataRef.current
      if (!current) return
      const next = applyWrite(current, summary, tombstones.current, kind)
      if (next !== current) publish(next)
    },
    [publish],
  )

  const loadSeq = useRef(0)
  /** Read the board. Only the newest read is applied; `quiet` reads show no spinner or error. */
  const load = useCallback(
    async (quiet = false): Promise<'applied' | 'superseded' | 'failed'> => {
      const token = ++loadSeq.current
      const from = begin()
      if (!quiet) {
        setLoading(true)
        setLoadError(null)
      }
      try {
        const board = await api.board(host.iii)
        if (token !== loadSeq.current) return 'superseded'
        publish(applyRead(board, journal.current.slice(from), tombstones.current))
        setLoadError(null)
        return 'applied'
      } catch (error) {
        if (token !== loadSeq.current) return 'superseded'
        if (!quiet) setLoadError(errorMessage(error))
        return 'failed'
      } finally {
        end()
        if (token === loadSeq.current) setLoading(false)
      }
    },
    [host, begin, end, publish],
  )

  useEffect(() => {
    void load()
  }, [load])

  // Project and release edits fire no ticket event: read the board again when the pane comes back into view,
  // and when a Releases or Projects page of this tab announces a write.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') void load(true)
    }
    const onPlan = () => void load(true)
    document.addEventListener('visibilitychange', onVisible)
    window.addEventListener(PLAN_CHANGED_EVENT, onPlan)
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      window.removeEventListener(PLAN_CHANGED_EVENT, onPlan)
    }
  }, [load])

  // The Releases and Projects pages open the board on a release or project:
  // `{ group, release | project, layout }` (`project: ''` is "No project").
  useEffect(() => {
    const context = panelContext?.context as
      | { group?: unknown; release?: unknown; project?: unknown; layout?: unknown }
      | undefined
    if (!context || typeof context !== 'object') return
    // Once per request: a remount, or another board pane, gets the same context again.
    if (panelContext && handledContexts.has(panelContext.id)) return
    if (panelContext) handledContexts.add(panelContext.id)
    if (context.layout === 'list') setLayout('list')
    if (context.group === 'release' || context.group === 'project' || context.group === 'parent') {
      setLayout('board')
      setGroup(context.group)
    }
    const release = typeof context.release === 'string' ? context.release : null
    const project = typeof context.project === 'string' ? context.project : null
    if (release) {
      // The lane (planned ones are folded by default) and the group a released release sits under: nothing to
      // see otherwise. The group only exists under the filter below when the release is released.
      setFolded((prev) => ({
        ...(prev && typeof prev === 'object' ? prev : {}),
        'release:released': false,
        [`release:${release}`]: false,
      }))
    }
    if (release !== null || project !== null) {
      setActiveViewId(ALL_VIEW.id)
      setFilters({
        ...EMPTY_FILTERS,
        releases: release !== null ? [release] : [],
        projects: project !== null ? [project] : [],
      })
    }
  }, [panelContext, setActiveViewId, setFilters, setFolded, setGroup, setLayout])

  useEffect(() => {
    let cancelled = false
    void api
      .agents(host.iii)
      .then((result) => {
        if (!cancelled) setAgents(result.agents ?? [])
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [host])

  // A ticket that names a project or release this board has not heard of (created a moment ago, elsewhere):
  // read the board once more, after the burst of events that usually comes with it.
  const refetch = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(refetch.current), [])

  useEffect(() => {
    return subscribeChanges(host.iii, (event) => {
      if (!event?.ticket) return
      write(summarizeTicket(event.ticket), {
        moved: event.event === 'ticket.moved',
        restored: event.event === 'ticket.restored',
      })
      const board = dataRef.current
      const { project_id: project, release_id: release } = event.ticket
      if (
        board &&
        ((project && !board.projects.some((p) => p.id === project)) ||
          (release && !board.releases.some((r) => r.id === release)))
      ) {
        window.clearTimeout(refetch.current)
        refetch.current = window.setTimeout(() => void load(true), 400)
      }
    })
  }, [host, write, load])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-ticket',
          title: 'New ticket',
          enabled: () => inlineTicketId === null,
          run: () => setCreating({ status: newStatus }),
        },
        { id: 'refresh', title: 'Refresh board', run: () => void load() },
        {
          id: 'clear-filters',
          title: 'Clear filters',
          enabled: () => !isEmpty(filters),
          run: () => setFilters(EMPTY_FILTERS),
        },
      ]),
    [commands, newStatus, filters, inlineTicketId, load, setFilters],
  )

  const clearDrag = useCallback(() => {
    dragSeq.current += 1
    dragRef.current = null
    setDrag(null)
    setDropTarget(null)
  }, [])

  // The ticket being dragged was deleted meanwhile: there is nothing left to drop.
  useEffect(() => {
    if (drag && !tickets.some((ticket) => ticket.id === drag.id)) clearDrag()
  }, [drag, tickets, clearDrag])

  /** Keep where the board is scrolled while a ticket takes its place, and put it back on return. */
  const openInline = useCallback((key: string) => {
    const root = boardEl.current
    if (root) {
      const swim = root.querySelector('.kanban-swimlanes, .kanban-sections')
      scrollMemo.current = {
        board: root.scrollTop,
        strip: root.querySelector('.kanban-lanes')?.scrollLeft ?? 0,
        lanes: Object.fromEntries(
          Array.from(root.querySelectorAll<HTMLElement>('[data-lane]'), (lane) => [
            lane.dataset.lane ?? '',
            lane.querySelector('.kanban-lane__body')?.scrollTop ?? 0,
          ]),
        ),
        swim: { top: swim?.scrollTop ?? 0, left: swim?.scrollLeft ?? 0 },
      }
    }
    setInlineTicketId(key)
  }, [])

  useLayoutEffect(() => {
    const root = boardEl.current
    const saved = scrollMemo.current
    if (inlineTicketId !== null || !root || !saved) return
    scrollMemo.current = null
    root.scrollTop = saved.board
    const strip = root.querySelector('.kanban-lanes')
    if (strip) strip.scrollLeft = saved.strip
    root.querySelectorAll<HTMLElement>('[data-lane]').forEach((lane) => {
      const body = lane.querySelector('.kanban-lane__body')
      if (body) body.scrollTop = saved.lanes[lane.dataset.lane ?? ''] ?? 0
    })
    const swim = root.querySelector('.kanban-swimlanes, .kanban-sections')
    if (swim) {
      swim.scrollTop = saved.swim.top
      swim.scrollLeft = saved.swim.left
    }
  }, [inlineTicketId])

  const openTicket = useCallback(
    (id: string) => {
      if (canOpenPane) {
        host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id } })
        return
      }
      openInline(id)
    },
    [canOpenPane, host, openInline],
  )

  /**
   * Put a card in a status at a position; `facet` also moves it to another release, project or parent (`value`
   * null = none, `name` the swimlane): that is its own write, then the move, so the history says what happened.
   */
  const applyMove = useCallback(
    (
      id: string,
      status: string,
      index: number,
      facet?: { key: 'release' | 'project' | 'parent'; value: string | null; name: string },
    ) => {
      const before = dataRef.current
      const moving = before?.columns.flatMap((column) => column.tickets).find((ticket) => ticket.id === id)
      if (!before || !moving) return
      // Optimistic: the worker renumbers the target lane (and compacts the one the card left); so do we.
      const placed = { ...moving, status, order: index + 1, ...(facet ? { [`${facet.key}_id`]: facet.value } : {}) }
      publish({ ...before, columns: upsertSummary(before.columns, placed, true) })
      const from = begin()
      // A hold left by an earlier drop of this card (still being written) stays up until this one is done as well.
      const hold = { status, order: index + 1 }
      if (facet || held.current.has(id)) held.current.set(id, hold)
      // The card as the worker has it once its swimlane is saved: if the move then fails, that is what stays.
      let relocated: Ticket | null = null
      const move = () => api.move(host.iii, { id, status, index, actor: 'user' })
      const save = () =>
        Promise.resolve()
          .then(() =>
            facet
              ? api.update(host.iii, { id, [facet.key]: facet.value, actor: 'user' }).then((updated) => {
                  relocated = updated
                  noteOwnWrite(updated)
                  return move()
                })
              : move(),
          )
          .then((ticket) => {
            setActionError(null)
            noteOwnWrite(ticket)
            write(summarizeTicket(ticket), { moved: true })
            // Another ticket changed while this move was pending (its place was drawn against our guess), or the
            // worker found nothing to change: read the board once more. Otherwise the drawn lane is the real one.
            const crowded = journal.current.slice(from).some((entry) => entry.summary.id !== id)
            if (crowded || Date.parse(ticket.updated_at) <= Date.parse(moving.updated_at)) void load(true)
          })
          .catch((error) => {
            const saved = relocated
            if (saved && facet) {
              setNotice({
                headline: 'That change was saved in part',
                detail: `Moved to ${facet.name}, but the status change failed: ${errorMessage(error)}`,
              })
            } else {
              setActionError(errorMessage(error))
            }
            // If the board cannot be read either, show it as it was before the move rather than a move that never
            // happened (keeping the swimlane the worker did save).
            void load().then((result) => {
              if (result === 'failed') {
                publish(saved ? { ...before, columns: upsertSummary(before.columns, summarizeTicket(saved)) } : before)
              }
            })
          })
          .finally(() => {
            if (held.current.get(id) === hold) held.current.delete(id)
            end()
          })
      // After the writes of the card's earlier drops, so the last drop is the last write. `save` never rejects.
      const job = (moveTails.get(id) ?? Promise.resolve()).then(save)
      moveTails.set(id, job)
      void job.then(() => {
        if (moveTails.get(id) === job) moveTails.delete(id)
      })
    },
    [host, load, publish, begin, end, write, setActionError],
  )

  const handleDragStart = useCallback(
    (ticket: TicketSummary, event: DragEvent<HTMLElement>) => {
      const source = event.currentTarget
      event.dataTransfer.effectAllowed = 'move'
      event.dataTransfer.setData(DRAG_TYPE, ticket.id)
      event.dataTransfer.setData('text/plain', ticket.id)
      const height = source.getBoundingClientRect().height
      const token = ++dragSeq.current
      // The browser sends dragend to the node the drag started on, even when React has replaced it since (the
      // card was moved to another lane by someone else): listen there, not through React or window.
      source.addEventListener(
        'dragend',
        () => {
          if (dragSeq.current === token) clearDrag()
        },
        { once: true },
      )
      window.setTimeout(() => {
        if (dragSeq.current !== token) return
        const state = { id: ticket.id, from: ticket.status, height }
        dragRef.current = state
        setDrag(state)
      }, 0)
    },
    [clearDrag],
  )

  /** `lane` is the swimlane the cell belongs to, null on a board without swimlanes (one cell per status). */
  const handleDragOver = useCallback((lane: string | null, status: string, event: DragEvent<HTMLElement>) => {
    const dragged = dragRef.current
    if (!dragged || !carriesTicket(event)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'move'
    const cards = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('[data-ticket-card]'))
    const others = cards.filter((node) => node.dataset.ticketId !== dragged.id)
    const at = cards.findIndex((node) => node.dataset.ticketId === dragged.id)
    const ids = others.map((node) => node.dataset.ticketId ?? '')
    let index = others.length
    for (let i = 0; i < others.length; i += 1) {
      const rect = others[i].getBoundingClientRect()
      if (event.clientY < rect.top + rect.height / 2) {
        index = i
        break
      }
    }
    setDropTarget((current) =>
      current &&
      current.lane === lane &&
      current.status === status &&
      current.index === index &&
      current.at === at &&
      current.ids.join() === ids.join()
        ? current
        : { lane, status, index, ids, at },
    )
  }, [])

  const handleDragLeave = useCallback((event: DragEvent<HTMLElement>) => {
    const next = event.relatedTarget as Node | null
    if (next && event.currentTarget.contains(next)) return
    setDropTarget(null)
  }, [])

  const handleDrop = useLatest((lane: string | null, status: string, event: DragEvent<HTMLElement>) => {
    const dragged = dragRef.current
    if (!dragged || !carriesTicket(event)) return
    event.preventDefault()
    const target = dropTarget
    clearDrag()
    if (!target || target.status !== status || target.lane !== lane) return
    const ticket = tickets.find((t) => t.id === dragged.id)
    if (!ticket) return
    const swim = lane === null ? undefined : laneById.get(lane)
    if (swim && refuseDrop(ticket, swim)) return
    // Dropped between the same visible neighbours it already has: nothing to write.
    if (target.at === target.index) return
    // The drop index counts the cards drawn in the cell; the server wants it among the whole column.
    const index = fullIndex(
      columns.find((column) => column.id === status)?.tickets ?? [],
      target.ids,
      target.index,
      ticket.id,
    )
    // Another swimlane: also set its release, project or parent ("No release" clears it).
    const facet = swim ? laneChange(ticket, swim, known) : undefined
    // The card may land past "Show N more": open the cell it was dropped in.
    if (lane !== null) setOpenCells((cells) => new Set(cells).add(cellKey(lane, status)))
    applyMove(ticket.id, status, index, facet)
  })

  const deletedCount = data?.deleted_count ?? 0

  // Counted from the lanes: the worker's total also holds tickets whose status is no column.
  const headerDescription = useMemo(() => {
    if (!data) return 'Board'
    const base = `${tickets.length} ${tickets.length === 1 ? 'ticket' : 'tickets'} · ${data.id_prefix}`
    return deletedCount > 0 ? `${base} · ${deletedCount} deleted kept on disk` : base
  }, [data, tickets, deletedCount])

  const priorities = useMemo(() => data?.priorities ?? [], [data?.priorities])
  const dragged = useMemo(() => (drag ? tickets.find((ticket) => ticket.id === drag.id) : undefined), [drag, tickets])
  const visibleCards = (column: BoardColumn) => column.tickets.filter((ticket) => shownIds.has(ticket.id))
  const laneCount = (column: BoardColumn) =>
    filtering ? `${visibleCards(column).length} of ${column.tickets.length}` : String(column.tickets.length)

  // Everything below up to the swimlane props keeps its identity while a card is dragged over the board, so the
  // memoised cards, cells and swimlanes are only redrawn where the drop target moves.
  const renderCard = useCallback(
    (ticket: TicketSummary, dragging: boolean) => (
      <TicketCard
        agent={agents.find((a) => a.id === ticket.assignee)}
        canOpenPane={canOpenPane}
        changed={isChangedSinceVisit(ticket, ctx)}
        doneStatus={doneStatus}
        dragging={dragging}
        highlight={filters.labels}
        now={ctx.now}
        onDragStart={handleDragStart}
        onOpen={openTicket}
        onOpenHere={openInline}
        // A parent the board does not hold (deleted) is not named: its key would lead nowhere.
        parentKey={ticket.parent_id ? keyById.get(ticket.parent_id) : undefined}
        priorities={priorities}
        subs={subtasks.get(ticket.id)}
        ticket={ticket}
      />
    ),
    [
      agents,
      canOpenPane,
      ctx,
      doneStatus,
      filters.labels,
      handleDragStart,
      keyById,
      openInline,
      openTicket,
      priorities,
      subtasks,
    ],
  )

  const refuseFor = useCallback((lane: Swimlane) => (dragged ? refuseDrop(dragged, lane) : null), [dragged, refuseDrop])
  const openLane = useCallback(
    (lane: Swimlane) => {
      if (lane.facet === 'parent') openTicket(lane.parent?.key ?? '')
      else
        host.panels?.open({
          pageId: lane.facet === 'release' ? RELEASES_PAGE_ID : PROJECTS_PAGE_ID,
          context: { id: lane.value },
        })
    },
    [host, openTicket],
  )
  const onExpand = useCallback((cell: string) => setOpenCells((cells) => new Set(cells).add(cell)), [])
  const onAdd = useCallback((lane: Swimlane, column: BoardColumn) => setCreating(newIn(lane, column.id)), [])
  const onSwimDragOver = useCallback(
    (lane: Swimlane, status: string, event: DragEvent<HTMLElement>) => handleDragOver(lane.id, status, event),
    [handleDragOver],
  )
  const onSwimDrop = useCallback(
    (lane: Swimlane, status: string, event: DragEvent<HTMLElement>) => handleDrop(lane.id, status, event),
    [handleDrop],
  )
  const swimStatusLabel = useCallback((status: string) => statusLabel(columns, status), [columns])

  if (inlineTicketId) {
    return (
      <TicketScreen
        key={inlineTicketId}
        host={host}
        ticketId={inlineTicketId}
        doneStatus={doneStatus}
        canceledStatus={canceledStatus}
        columns={columns}
        priorities={priorities}
        agents={agents}
        onBack={() => setInlineTicketId(null)}
        onOpenTicket={(key: string) => setInlineTicketId(key)}
        onRequestClose={onRequestClose}
        commands={commands}
      />
    )
  }

  const hovered = dropTarget?.lane ? laneById.get(dropTarget.lane) : undefined
  const dragMessage = !dragged
    ? ''
    : hovered && dropTarget
      ? `Drop to move ${dragged.key} to ${statusLabel(columns, dropTarget.status)} in ${hovered.short}`
      : `Moving ${dragged.key}: drop it on a status in a swimlane`

  const swimProps = {
    lanes,
    columns,
    collapsed: isFolded,
    onToggle: toggleLane,
    expanded: openCells,
    onExpand,
    drag,
    dropTarget,
    refuseFor,
    renderCard,
    onDragOver: onSwimDragOver,
    onDragLeave: handleDragLeave,
    onDrop: onSwimDrop,
    onAdd,
    onOpenLane: canOpenPane ? openLane : undefined,
    statusLabel: swimStatusLabel,
    now: ctx.now,
    empty: filtering ? 'No ticket here matches the filters.' : 'No tickets to group yet.',
  }

  const renderLane = (column: BoardColumn, head: boolean) => {
    const isDropping = dropTarget?.lane === null && dropTarget.status === column.id && drag !== null
    const cards = visibleCards(column)
    // dropTarget.index counts the visible cards without the dragged one.
    const others = cards.filter((ticket) => ticket.id !== drag?.id)
    return (
      <section
        aria-label={`${column.label}, ${laneCount(column)} ${column.tickets.length === 1 ? 'ticket' : 'tickets'}`}
        className="kanban-lane"
        data-drop={isDropping ? 'true' : undefined}
        data-lane={column.id}
        key={column.id}
        onDragLeave={handleDragLeave}
        onDragOver={(event) => handleDragOver(null, column.id, event)}
        onDrop={(event) => handleDrop(null, column.id, event)}
      >
        {head ? (
          <header className="kanban-lane__head">
            <StatusGlyph canceledStatus={canceledStatus} columns={columns} doneStatus={doneStatus} status={column.id} />
            <span className="kanban-lane__title">{column.label}</span>
            <span className="kanban-lane__count">{laneCount(column)}</span>
            <IconButton
              className="kanban-lane__add"
              label={`New ticket in ${column.label}`}
              onClick={() => setCreating({ status: column.id })}
              variant="ghost"
            >
              <Plus size={16} />
            </IconButton>
          </header>
        ) : null}
        <div className="kanban-lane__body">
          {cards.length === 0 && !isDropping ? (
            filtering ? (
              <p className="kanban-lane__none">No ticket here matches the filters.</p>
            ) : (
              <button className="kanban-lane__empty" onClick={() => setCreating({ status: column.id })} type="button">
                No tickets — add one
              </button>
            )
          ) : null}
          {cards.map((ticket) => (
            <div key={ticket.id}>
              {isDropping && dropTarget && others[dropTarget.index]?.id === ticket.id ? (
                <div className="kanban-placeholder" style={{ height: drag?.height ?? 58 }} />
              ) : null}
              {renderCard(ticket, drag?.id === ticket.id)}
            </div>
          ))}
          {isDropping && dropTarget && dropTarget.index >= others.length ? (
            <div className="kanban-placeholder" style={{ height: drag?.height ?? 58 }} />
          ) : null}
        </div>
      </section>
    )
  }

  return (
    <PageShell className="kanban-shell">
      <PageHeader
        actions={
          <>
            <IconButton
              className="kanban-header-action"
              data-narrow={narrow ? 'true' : undefined}
              label="Refresh board"
              onClick={() => void load()}
              variant="ghost"
            >
              <RefreshCw size={16} />
            </IconButton>
            {narrow ? (
              <IconButton
                className="kanban-header-action"
                data-narrow="true"
                label={newLabel}
                onClick={() => setCreating({ status: newStatus })}
                variant="primary"
              >
                <Plus size={16} />
              </IconButton>
            ) : (
              <Button onClick={() => setCreating({ status: newStatus })} variant="primary">
                <Plus size={16} />
                New ticket
              </Button>
            )}
          </>
        }
        description={headerDescription}
        icon={<Columns2 size={16} />}
        onClose={onRequestClose}
        title="Kanban"
      />
      <PageMain>
        <div
          className="kanban-board"
          data-autofocus=""
          data-narrow={narrow ? 'true' : undefined}
          ref={setBoardEl}
          tabIndex={-1}
        >
          {loadError ? (
            <div className="kanban-pad">
              <StatusPanel
                action={
                  <Button onClick={() => void load()} size={narrow ? 'lg' : 'md'} variant="pill">
                    Retry
                  </Button>
                }
                detail={loadError}
                headline="The board could not be read"
                icon={<TriangleAlert size={16} />}
                role="alert"
                variant="alert"
              />
            </div>
          ) : null}

          {notice ? (
            <div className="kanban-pad">
              <StatusPanel
                action={
                  <Button onClick={() => setActionError(null)} size={narrow ? 'lg' : 'md'} variant="pill">
                    Dismiss
                  </Button>
                }
                detail={notice.detail}
                headline={notice.headline}
                icon={<TriangleAlert size={16} />}
                role="alert"
                variant="alert"
              />
            </div>
          ) : null}

          {loading && !data ? (
            <div className="kanban-lanes">
              {[0, 1, 2, 3, 4].map((index) => (
                <section className="kanban-lane" key={index}>
                  <header className="kanban-lane__head">
                    <Skeleton className="kanban-skeleton kanban-skeleton--title" />
                  </header>
                  <div className="kanban-lane__body">
                    <Skeleton className="kanban-skeleton kanban-skeleton--card" />
                    <Skeleton className="kanban-skeleton kanban-skeleton--card" />
                  </div>
                </section>
              ))}
            </div>
          ) : null}

          {data && columns.length === 0 ? (
            <div className="kanban-pad">
              <EmptyState
                action={{ label: 'New ticket', onClick: () => setCreating({ status: data.default_status }) }}
                description="The kanban configuration declares no columns yet. Add one in settings."
                icon={Columns2}
                title="No columns configured"
              />
            </div>
          ) : null}

          {data && columns.length > 0 ? (
            <>
              <ViewTabs
                activeId={activeView.id}
                counts={viewCounts}
                edited={edited}
                layout={layout}
                narrow={narrow}
                onDelete={deleteView}
                onLayoutChange={setLayout}
                onSelect={selectView}
                views={views}
              />
              <FilterBar
                agents={agents}
                canSaveView={edited && filtering}
                ctx={ctx}
                filters={filters}
                narrow={narrow}
                onChange={setFilters}
                onSaveView={saveView}
                priorities={priorities}
                projects={projects}
                releases={releases}
                shownCount={shown.length}
                tickets={tickets}
                group={
                  layout === 'board'
                    ? { value: group, onChange: setGroup, showEmpty, onShowEmptyChange: setShowEmpty }
                    : undefined
                }
              />
            </>
          ) : null}

          {data && columns.length > 0 && layout === 'list' ? (
            <TicketList
              agents={agents}
              columns={columns}
              ctx={ctx}
              highlightLabels={filters.labels}
              host={host}
              labels={allLabels(tickets)}
              narrow={narrow}
              onError={setActionError}
              onOpenTicket={openTicket}
              doneStatus={doneStatus}
              canceledStatus={canceledStatus}
              projects={projects}
              releases={releases}
              subtasks={subtasks}
              onUpdated={(updated: Ticket[]) => {
                setActionError(null)
                for (const ticket of updated) {
                  noteOwnWrite(ticket)
                  write(summarizeTicket(ticket))
                }
              }}
              priorities={priorities}
              tickets={shown}
            />
          ) : null}

          {data && columns.length > 0 && layout === 'board' && narrow ? (
            <>
              <Tabs onValueChange={setActiveLane} value={visibleLane?.id ?? columns[0].id}>
                <TabsList className="kanban-lane-tabs">
                  {columns.map((column) => (
                    <TabsTrigger icon={false} key={column.id} value={column.id}>
                      {column.label}
                      <span className="kanban-lane__count">{laneCount(column)}</span>
                    </TabsTrigger>
                  ))}
                </TabsList>
              </Tabs>
              {group !== 'none' && drag ? <DragLine message={dragMessage} /> : null}
              {group === 'none' ? (
                <div className="kanban-lanes kanban-lanes--single">
                  {visibleLane ? renderLane(visibleLane, false) : null}
                </div>
              ) : visibleLane ? (
                <SwimlaneSections {...swimProps} column={visibleLane} showEmpty={showEmpty} />
              ) : null}
            </>
          ) : null}

          {data && columns.length > 0 && layout === 'board' && !narrow && group === 'none' ? (
            <div className="kanban-lanes">{columns.map((column) => renderLane(column, true))}</div>
          ) : null}

          {data && columns.length > 0 && layout === 'board' && !narrow && group !== 'none' ? (
            <>
              {drag ? <DragLine message={dragMessage} /> : null}
              <SwimlaneGrid
                {...swimProps}
                canceledStatus={canceledStatus}
                doneStatus={doneStatus}
                columnCount={laneCount}
                onAddInColumn={(column) => setCreating({ status: column.id })}
              />
            </>
          ) : null}
        </div>
      </PageMain>

      <CreateTicketDialog
        agents={agents}
        columns={columns}
        defaultStatus={creating?.status ?? data?.default_status ?? 'backlog'}
        defaults={creating ?? undefined}
        host={host}
        onCreated={(ticket) => {
          setActionError(null)
          noteOwnWrite(ticket)
          setCreating(null)
          write(summarizeTicket(ticket))
          openTicket(ticket.key)
        }}
        onOpenChange={(open) => {
          if (!open) setCreating(null)
        }}
        open={creating !== null}
        priorities={data?.priorities ?? ['low', 'medium', 'high', 'urgent']}
      />
    </PageShell>
  )
}
