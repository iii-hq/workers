import {
  type FunctionCallMessage,
  type Host,
  Markdown,
  type MentionRendererProps,
  type MentionRendererRegistration,
  ShadcnBadge,
  ShadcnCard,
  ShadcnProgress,
  ShadcnTable,
  ShadcnTableBody,
  ShadcnTableCell,
  ShadcnTableHead,
  ShadcnTableHeader,
  ShadcnTableRow,
  type ObserverActivityMessage,
} from '@iii-dev/console-ui'
import { formatRelative, unwrapEnvelope } from '@iii-dev/console-ui/format'
import { ChevronRight, CircleX, FolderOpen, MessageSquare, Milestone } from 'lucide-react'
import { type KeyboardEvent, type ReactNode, useEffect, useState } from 'react'
import { formatTarget, observerScope, plural, releaseStatusText } from './plan-rules'
import {
  api,
  CHANGE_FN,
  COMMENT_FN,
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
import { AssigneeAvatar } from './ui/avatar'
import { StatusIcon } from './ui/glyphs'

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

/** A facet with nothing in it: said in the ghost ink, not left out. */
const none = () => <span className="kb-chat-card__none">none</span>

type Fact = { label: string; value: ReactNode }

/** Facts about a card as quiet `label value` pairs on one wrapping line. */
function Facts({ items, label }: { items: Fact[]; label: string }) {
  if (items.length === 0) return null
  return (
    <div aria-label={label} className="kb-chat-card__facts" role="group">
      {items.map((item) => (
        <span className="kb-chat-card__fact" key={item.label}>
          <span className="kb-chat-card__fact-label">{item.label}</span>
          <span className="kb-chat-card__fact-value">{item.value}</span>
        </span>
      ))}
    </div>
  )
}

/** A state badge with a tone dot: the column a ticket is in, or a release's or project's status. */
function StateBadge({ tone, children }: { tone: 'ink' | 'ghost' | 'ok'; children: ReactNode }) {
  return (
    <ShadcnBadge className="kb-chat-card__status" variant="outline">
      <span aria-hidden="true" className="kb-swim__dot" data-tone={tone} />
      {children}
    </ShadcnBadge>
  )
}

/** A card that opens a pane when the console can open one: clickable, focusable, Enter and Space. */
function cardProps(interactive: boolean, open: () => void, label: string) {
  if (!interactive) return {}
  return {
    'aria-label': label,
    onClick: open,
    onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault()
        open()
      }
    },
    role: 'button',
    tabIndex: 0,
  }
}

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
  const facets: Fact[] = []
  if (ticket.parent_id) facets.push({ label: 'Parent', value: ticket.parent?.key ?? ticket.parent_id })
  if (ticket.release_id === null) facets.push({ label: 'Release', value: none() })
  else if (ticket.release_id && answer)
    facets.push({ label: 'Release', value: lookup?.releases.get(ticket.release_id) ?? ticket.release_id })
  if (ticket.project_id === null) facets.push({ label: 'Project', value: none() })
  else if (ticket.project_id && answer)
    facets.push({ label: 'Project', value: lookup?.projects.get(ticket.project_id) ?? ticket.project_id })
  const step = lookup && columns.length > 0 ? statusGlyph(columns, ticket.status, lookup.doneStatus, lookup.canceledStatus).step : 'unknown'
  return (
    <ShadcnCard
      className="kb-chat kb-chat-card"
      data-deleted={deleted ? 'true' : undefined}
      size="sm"
      {...cardProps(interactive, open, `${ticket.key}: ${ticket.title}`)}
    >
      <div className="kb-chat-card__top">
        {verb ? <span className="kb-chat-card__verb">{verb}</span> : null}
        <span className="kb-chat-card__key">{ticket.key}</span>
        <span className="kb-chat-card__marks">
          {deleted ? (
            <ShadcnBadge className="kb-chat-card__status" data-deleted="true" variant="outline">
              <CircleX size={16} />
              Deleted
            </ShadcnBadge>
          ) : (
            <ShadcnBadge className="kb-chat-card__status" variant="outline">
              <StatusIcon step={step} />
              {columnLabel(columns, ticket.status)}
            </ShadcnBadge>
          )}
          <PriorityChip priorities={lookup?.priorities ?? []} priority={ticket.priority} />
        </span>
      </div>
      <p className="kb-chat-card__title" title={ticket.title}>
        {ticket.title}
      </p>
      {ticket.description ? <p className="kb-chat-card__excerpt">{excerpt(ticket.description)}</p> : null}
      <Facts items={facets} label={`${ticket.key} facets`} />
      <div className="kb-chat-card__foot">
        <span className="kb-chat-card__person">
          <AssigneeAvatar agents={[]} id={ticket.assignee} />
          {ticket.assignee ?? 'Unassigned'}
        </span>
        {comments > 0 ? (
          <span aria-label={`${comments} comments`} className="kb-chat-card__stat" role="img">
            <MessageSquare size={16} />
            {comments}
          </span>
        ) : null}
        <span>updated {formatRelative(ticket.updated_at)}</span>
        {interactive ? (
          <span className="kb-chat-card__open">
            Open ticket
            <ChevronRight size={16} />
          </span>
        ) : null}
      </div>
    </ShadcnCard>
  )
}

