import {
  Badge,
  Card,
  type FunctionTriggerMessage,
  type Host,
  Markdown,
  type MentionRendererProps,
  type MentionRendererRegistration,
  MetaRow,
  type MetaRowItem,
  StatusDot,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  type TriggerActivityMessage,
} from '@iii-dev/console-ui'
import { formatRelative, unwrapEnvelope } from '@iii-dev/console-ui/format'
import { ChevronRight, Flag, MessageCircle } from 'lucide-react'
import { useEffect, useState } from 'react'
import { ProjectGlyph, StatusBadge } from './plan-dialogs'
import { formatTarget, plural, releaseStatusText, triggerScope } from './plan-rules'
import {
  api,
  type Column,
  PROJECTS_PAGE_ID,
  PriorityChip,
  type ProjectView,
  priorityLabel,
  RELEASES_PAGE_ID,
  type ReleaseView,
  statusGlyph,
  TICKET_PAGE_ID,
  type Ticket,
  type TicketSummary,
} from './shared'

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
  if (output && typeof output === 'object' && (output as Record<string, unknown>).error) return undefined
  const details = unwrapEnvelope(output)
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
  ['kanban::ticket::create']: 'Created',
  ['kanban::ticket::update']: 'Updated',
  ['kanban::ticket::move']: 'Moved',
  ['kanban::ticket::delete']: 'Deleted',
  ['kanban::ticket::restore']: 'Restored',
  ['kanban::comment::create']: 'Commented on',
}

function ticketVerb(functionId: string): string | undefined {
  return TICKET_VERBS[functionId]
}

function excerpt(text: string, limit = 140): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > limit ? `${flat.slice(0, limit - 1)}…` : flat
}

/**
 * What a card needs from the board and cannot carry itself: the names of every project and release, the column
 * labels and the priority order. Read once and kept briefly: a chat can hold dozens of cards.
 */
type Lookup = {
  projects: Map<string, string>
  releases: Map<string, string>
  columns: Column[]
  doneStatus: string
  canceledStatus: string | null
  priorities: string[]
}
let lookupCache: { at: number; lookup: Promise<Lookup> } | null = null

const FRESH_MS = 30_000
/** A load this young answers a card that found its project missing: N such cards share one refresh. */
const SHARED_REFRESH_MS = 2_000

function boardLookup(host: Host, refresh = false): Promise<Lookup> {
  if (lookupCache && Date.now() - lookupCache.at < (refresh ? SHARED_REFRESH_MS : FRESH_MS)) return lookupCache.lookup
  const lookup = Promise.all([
    api.projects(host.iii),
    api.releases(host.iii),
    // Column labels and priorities are a nicety: without them a card falls back to the raw words.
    api.config(host.iii).catch(() => null),
  ]).then(([projects, releases, config]) => ({
    projects: new Map(projects.projects.map((project) => [project.id, project.name])),
    releases: new Map(releases.releases.map((release) => [release.id, release.name])),
    columns: config?.columns ?? [],
    doneStatus: config?.done_status ?? '',
    canceledStatus: config?.canceled_status ?? null,
    priorities: config?.priorities ?? [],
  }))
  lookupCache = { at: Date.now(), lookup }
  lookup.catch(() => {
    if (lookupCache?.lookup === lookup) lookupCache = null
  })
  return lookup
}

/**
 * The lookup for a card. `answer` is null until it settles (the project and release lines are absent then, not
 * ids); a lookup that fails settles with `lookup: null` and the lines fall back to the ids.
 */
function useCardLookup(host: Host, projectId?: string | null, releaseId?: string | null) {
  const [answer, setAnswer] = useState<{ lookup: Lookup | null } | null>(null)
  useEffect(() => {
    let cancelled = false
    void (async () => {
      let lookup: Lookup | null = null
      try {
        lookup = await boardLookup(host)
        // A project made a moment ago is not in the kept answer yet: ask again.
        if ((projectId && !lookup.projects.has(projectId)) || (releaseId && !lookup.releases.has(releaseId))) {
          lookup = await boardLookup(host, true)
        }
      } catch {
        lookup = null
      }
      if (!cancelled) setAnswer({ lookup })
    })()
    return () => {
      cancelled = true
    }
  }, [host, projectId, releaseId])
  return answer
}

