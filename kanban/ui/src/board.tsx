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
import { Bot, CircleDashed, Columns2, Maximize2, MessageCircle, Plus, RefreshCw, TriangleAlert } from 'lucide-react'
import { type DragEvent, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { applyRead, applyWrite, type Tombstones, upsertSummary, type Write, type WriteKind } from './board-state'
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
  PriorityChip,
  StatusGlyph,
  subscribeChanges,
  summarizeTicket,
  TICKET_PAGE_ID,
  type Ticket,
  type TicketSummary,
  useOwnWritesVersion,
} from './shared'
import { TicketScreen } from './ticket'

type DragState = { id: string; from: string; height: number }

/** Set by dragstart: only a drag that carries it is a ticket drag (not a file, a selection, another pane's card). */
const DRAG_TYPE = 'application/x-kanban-ticket'
const carriesTicket = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes(DRAG_TYPE)

const VIEWS_KEY = 'kanban:views'
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
type Scroll = { board: number; strip: number; lanes: Record<string, number> }

export function BoardPage({ host, onRequestClose, paneId, tabId, commands }: PageRenderProps & { host: Host }) {
  const [data, setData] = useState<BoardData | null>(null)
  // The latest board, readable between renders: every write is applied to it, not to a stale closure.
  const dataRef = useRef<BoardData | null>(null)
  const tombstones = useRef<Tombstones>(new Map())
  const [agents, setAgents] = useState<AgentProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [creating, setCreating] = useState<string | null>(null)
  const [inlineTicketId, setInlineTicketId] = useState<string | null>(null)
  const [drag, setDrag] = useState<DragState | null>(null)
  const [dropTarget, setDropTarget] = useState<{ status: string; index: number } | null>(null)
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
  const columns = data?.columns ?? []
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

  /** A ticket as the worker has it (event, reply, bulk result): applied unless the board is already past it. */
  const write = useCallback(
    (summary: TicketSummary, kind: WriteKind = {}) => {
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

  useEffect(() => {
    return subscribeChanges(host.iii, (event) => {
      if (!event?.ticket) return
      write(summarizeTicket(event.ticket), {
        moved: event.event === 'ticket.moved',
        restored: event.event === 'ticket.restored',
      })
    })
  }, [host, write])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-ticket',
          title: 'New ticket',
          enabled: () => inlineTicketId === null,
          run: () => setCreating(newStatus),
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
      scrollMemo.current = {
        board: root.scrollTop,
        strip: root.querySelector('.kanban-lanes')?.scrollLeft ?? 0,
        lanes: Object.fromEntries(
          Array.from(root.querySelectorAll<HTMLElement>('[data-lane]'), (lane) => [
            lane.dataset.lane ?? '',
            lane.querySelector('.kanban-lane__body')?.scrollTop ?? 0,
          ]),
        ),
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

  const applyMove = useCallback(
    (id: string, status: string, index: number) => {
      const before = dataRef.current
      const moving = before?.columns.flatMap((column) => column.tickets).find((ticket) => ticket.id === id)
      if (!before || !moving) return
      // Optimistic: the worker renumbers the target lane (and compacts the one the card left); so do we.
      publish({ ...before, columns: upsertSummary(before.columns, { ...moving, status, order: index + 1 }, true) })
      const from = begin()
      void api
        .move(host.iii, { id, status, index, actor: 'user' })
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
          setActionError(errorMessage(error))
          // If the board cannot be read either, show it as it was before the move rather than a move that never happened.
          void load().then((result) => {
            if (result === 'failed') publish(before)
          })
        })
        .finally(end)
    },
    [host, load, publish, begin, end, write],
  )

  const handleDragStart = (ticket: TicketSummary, event: DragEvent<HTMLElement>) => {
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
  }

  const handleDragOver = (status: string, event: DragEvent<HTMLElement>) => {
    const dragged = dragRef.current
    if (!dragged || !carriesTicket(event)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'move'
    const cards = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('[data-ticket-card]'))
    const others = cards.filter((node) => node.dataset.ticketId !== dragged.id)
    let index = others.length
    for (let i = 0; i < others.length; i += 1) {
      const rect = others[i].getBoundingClientRect()
      if (event.clientY < rect.top + rect.height / 2) {
        index = i
        break
      }
    }
    setDropTarget((current) =>
      current && current.status === status && current.index === index ? current : { status, index },
    )
  }

  const handleDragLeave = (event: DragEvent<HTMLElement>) => {
    const next = event.relatedTarget as Node | null
    if (next && event.currentTarget.contains(next)) return
    setDropTarget(null)
  }

  /** The drop index counts visible cards; the server wants it among the whole column. */
  const toFullIndex = (status: string, visibleIndex: number, draggedId: string) => {
    const full = (columns.find((column) => column.id === status)?.tickets ?? []).filter((t) => t.id !== draggedId)
    const visible = full.filter((t) => shownIds.has(t.id))
    if (visibleIndex < visible.length) return full.indexOf(visible[visibleIndex])
    const last = visible[visible.length - 1]
    return last ? full.indexOf(last) + 1 : full.length
  }

  const handleDrop = (status: string, event: DragEvent<HTMLElement>) => {
    const dragged = dragRef.current
    if (!dragged || !carriesTicket(event)) return
    event.preventDefault()
    const target = dropTarget
    clearDrag()
    if (!target || target.status !== status) return
    const ticket = tickets.find((t) => t.id === dragged.id)
    if (!ticket) return
    // Dropped between the same visible neighbours it already has: nothing to write.
    const lane = columns.find((column) => column.id === status)
    const at = lane ? lane.tickets.filter((t) => shownIds.has(t.id)).findIndex((t) => t.id === ticket.id) : -1
    if (ticket.status === status && at === target.index) return
    applyMove(ticket.id, status, toFullIndex(status, target.index, ticket.id))
  }

  const deletedCount = data?.deleted_count ?? 0

  // Counted from the lanes: the worker's total also holds tickets whose status is no column.
  const headerDescription = useMemo(() => {
    if (!data) return 'Board'
    const base = `${tickets.length} ${tickets.length === 1 ? 'ticket' : 'tickets'} · ${data.id_prefix}`
    return deletedCount > 0 ? `${base} · ${deletedCount} deleted kept on disk` : base
  }, [data, tickets, deletedCount])

  if (inlineTicketId) {
    return (
      <TicketScreen
        key={inlineTicketId}
        host={host}
        ticketId={inlineTicketId}
        columns={columns}
        priorities={data?.priorities ?? []}
        agents={agents}
        onBack={() => setInlineTicketId(null)}
        onOpenTicket={(key: string) => setInlineTicketId(key)}
        onRequestClose={onRequestClose}
        commands={commands}
      />
    )
  }

  const priorities = data?.priorities ?? []
  const visibleCards = (column: BoardColumn) => column.tickets.filter((ticket) => shownIds.has(ticket.id))
  const laneCount = (column: BoardColumn) =>
    filtering ? `${visibleCards(column).length} of ${column.tickets.length}` : String(column.tickets.length)

  const renderCard = (ticket: TicketSummary) => {
    const changed = isChangedSinceVisit(ticket, ctx)
    const agent = agents.find((a) => a.id === ticket.assignee)
    const assignee = agent?.name ?? ticket.assignee
    const labels = [...ticket.labels].sort(
      (a, b) => Number(filters.labels.includes(b)) - Number(filters.labels.includes(a)),
    )
    const rest = labels.slice(3)
    const rank = priorities.indexOf(ticket.priority)
    const notable = rank > 0 && rank >= priorities.length - 2
    return (
      <Card
        aria-label={`${ticket.key}: ${ticket.title}${changed ? ', changed since your last visit' : ''}`}
        className="kanban-card"
        data-changed={changed ? 'true' : undefined}
        data-dragging={drag?.id === ticket.id ? 'true' : undefined}
        data-ticket-card=""
        data-ticket-id={ticket.id}
        draggable
        interactive
        onClick={() => openTicket(ticket.key)}
        onDragStart={(event) => handleDragStart(ticket, event)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            openTicket(ticket.key)
          }
        }}
        role="button"
        tabIndex={0}
      >
        <div className="kanban-card__head">
          <div className="kanban-card__title">{ticket.title}</div>
          {canOpenPane ? (
            <IconButton
              className="kanban-card__here"
              label="Open in this tab"
              onClick={(event) => {
                event.stopPropagation()
                openInline(ticket.key)
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
              <Chip className="kanban-card__label" key={label} selected={filters.labels.includes(label)}>
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
        <div className="kanban-card__foot">
          <span className="kanban-card__key">{ticket.key}</span>
          {notable ? <PriorityChip priority={ticket.priority} priorities={priorities} /> : null}
          <span className="kanban-card__spacer" />
          {changed ? (
            <span className="kanban-card__changed" title="Changed since your last visit">
              <StatusDot tone="accent" />
              {formatRelative(ticket.updated_at, ctx.now)}
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
  }

  const renderLane = (column: BoardColumn, head: boolean) => {
    const isDropping = dropTarget?.status === column.id && drag !== null
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
        onDragOver={(event) => handleDragOver(column.id, event)}
        onDrop={(event) => handleDrop(column.id, event)}
      >
        {head ? (
          <header className="kanban-lane__head">
            <StatusGlyph columns={columns} status={column.id} />
            <span className="kanban-lane__title">{column.label}</span>
            <span className="kanban-lane__count">{laneCount(column)}</span>
            <IconButton
              className="kanban-lane__add"
              label={`New ticket in ${column.label}`}
              onClick={() => setCreating(column.id)}
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
              <button className="kanban-lane__empty" onClick={() => setCreating(column.id)} type="button">
                No tickets — add one
              </button>
            )
          ) : null}
          {cards.map((ticket) => (
            <div key={ticket.id}>
              {isDropping && others[dropTarget.index]?.id === ticket.id ? (
                <div className="kanban-placeholder" style={{ height: drag?.height ?? 58 }} />
              ) : null}
              {renderCard(ticket)}
            </div>
          ))}
          {isDropping && dropTarget.index >= others.length ? (
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
                onClick={() => setCreating(newStatus)}
                variant="primary"
              >
                <Plus size={16} />
              </IconButton>
            ) : (
              <Button onClick={() => setCreating(newStatus)} variant="primary">
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

          {actionError ? (
            <div className="kanban-pad">
              <StatusPanel
                action={
                  <Button onClick={() => setActionError(null)} size={narrow ? 'lg' : 'md'} variant="pill">
                    Dismiss
                  </Button>
                }
                detail={actionError}
                headline="That change was not saved"
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
                action={{ label: 'New ticket', onClick: () => setCreating(data.default_status) }}
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
                shownCount={shown.length}
                tickets={tickets}
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
              <div className="kanban-lanes kanban-lanes--single">
                {visibleLane ? renderLane(visibleLane, false) : null}
              </div>
            </>
          ) : null}

          {data && columns.length > 0 && layout === 'board' && !narrow ? (
            <div className="kanban-lanes">{columns.map((column) => renderLane(column, true))}</div>
          ) : null}
        </div>
      </PageMain>

      <CreateTicketDialog
        agents={agents}
        columns={columns}
        defaultStatus={creating ?? data?.default_status ?? 'backlog'}
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
