import {
  Button,
  Card,
  Chip,
  EmptyState,
  IconButton,
  PageHeader,
  PageMain,
  PageShell,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsList,
  TabsTrigger,
  type Host,
  type PageRenderProps,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from 'react'
import { CreateTicketDialog } from './overlays'
import { TicketScreen } from './ticket'
import {
  AlertIcon,
  CommentIcon,
  ColumnsIcon,
  ExpandIcon,
  PlusIcon,
  RefreshIcon,
  TICKET_PAGE_ID,
  api,
  errorMessage,
  priorityTone,
  subscribeChanges,
  summarizeTicket,
  upsertSummary,
  useContainerNarrow,
  usePaneState,
  type AgentProfile,
  type BoardColumn,
  type BoardData,
  type ChangeEvent,
  type TicketSummary,
} from './shared'

type DragState = { id: string; from: string; height: number }

export function BoardPage({
  host,
  onRequestClose,
  paneId,
  tabId,
  commands,
}: PageRenderProps & { host: Host }) {
  const [data, setData] = useState<BoardData | null>(null)
  const [agents, setAgents] = useState<AgentProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [creating, setCreating] = useState<string | null>(null)
  const [inlineTicketId, setInlineTicketId] = useState<string | null>(null)
  const [drag, setDrag] = useState<DragState | null>(null)
  const [dropTarget, setDropTarget] = useState<{ status: string; index: number } | null>(null)
  const [narrowRef, narrow] = useContainerNarrow(760)
  const laneKey = `kanban:lane:${paneId ?? tabId ?? 'default'}`
  const [activeLane, setActiveLane] = usePaneState<string>(laneKey, 'backlog')
  const dragRef = useRef<DragState | null>(null)

  const canOpenPane = Boolean(host.panels && typeof host.panels.open === 'function')

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const board = await api.board(host.iii)
      setData(board)
    } catch (error) {
      setLoadError(errorMessage(error))
    } finally {
      setLoading(false)
    }
  }, [host])

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
    return subscribeChanges(host.iii, (event: ChangeEvent) => {
      if (!event || !event.ticket) return
      setData((current) => {
        if (!current) return current
        const columns = upsertSummary(current.columns, summarizeTicket(event.ticket))
        const visible = columns.reduce((total, column) => total + column.tickets.length, 0)
        const deletedDelta = event.event === 'ticket.deleted' ? 1 : event.event === 'ticket.restored' ? -1 : 0
        return {
          ...current,
          columns,
          ticket_count: visible,
          deleted_count: Math.max(0, current.deleted_count + deletedDelta),
          updated_at: event.at,
        }
      })
    })
  }, [host])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-ticket',
          title: 'New ticket',
          enabled: () => inlineTicketId === null,
          run: () => setCreating(data?.default_status ?? 'backlog'),
        },
        { id: 'refresh', title: 'Refresh board', run: () => void load() },
      ]),
    [commands, data?.default_status, inlineTicketId, load],
  )

  useEffect(() => {
    const clear = () => {
      dragRef.current = null
      setDrag(null)
      setDropTarget(null)
    }
    window.addEventListener('dragend', clear)
    window.addEventListener('drop', clear)
    return () => {
      window.removeEventListener('dragend', clear)
      window.removeEventListener('drop', clear)
    }
  }, [])

  const openTicket = useCallback(
    (id: string) => {
      if (canOpenPane) {
        host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id } })
        return
      }
      setInlineTicketId(id)
    },
    [canOpenPane, host],
  )

  const applyMove = useCallback(
    (id: string, status: string, index: number) => {
      setData((current) => {
        if (!current) return current
        const moving = current.columns.flatMap((column) => column.tickets).find((t) => t.id === id)
        if (!moving) return current
        const columns: BoardColumn[] = current.columns.map((column) => ({
          ...column,
          tickets: column.tickets.filter((t) => t.id !== id),
        }))
        return {
          ...current,
          columns: columns.map((column) => {
            if (column.id !== status) return column
            const tickets = [...column.tickets]
            const position = Math.max(0, Math.min(index, tickets.length))
            tickets.splice(position, 0, { ...moving, status })
            return { ...column, tickets: tickets.map((t, i) => ({ ...t, order: i + 1 })) }
          }),
        }
      })
      void api
        .move(host.iii, { id, status, index, actor: 'user' })
        .then((ticket) => {
          setData((current) =>
            current ? { ...current, columns: upsertSummary(current.columns, summarizeTicket(ticket)) } : current,
          )
        })
        .catch((error) => {
          setActionError(errorMessage(error))
          void load()
        })
    },
    [host, load],
  )

  const handleDragStart = (ticket: TicketSummary, event: DragEvent<HTMLElement>) => {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData('text/plain', ticket.id)
    const height = event.currentTarget.getBoundingClientRect().height
    window.setTimeout(() => {
      const state = { id: ticket.id, from: ticket.status, height }
      dragRef.current = state
      setDrag(state)
    }, 0)
  }

  const handleDragOver = (status: string, event: DragEvent<HTMLElement>) => {
    const dragged = dragRef.current
    if (!dragged) return
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

  const handleDrop = (status: string, event: DragEvent<HTMLElement>) => {
    event.preventDefault()
    const dragged = dragRef.current
    const target = dropTarget
    dragRef.current = null
    setDrag(null)
    setDropTarget(null)
    if (!dragged || !target || target.status !== status) return
    applyMove(dragged.id, status, target.index)
  }

  const columns = data?.columns ?? []
  const visibleLane = columns.find((column) => column.id === activeLane) ?? columns[0] ?? null
  const deletedCount = data?.deleted_count ?? 0

  const headerDescription = useMemo(() => {
    if (!data) return 'Board'
    const base = `${data.ticket_count} ${data.ticket_count === 1 ? 'ticket' : 'tickets'} · ${data.id_prefix}`
    return deletedCount > 0 ? `${base} · ${deletedCount} deleted kept on disk` : base
  }, [data, deletedCount])

  if (inlineTicketId) {
    return (
      <TicketScreen
        host={host}
        ticketId={inlineTicketId}
        columns={columns}
        priorities={data?.priorities ?? []}
        agents={agents}
        onBack={() => setInlineTicketId(null)}
        onRequestClose={onRequestClose}
        commands={commands}
      />
    )
  }

  const renderLane = (column: BoardColumn) => {
    const isDropping = dropTarget?.status === column.id && drag !== null
    const cards = column.tickets
    return (
      <section
        aria-label={`${column.label}, ${cards.length} ${cards.length === 1 ? 'ticket' : 'tickets'}`}
        className="kanban-lane"
        data-drop={isDropping ? 'true' : undefined}
        key={column.id}
        onDragLeave={handleDragLeave}
        onDragOver={(event) => handleDragOver(column.id, event)}
        onDrop={(event) => handleDrop(column.id, event)}
      >
        <header className="kanban-lane__head">
          <span className="kanban-lane__title">{column.label}</span>
          <span className="kanban-lane__count">{cards.length}</span>
          <IconButton
            className="kanban-lane__add"
            label={`New ticket in ${column.label}`}
            onClick={() => setCreating(column.id)}
            variant="ghost"
          >
            <PlusIcon />
          </IconButton>
        </header>
        <div className="kanban-lane__body">
          {cards.length === 0 && !isDropping ? (
            <button className="kanban-lane__empty" onClick={() => setCreating(column.id)} type="button">
              No tickets — add one
            </button>
          ) : null}
          {cards.map((ticket, index) => (
            <div key={ticket.id}>
              {isDropping && dropTarget?.index === index ? (
                <div className="kanban-placeholder" style={{ height: drag?.height ?? 58 }} />
              ) : null}
              <Card
                aria-label={`${ticket.key}: ${ticket.title}`}
                className="kanban-card"
                data-dragging={drag?.id === ticket.id ? 'true' : undefined}
                data-ticket-card=""
                data-ticket-id={ticket.id}
                draggable
                interactive
                onClick={() => openTicket(ticket.key)}
                onDragEnd={() => {
                  dragRef.current = null
                  setDrag(null)
                  setDropTarget(null)
                }}
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
                <div className="kanban-card__top">
                  <span className="kanban-card__key">{ticket.key}</span>
                  {ticket.priority !== 'low' && ticket.priority !== 'medium' ? (
                    <Chip tone={priorityTone(ticket.priority)}>{ticket.priority}</Chip>
                  ) : null}
                  {canOpenPane ? (
                    <IconButton
                      className="kanban-card__here"
                      label="Open in this tab"
                      onClick={(event) => {
                        event.stopPropagation()
                        setInlineTicketId(ticket.key)
                      }}
                      onKeyDown={(event) => event.stopPropagation()}
                      variant="ghost"
                    >
                      <ExpandIcon />
                    </IconButton>
                  ) : null}
                </div>
                <div className="kanban-card__title">{ticket.title}</div>
                <div className="kanban-card__foot">
                  {ticket.assignee ? (
                    <span className="kanban-assignee" data-color={agents.find((a) => a.id === ticket.assignee)?.color ?? undefined}>
                      {agents.find((a) => a.id === ticket.assignee)?.logo ?? ''}
                      <span>{agents.find((a) => a.id === ticket.assignee)?.name ?? ticket.assignee}</span>
                    </span>
                  ) : (
                    <span className="kanban-card__unassigned">Unassigned</span>
                  )}
                  {ticket.comment_count > 0 ? (
                    <span className="kanban-card__meta">
                      <CommentIcon />
                      {ticket.comment_count}
                    </span>
                  ) : null}
                </div>
              </Card>
            </div>
          ))}
          {isDropping && dropTarget?.index >= cards.length ? (
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
            <IconButton label="Refresh board" onClick={() => void load()} variant="ghost">
              <RefreshIcon />
            </IconButton>
            <Button onClick={() => setCreating(data?.default_status ?? 'backlog')} variant="primary">
              <PlusIcon />
              New ticket
            </Button>
          </>
        }
        description={headerDescription}
        icon={<ColumnsIcon />}
        onClose={onRequestClose}
        title="Kanban"
      />
      <PageMain>
        <div className="kanban-board" data-autofocus="" ref={narrowRef} tabIndex={-1}>
          {loadError ? (
            <div className="kanban-pad">
              <StatusPanel
                detail={loadError}
                headline="The board could not be read"
                icon={<AlertIcon />}
                variant="alert"
              />
              <Button onClick={() => void load()} variant="pill">
                Retry
              </Button>
            </div>
          ) : null}

          {actionError ? (
            <div className="kanban-pad">
              <StatusPanel
                detail={actionError}
                headline="That change was not saved"
                icon={<AlertIcon />}
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
                icon={ColumnsIcon}
                title="No columns configured"
              />
            </div>
          ) : null}

          {data && columns.length > 0 && narrow ? (
            <>
              <Tabs onValueChange={setActiveLane} value={visibleLane?.id ?? columns[0].id}>
                <TabsList className="kanban-lane-tabs">
                  {columns.map((column) => (
                    <TabsTrigger icon={false} key={column.id} value={column.id}>
                      {column.label}
                      <span className="kanban-lane__count">{column.tickets.length}</span>
                    </TabsTrigger>
                  ))}
                </TabsList>
              </Tabs>
              <div className="kanban-lanes kanban-lanes--single">
                {visibleLane ? renderLane(visibleLane) : null}
              </div>
            </>
          ) : null}

          {data && columns.length > 0 && !narrow ? (
            <div className="kanban-lanes">{columns.map((column) => renderLane(column))}</div>
          ) : null}
        </div>
      </PageMain>

      <CreateTicketDialog
        agents={agents}
        columns={columns}
        defaultStatus={creating ?? data?.default_status ?? 'backlog'}
        host={host}
        onCreated={(ticket) => {
          setCreating(null)
          setData((current) =>
            current ? { ...current, columns: upsertSummary(current.columns, summarizeTicket(ticket)) } : current,
          )
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