/** The column's own label (`In review`); the raw id, title-cased, until the columns are known. */
const columnLabel = (columns: Column[], status: string) =>
  columns.find((column) => column.id === status)?.label ?? titleCase(status)

/** The dot of a status badge, by the column's place on the board (the same reading as its glyph). */
function statusTone(lookup: Lookup | undefined | null, status: string): 'ink' | 'ghost' | 'ok' {
  if (!lookup || lookup.columns.length === 0) return 'ghost'
  const { tone } = statusGlyph(lookup.columns, status, lookup.doneStatus, lookup.canceledStatus)
  return tone === 'ok' ? 'ok' : tone === 'ghost' || tone === 'faint' ? 'ghost' : 'ink'
}

/** A facet with nothing in it: said in the ghost ink, not left out. */
const none = () => <span className="kanban-chat-none">none</span>

function TicketCard({
  host,
  ticket,
  verb,
  commentCount,
}: {
  host: Host
  ticket: Ticket
  verb?: string
  /** For a projection that counts comments instead of carrying them. */
  commentCount?: number
}) {
  const open = () => host.panels?.open({ pageId: TICKET_PAGE_ID, context: { id: ticket.key } })
  const interactive = Boolean(host.panels?.open)
  const deleted = Boolean(ticket.deleted_at)
  const comments = commentCount ?? ticket.comments?.length ?? 0
  const answer = useCardLookup(host, ticket.project_id, ticket.release_id)
  const lookup = answer?.lookup
  const columns = lookup?.columns ?? []
  // "none" is known at once; a name waits for the lookup. Nothing is said of a field the projection did not carry.
  const facets: MetaRowItem[] = []
  if (ticket.parent_id) facets.push({ label: 'Parent', value: ticket.parent?.key ?? ticket.parent_id })
  if (ticket.release_id === null) facets.push({ label: 'Release', value: none() })
  else if (ticket.release_id && answer)
    facets.push({ label: 'Release', value: lookup?.releases.get(ticket.release_id) ?? ticket.release_id })
  if (ticket.project_id === null) facets.push({ label: 'Project', value: none() })
  else if (ticket.project_id && answer)
    facets.push({ label: 'Project', value: lookup?.projects.get(ticket.project_id) ?? ticket.project_id })
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
      {facets.length > 0 ? (
        <MetaRow aria-label={`${ticket.key} facets`} className="kanban-chat-meta" items={facets} role="group" />
      ) : null}
      <div className="kanban-chat-card__top">
        {verb ? <span className="kanban-chat-card__verb">{verb}</span> : null}
        <span className="kanban-card__key">{ticket.key}</span>
        <span className="kanban-chat-card__marks">
          {deleted ? (
            <Badge variant="alert">
              <StatusDot tone="alert" />
              Deleted
            </Badge>
          ) : (
            <StatusBadge tone={statusTone(lookup, ticket.status)}>{columnLabel(columns, ticket.status)}</StatusBadge>
          )}
          <PriorityChip priorities={lookup?.priorities ?? []} priority={ticket.priority} />
        </span>
      </div>
      <div className="kanban-chat-card__title" title={ticket.title}>
        {ticket.title}
      </div>
      {ticket.description ? <div className="kanban-chat-card__excerpt">{excerpt(ticket.description)}</div> : null}
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
            <MessageCircle size={16} />
            {comments}
          </span>
        ) : null}
        <span className="kanban-chat-card__time">updated {formatRelative(ticket.updated_at)}</span>
        {interactive ? (
          <span className="kanban-chat-card__open">
            Open ticket
            <ChevronRight size={16} />
          </span>
        ) : null}
      </div>
    </Card>
  )
}

