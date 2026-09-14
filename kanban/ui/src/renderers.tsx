import { Card, Chip, Markdown, Table, TableBody, TableCell, TableFrame, TableHead, TableHeader, TableRow, TableViewport, type FunctionTriggerMessage, type Host, type TriggerActivityMessage } from '@iii-dev/console-ui'
import { ChevronRightIcon, CommentIcon, TICKET_PAGE_ID, priorityTone, relativeTime, type Ticket, type TicketSummary } from './shared'

const TICKET_FUNCTIONS = [
  'kanban::ticket::get',
  'kanban::ticket::create',
  'kanban::ticket::update',
  'kanban::ticket::move',
  'kanban::ticket::restore',
  'kanban::ticket::delete',
  'kanban::ticket::list',
  'kanban::board::get',
  'kanban::comment::create',
  'kanban::comment::list',
  'kanban::activity::list',
  'kanban::agent::list',
]

function unwrapDetails(output: unknown): unknown {
  if (!output || typeof output !== 'object') return output
  const record = output as Record<string, unknown>
  if (record.error) return undefined
  if (!('details' in record)) return output
  const details = record.details
  if (typeof details === 'string') {
    try {
      return JSON.parse(details)
    } catch {
      return details
    }
  }
  return details
}

function asTicket(value: unknown): Ticket | null {
  if (!value || typeof value !== 'object') return null
  const record = value as Record<string, unknown>
  if (typeof record.key !== 'string' || typeof record.title !== 'string') return null
  if (typeof record.status !== 'string') return null
  return value as Ticket
}

/** A ticket, or the ticket a `{ comment, ticket }` reply carries. */
function asTicketLike(value: unknown): Ticket | null {
  const direct = asTicket(value)
  if (direct) return direct
  if (!value || typeof value !== 'object') return null
  return asTicket((value as { ticket?: unknown }).ticket)
}

function asSummaryList(value: unknown): TicketSummary[] | null {
  if (!value || typeof value !== 'object') return null
  const record = value as Record<string, unknown>
  if (Array.isArray(record.tickets)) return record.tickets as TicketSummary[]
  if (Array.isArray(record.columns)) {
    return (record.columns as { tickets?: TicketSummary[] }[]).flatMap((column) => column.tickets ?? [])
  }
  return null
}

function titleCase(value: string): string {
  const words = value.replace(/_/g, ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

const TICKET_VERBS: Record<string, string> = {
  'kanban::ticket::create': 'Created',
  'kanban::ticket::update': 'Updated',
  'kanban::ticket::move': 'Moved',
  'kanban::ticket::delete': 'Deleted',
  'kanban::ticket::restore': 'Restored',
  'kanban::comment::create': 'Commented on',
}

function ticketVerb(functionId: string): string | undefined {
  return TICKET_VERBS[functionId]
}

function excerpt(text: string, limit = 140): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > limit ? `${flat.slice(0, limit - 1)}…` : flat
}

function TicketCard({
  host,
  ticket,
  verb,
}: {
  host: Host
  ticket: Ticket
  verb?: string
}) {
  const open = () => host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id: ticket.key } })
  const interactive = Boolean(host.panels?.open)
  const deleted = Boolean(ticket.deleted_at)
  const comments = ticket.comments?.length ?? 0
  return (
    <Card
      aria-label={interactive ? `${ticket.key}: ${ticket.title}` : undefined}
      className="kanban-chat-card"
      data-deleted={deleted ? 'true' : undefined}
      interactive={interactive}
      onClick={interactive ? open : undefined}
      onKeyDown={
        interactive
          ? (event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault()
                open()
              }
            }
          : undefined
      }
      role={interactive ? 'button' : undefined}
      tabIndex={interactive ? 0 : undefined}
    >
      <div className="kanban-chat-card__top">
        {verb ? <span className="kanban-chat-card__verb">{verb}</span> : null}
        <span className="kanban-card__key">{ticket.key}</span>
        <span className="kanban-chat-card__marks">
          <Chip tone={deleted ? 'warning' : 'neutral'}>{deleted ? 'Deleted' : titleCase(ticket.status)}</Chip>
          <Chip tone={priorityTone(ticket.priority)}>{titleCase(ticket.priority)}</Chip>
        </span>
      </div>
      <div className="kanban-chat-card__title" title={ticket.title}>
        {ticket.title}
      </div>
      {ticket.description ? (
        <div className="kanban-chat-card__excerpt">{excerpt(ticket.description)}</div>
      ) : null}
      <div className="kanban-chat-card__foot">
        {ticket.assignee ? (
          <span className="kanban-chat-card__assignee">
            Assigned to <span className="kanban-mono">{ticket.assignee}</span>
          </span>
        ) : (
          <span className="kanban-card__unassigned">Unassigned</span>
        )}
        {comments > 0 ? (
          <span className="kanban-chat-card__comments">
            <CommentIcon />
            {comments}
          </span>
        ) : null}
        <span className="kanban-chat-card__time">updated {relativeTime(ticket.updated_at)}</span>
        {interactive ? (
          <span className="kanban-chat-card__open">
            Open ticket
            <ChevronRightIcon />
          </span>
        ) : null}
      </div>
    </Card>
  )
}