function TicketTable({ host, tickets }: { host: Host; tickets: TicketSummary[] }) {
  const columns = useCardLookup(host)?.lookup?.columns ?? []
  const rows = tickets.slice(0, 25)
  return (
    <div className="kb-chat kb-chat-table">
      <ShadcnTable>
        <ShadcnTableHeader>
          <ShadcnTableRow>
            <ShadcnTableHead>Key</ShadcnTableHead>
            <ShadcnTableHead>Title</ShadcnTableHead>
            <ShadcnTableHead>Status</ShadcnTableHead>
            <ShadcnTableHead>Priority</ShadcnTableHead>
          </ShadcnTableRow>
        </ShadcnTableHeader>
        <ShadcnTableBody>
          {rows.map((ticket) => (
            <ShadcnTableRow key={ticket.id}>
              <ShadcnTableCell className="kb-chat-mono">{ticket.key}</ShadcnTableCell>
              <ShadcnTableCell>{ticket.title}</ShadcnTableCell>
              <ShadcnTableCell>{columnLabel(columns, ticket.status)}</ShadcnTableCell>
              <ShadcnTableCell>{priorityLabel(ticket.priority)}</ShadcnTableCell>
            </ShadcnTableRow>
          ))}
        </ShadcnTableBody>
      </ShadcnTable>
    </div>
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
  const facets: Fact[] = [
    release
      ? { label: 'Target', value: release.target_at ? formatTarget(release.target_at) : none() }
      : { label: 'Lead', value: project?.lead ?? none() },
    { label: 'Done', value: `${entity.done} of ${entity.total}` },
    { label: 'Label', value: entity.source_label ?? none() },
  ]
  const percent = entity.total > 0 ? Math.round((entity.done / entity.total) * 100) : 0
  return (
    <ShadcnCard className="kb-chat kb-chat-card" size="sm" {...cardProps(interactive, open, `Open ${kind} ${entity.name}`)}>
      <div className="kb-chat-card__head">
        {verb ? <span className="kb-chat-card__verb">{verb}</span> : null}
        {project ? (
          <span className="kb-project-dot" data-tone={project.color ?? undefined}>
            <FolderOpen size={16} />
          </span>
        ) : (
          <Milestone size={16} />
        )}
        <span className="kb-chat-card__name" title={entity.name}>
          {entity.name}
        </span>
        <StateBadge
          tone={archived ? 'ghost' : entity.status === 'released' ? 'ok' : entity.status === 'planned' ? 'ghost' : 'ink'}
        >
          {release ? releaseStatusText(release) : titleCase(entity.status)}
        </StateBadge>
      </div>
      <ShadcnProgress aria-label={`${entity.done} of ${entity.total} done`} className="kb-chat-card__progress" value={percent} />
      <Facts items={facets} label={`${entity.name} facets`} />
      <div className="kb-chat-card__foot">
        <span>
          {release ? `${entity.total - entity.done} not done` : `${plural(entity.total - entity.done, 'open ticket')}`}
        </span>
        {note ? <span>{note}</span> : null}
        {interactive ? (
          <span className="kb-chat-card__open">
            Open {kind}
            <ChevronRight size={16} />
          </span>
        ) : null}
      </div>
    </ShadcnCard>
  )
}

function EntityTable({ kind, entities }: { kind: 'release' | 'project'; entities: Entity[] }) {
  const rows = entities.slice(0, 25)
  return (
    <div className="kb-chat kb-chat-table">
      <ShadcnTable>
        <ShadcnTableHeader>
          <ShadcnTableRow>
            <ShadcnTableHead>Name</ShadcnTableHead>
            <ShadcnTableHead>Status</ShadcnTableHead>
            <ShadcnTableHead>{kind === 'release' ? 'Target' : 'Lead'}</ShadcnTableHead>
            <ShadcnTableHead>Done</ShadcnTableHead>
          </ShadcnTableRow>
        </ShadcnTableHeader>
        <ShadcnTableBody>
          {rows.map((entity) => {
            const day = kind === 'release' ? (entity as ReleaseView).target_at : null
            return (
              <ShadcnTableRow key={entity.id}>
                <ShadcnTableCell>{entity.name}</ShadcnTableCell>
                <ShadcnTableCell>
                  {kind === 'release' ? releaseStatusText(entity as ReleaseView) : titleCase(entity.status)}
                </ShadcnTableCell>
                <ShadcnTableCell className="kb-chat-mono">
                  {kind === 'release' ? (day ? formatTarget(day) : '—') : ((entity as ProjectView).lead ?? '—')}
                </ShadcnTableCell>
                <ShadcnTableCell className="kb-chat-mono">
                  {entity.done}/{entity.total}
                </ShadcnTableCell>
              </ShadcnTableRow>
            )
          })}
        </ShadcnTableBody>
      </ShadcnTable>
    </div>
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
  const card = (message: FunctionCallMessage) => {
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
    tryRender: (message: FunctionCallMessage) => {
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
  const resolved = (message: FunctionCallMessage): Ticket | null => {
    const details = unwrapDetails(message.output)
    if (details === undefined) return null
    return asTicketLike(details)
  }

  return {
    id: 'kanban/page.js#ticket',
    isMatch: (functionId: string) => TICKET_FUNCTIONS.includes(functionId),
    metadata: { display: true },
    tryRenderDisplay: (message: FunctionCallMessage) => {
      const ticket = resolved(message)
      if (!ticket) return null
      return <TicketCard host={host} ticket={ticket} verb={ticketVerb(message.functionId)} />
    },
    tryRender: (message: FunctionCallMessage) => {
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

export function createKanbanObserverRenderer(host: Host) {
  const isKanbanSource = (functionId: string) => functionId === CHANGE_FN || functionId === COMMENT_FN

  const describe = (activity: ObserverActivityMessage): string => {
    const payload = activity.payload as Record<string, unknown> | undefined
    const event = payload && typeof payload.event === 'string' ? payload.event : null
    const key = payload && typeof payload.ticket_key === 'string' ? payload.ticket_key : null
    if (event && key) return `${event} · ${key}`
    if (event) return event
    return activity.action ?? activity.functionId
  }

  return {
    id: 'kanban/page.js#observer-activity',
    isMatch: isKanbanSource,
    tryRenderDisplay: (activity: ObserverActivityMessage) => (
      <span className="kb-chat kb-chat-line">
        <span className="kb-chat-line__title">{describe(activity)}</span>
      </span>
    ),
    tryRender: (activity: ObserverActivityMessage) => {
      const payload = activity.payload as Record<string, unknown> | undefined
      const args = activity.args ?? {}
      return (
        <div className="kb-chat kb-observer-detail">
          <div className="kb-observer-detail__row">
            <span className="kb-observer-detail__label">Watches</span>
            <span className="kb-chat-mono">{observerScope(activity.functionId, args)}</span>
          </div>
          {typeof args.author === 'string' && args.author ? (
            <div className="kb-observer-detail__row">
              <span className="kb-observer-detail__label">Only author</span>
              <span className="kb-chat-mono">{args.author}</span>
            </div>
          ) : null}
          {typeof args.exclude_author === 'string' && args.exclude_author ? (
            <div className="kb-observer-detail__row">
              <span className="kb-observer-detail__label">Except author</span>
              <span className="kb-chat-mono">{args.exclude_author}</span>
            </div>
          ) : null}
          {payload ? (
            <div className="kb-observer-detail__row">
              <span className="kb-observer-detail__label">Last event</span>
              <span className="kb-chat-mono">{describe(activity)}</span>
            </div>
          ) : null}
        </div>
      )
    },
    tryRenderDetails: (activity: ObserverActivityMessage) => {
      const payload = activity.payload as { ticket?: unknown; comment?: unknown } | undefined
      const ticket = payload ? asTicket(payload.ticket) : null
      if (!ticket) return null
      const comment = payload?.comment as { body?: string; author?: string } | undefined
      return (
        <div className="kb-chat kb-observer-detail">
          <TicketCard host={host} ticket={ticket} />
          {comment && typeof comment.body === 'string' ? (
            <div className="kb-chat-prose">
              <Markdown>{comment.body}</Markdown>
            </div>
          ) : null}
        </div>
      )
    },
    redactRaw: (value: unknown) => value,
  }
}