function TicketTable({ host, tickets }: { host: Host; tickets: TicketSummary[] }) {
  const columns = useCardLookup(host)?.lookup?.columns ?? []
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
                <TableCell>{columnLabel(columns, ticket.status)}</TableCell>
                <TableCell>{priorityLabel(ticket.priority)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableFrame>
    </TableViewport>
  )
}

/* ── releases and projects ────────────────────────────────────────────── */

type Entity = ReleaseView | ProjectView

const RELEASE_VERBS: Record<string, string> = {
  create: 'Created',
  update: 'Updated',
  ship: 'Marked as released',
  reopen: 'Reopened',
}
const PROJECT_VERBS: Record<string, string> = {
  create: 'Created',
  update: 'Updated',
  archive: 'Archived',
  restore: 'Restored',
}

function asEntity(value: unknown): Entity | null {
  if (!value || typeof value !== 'object') return null
  const record = value as Record<string, unknown>
  const named = typeof record.id === 'string' && typeof record.name === 'string' && typeof record.status === 'string'
  return named && typeof record.total === 'number' && typeof record.done === 'number' ? (value as Entity) : null
}

/** A release or project, or the one a `{ release, moved }` ship reply carries. */
function asEntityLike(value: unknown): { entity: Entity; moved: number | null } | null {
  const direct = asEntity(value)
  if (direct) return { entity: direct, moved: null }
  if (!value || typeof value !== 'object') return null
  const record = value as { release?: unknown; moved?: unknown }
  const entity = asEntity(record.release)
  return entity ? { entity, moved: Array.isArray(record.moved) ? record.moved.length : null } : null
}

function entityList(value: unknown, kind: 'release' | 'project'): Entity[] | null {
  if (!value || typeof value !== 'object') return null
  const list = (value as Record<string, unknown>)[kind === 'release' ? 'releases' : 'projects']
  return Array.isArray(list) ? (list as Entity[]) : null
}

function PlanCard({
  host,
  kind,
  entity,
  verb,
  note,
}: {
  host: Host
  kind: 'release' | 'project'
  entity: Entity
  verb?: string
  note?: string
}) {
  const release = kind === 'release' ? (entity as ReleaseView) : null
  const project = kind === 'project' ? (entity as ProjectView) : null
  const open = () =>
    host.panels?.open({ pageId: release ? RELEASES_PAGE_ID : PROJECTS_PAGE_ID, context: { id: entity.id } })
  const interactive = Boolean(host.panels?.open)
  const archived = entity.status === 'archived'
  const facets: MetaRowItem[] = [
    release
      ? { label: 'Target', value: release.target_at ? formatTarget(release.target_at) : none() }
      : { label: 'Lead', value: project?.lead ?? none() },
    { label: 'Done', value: `${entity.done} of ${entity.total}` },
    { label: 'Label', value: entity.source_label ?? none() },
  ]
  return (
    <Card
      aria-label={interactive ? `Open ${kind} ${entity.name}` : undefined}
      className="kanban-chat-card"
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
      <MetaRow aria-label={`${entity.name} facets`} className="kanban-chat-meta" items={facets} role="group" />
      <div className="kanban-chat-card__head">
        {verb ? <span className="kanban-chat-card__verb">{verb}</span> : null}
        {project ? <ProjectGlyph color={project.color} /> : <Flag size={16} />}
        <span className="kanban-chat-card__name" title={entity.name}>
          {entity.name}
        </span>
        <StatusBadge
          tone={
            archived ? 'ghost' : entity.status === 'released' ? 'ok' : entity.status === 'planned' ? 'ghost' : 'ink'
          }
        >
          {release ? releaseStatusText(release) : titleCase(entity.status)}
        </StatusBadge>
      </div>
      <div className="kanban-chat-card__foot">
        <span>
          {release ? `${entity.total - entity.done} not done` : `${plural(entity.total - entity.done, 'open ticket')}`}
        </span>
        {note ? <span>{note}</span> : null}
        {interactive ? (
          <span className="kanban-chat-card__open">
            Open {kind}
            <ChevronRight size={16} />
          </span>
        ) : null}
      </div>
    </Card>
  )
}