function TicketTable({ tickets }: { tickets: TicketSummary[] }) {
  const rows = tickets.slice(0, 25)
  return (
    <TableViewport className="kanban-chat-table">
      <TableFrame>
        <Table density="compact">
          <TableHeader>
            <TableRow>
              <TableHead>Key</TableHead>
              <TableHead>Title</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Priority</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((ticket) => (
              <TableRow key={ticket.id}>
                <TableCell className="kanban-mono">{ticket.key}</TableCell>
                <TableCell>{ticket.title}</TableCell>
                <TableCell>{titleCase(ticket.status)}</TableCell>
                <TableCell>{titleCase(ticket.priority)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableFrame>
    </TableViewport>
  )
}

export function createTicketRenderer(host: Host) {
  const resolved = (message: FunctionTriggerMessage): Ticket | null => {
    const details = unwrapDetails(message.output)
    if (details === undefined) return null
    return asTicketLike(details)
  }

  return {
    id: 'kanban/page.js#ticket',
    isMatch: (functionId: string) => TICKET_FUNCTIONS.includes(functionId),
    metadata: { display: true },
    tryRenderDisplay: (message: FunctionTriggerMessage) => {
      const ticket = resolved(message)
      if (!ticket) return null
      return <TicketCard host={host} ticket={ticket} verb={ticketVerb(message.functionId)} />
    },
    tryRender: (message: FunctionTriggerMessage) => {
      const details = unwrapDetails(message.output)
      if (details === undefined) return null
      const ticket = asTicketLike(details)
      if (ticket) return <TicketCard host={host} ticket={ticket} verb={ticketVerb(message.functionId)} />
      const list = asSummaryList(details)
      if (list) return <TicketTable tickets={list} />
      return null
    },
    redactRaw: (value: unknown) => value,
  }
}

export function createKanbanTriggerRenderer(host: Host) {
  const isKanbanType = (triggerType: string) =>
    triggerType === 'kanban:change' || triggerType === 'kanban:comment'

  const describe = (activity: TriggerActivityMessage): string => {
    const payload = activity.payload as Record<string, unknown> | undefined
    const event = payload && typeof payload.event === 'string' ? payload.event : null
    const key = payload && typeof payload.ticket_key === 'string' ? payload.ticket_key : null
    if (event && key) return `${event} · ${key}`
    if (event) return event
    return activity.action ?? activity.triggerType
  }

  return {
    id: 'kanban/page.js#trigger-activity',
    isMatch: isKanbanType,
    tryRenderDisplay: (activity: TriggerActivityMessage) => (
      <span className="kanban-chat-line">
        <span className="kanban-chat-line__title">{describe(activity)}</span>
      </span>
    ),
    tryRender: (activity: TriggerActivityMessage) => {
      const payload = activity.payload as Record<string, unknown> | undefined
      const config = (activity.config ?? {}) as Record<string, unknown>
      return (
        <div className="kanban-trigger-detail">
          <div className="kanban-trigger-detail__row">
            <span className="kanban-trigger-detail__label">Fires on</span>
            <span className="kanban-mono">
              {typeof config.ticket_id === 'string' && config.ticket_id ? config.ticket_id : 'any ticket'}
            </span>
          </div>
          {typeof config.author === 'string' && config.author ? (
            <div className="kanban-trigger-detail__row">
              <span className="kanban-trigger-detail__label">Only author</span>
              <span className="kanban-mono">{config.author}</span>
            </div>
          ) : null}
          {typeof config.exclude_author === 'string' && config.exclude_author ? (
            <div className="kanban-trigger-detail__row">
              <span className="kanban-trigger-detail__label">Except author</span>
              <span className="kanban-mono">{config.exclude_author}</span>
            </div>
          ) : null}
          {payload ? (
            <div className="kanban-trigger-detail__row">
              <span className="kanban-trigger-detail__label">Last event</span>
              <span className="kanban-mono">{describe(activity)}</span>
            </div>
          ) : null}
        </div>
      )
    },
    tryRenderDetails: (activity: TriggerActivityMessage) => {
      const payload = activity.payload as { ticket?: unknown; comment?: unknown } | undefined
      const ticket = payload ? asTicket(payload.ticket) : null
      if (!ticket) return null
      const comment = payload?.comment as { body?: string; author?: string } | undefined
      return (
        <div className="kanban-trigger-detail">
          <TicketCard host={host} ticket={ticket} />
          {comment && typeof comment.body === 'string' ? (
            <div className="kanban-prose">
              <Markdown>{comment.body}</Markdown>
            </div>
          ) : null}
        </div>
      )
    },
    redactRaw: (value: unknown) => value,
  }
}