function EntityTable({ kind, entities }: { kind: 'release' | 'project'; entities: Entity[] }) {
  const rows = entities.slice(0, 25)
  return (
    <TableViewport className="kanban-chat-table">
      <TableFrame>
        <Table density="compact">
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>{kind === 'release' ? 'Target' : 'Lead'}</TableHead>
              <TableHead>Done</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((entity) => {
              const day = kind === 'release' ? (entity as ReleaseView).target_at : null
              return (
                <TableRow key={entity.id}>
                  <TableCell>{entity.name}</TableCell>
                  <TableCell>
                    {kind === 'release' ? releaseStatusText(entity as ReleaseView) : titleCase(entity.status)}
                  </TableCell>
                  <TableCell className="kanban-mono">
                    {kind === 'release' ? (day ? formatTarget(day) : '—') : ((entity as ProjectView).lead ?? '—')}
                  </TableCell>
                  <TableCell className="kanban-mono">
                    {entity.done}/{entity.total}
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      </TableFrame>
    </TableViewport>
  )
}

/** The cards of 'kanban::release::*' and 'kanban::project::*' results. */
export function createPlanRenderer(host: Host) {
  const kindOf = (functionId: string): 'release' | 'project' | null =>
    functionId.startsWith('kanban::release::')
      ? 'release'
      : functionId.startsWith('kanban::project::')
        ? 'project'
        : null
  const verbOf = (functionId: string, kind: 'release' | 'project') =>
    (kind === 'release' ? RELEASE_VERBS : PROJECT_VERBS)[functionId.split('::')[2] ?? '']
  const card = (message: FunctionTriggerMessage) => {
    const kind = kindOf(message.functionId)
    const details = unwrapDetails(message.output)
    if (!kind || details === undefined) return null
    const found = asEntityLike(details)
    if (!found) return null
    const note = found.moved ? `${plural(found.moved, 'ticket')} moved` : undefined
    return (
      <PlanCard entity={found.entity} host={host} kind={kind} note={note} verb={verbOf(message.functionId, kind)} />
    )
  }

  return {
    id: 'kanban/page.js#plan',
    isMatch: (functionId: string) => kindOf(functionId) !== null,
    metadata: { display: true },
    tryRenderDisplay: card,
    tryRender: (message: FunctionTriggerMessage) => {
      const single = card(message)
      if (single) return single
      const kind = kindOf(message.functionId)
      const details = unwrapDetails(message.output)
      const list = kind && details !== undefined ? entityList(details, kind) : null
      return kind && list ? <EntityTable entities={list} kind={kind} /> : null
    },
    redactRaw: (value: unknown) => value,
  }
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
      if (list) return <TicketTable host={host} tickets={list} />
      return null
    },
    redactRaw: (value: unknown) => value,
  }
}

/**
 * The preview of a `@kanban(id="…")` chat mention: the same card a ticket
 * function result shows, drawn from the mention view's `data` (the board
 * projection plus a description excerpt, see 'kanban/src/mentions.rs').
 */
export function createTicketMentionRenderer(host: Host): MentionRendererRegistration {
  return {
    provider: 'kanban',
    Preview: ({ view }: MentionRendererProps) => {
      const data = view.data as (Partial<Ticket> & { comment_count?: number }) | undefined
      const ticket = asTicket(data ? { ...data, comments: [], activity: [] } : null)
      // Not a ticket projection (an older backend): the console's generic
      // card takes over when a preview throws.
      if (!ticket) throw new Error('kanban mention without ticket data')
      return <TicketCard host={host} ticket={ticket} commentCount={data?.comment_count} />
    },
  }
}

export function createKanbanTriggerRenderer(host: Host) {
  const isKanbanType = (triggerType: string) => triggerType === 'kanban:change' || triggerType === 'kanban:comment'

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
            <span className="kanban-mono">{triggerScope(activity.triggerType, config)}</span>
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
