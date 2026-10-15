import { Chip, type ExtensionIii } from '@iii-dev/console-ui'
import { ChartPie, Circle, CircleCheck, CircleDashed, Contrast } from 'lucide-react'
import { useSyncExternalStore } from 'react'

export type Column = { id: string; label: string }

export type TicketSummary = {
  id: string
  key: string
  title: string
  status: string
  priority: string
  assignee: string | null
  labels: string[]
  order: number
  created_at: string
  updated_at: string
  deleted_at: string | null
  comment_count: number
  parent_id: string | null
  project_id: string | null
  release_id: string | null
}

export type BoardColumn = { id: string; label: string; tickets: TicketSummary[] }

export type BoardData = {
  columns: BoardColumn[]
  ticket_count: number
  deleted_count: number
  priorities: string[]
  default_status: string
  id_prefix: string
  data_path_resolved: string
  board_file: string
  updated_at: string
  /** The column that counts as done for progress, releases and subtasks. */
  done_status: string
  projects: ProjectView[]
  releases: ReleaseView[]
}

export type GlyphTone = 'blue' | 'purple' | 'teal' | 'green' | 'amber' | 'rose'

export type Project = {
  id: string
  name: string
  description: string
  lead: string | null
  color: GlyphTone | null
  source_label: string | null
  created_at: string
  updated_at: string
  archived_at: string | null
}

export type Release = {
  id: string
  name: string
  description: string
  /** `YYYY-MM-DD`. */
  target_at: string | null
  released_at: string | null
  source_label: string | null
  created_at: string
  updated_at: string
}

/** Live tickets per column id. */
export type ColumnCounts = Record<string, number>

export type ProjectView = Project & { status: 'active' | 'archived'; total: number; done: number; counts: ColumnCounts }

export type ReleaseView = Release & {
  status: 'planned' | 'active' | 'released'
  total: number
  done: number
  counts: ColumnCounts
  /** Not-done tickets in a released release. */
  reopened: number
  projects: { project_id: string | null; total: number; done: number }[]
}

export type Brief = {
  id: string
  key: string
  title: string
  status: string
  priority: string
  assignee: string | null
  deleted_at: string | null
  project_id: string | null
  release_id: string | null
}

export type ConvertResult = {
  entity: ProjectView | ReleaseView | null
  created: boolean
  /** Ticket keys per outcome. */
  buckets: { added: string[]; already_here: string[]; in_other: string[]; deleted: string[] }
  tickets: TicketSummary[]
}

export type Change = { field: string; from: unknown; to: unknown }

export type Comment = {
  id: string
  ticket_id: string
  parent_id: string | null
  body: string
  author: string
  created_at: string
}

export type Activity = {
  id: string
  ticket_id: string
  seq: number
  type: string
  actor: string
  at: string
  changes?: Change[]
  comment_id?: string | null
}

export type Ticket = {
  id: string
  key: string
  title: string
  description: string
  status: string
  priority: string
  assignee: string | null
  labels: string[]
  order: number
  created_at: string
  updated_at: string
  deleted_at: string | null
  created_by: string
  comments: Comment[]
  activity: Activity[]
  parent_id: string | null
  project_id: string | null
  release_id: string | null
  /** Every ticket the worker returns (and every change event) carries these. */
  parent: Brief | null
  children: Brief[]
  progress: { done: number; total: number }
}

export type AgentProfile = {
  id: string
  name: string
  description: string | null
  logo: string | null
  icon: string | null
  color: string | null
  model: string | null
}

/** `kanban::ticket::links`: tickets this one mentions by key, and tickets whose text mentions it. */
export type TicketLinks = { mentions: TicketSummary[]; mentioned_by: TicketSummary[] }

export type ConfigInfo = {
  configuration_id: string
  data_path: string
  data_path_resolved: string
  board_file: string
  project_root: string
  id_prefix: string
  default_status: string
  done_status: string
  columns: Column[]
  priorities: string[]
  agents_path_resolved: string
  subscribers: { change: number; comment: number }
}

export type ChangeEvent = {
  event: string
  ticket_id: string
  ticket_key: string
  ticket: Ticket
  comment: Comment | null
  at: string
}

export const COLUMNS_CONFIGURATION_ID = 'kanban'
export const BOARD_PAGE_ID = 'kanban-board'
export const TICKET_PAGE_ID = 'kanban-ticket'
export const RELEASES_PAGE_ID = 'kanban-releases'
export const PROJECTS_PAGE_ID = 'kanban-projects'

/**
 * Project and release writes fire no `kanban:change` event. Pages that make one dispatch this window event
 * (same tab) so open boards and pickers read the plan again; other tabs catch up on their next read.
 */
export const PLAN_CHANGED_EVENT = 'kanban:plan-changed'

export function announcePlanChange(): void {
  window.dispatchEvent(new CustomEvent(PLAN_CHANGED_EVENT))
}

export const api = {
  board: (iii: ExtensionIii) => iii.trigger<BoardData>('kanban::board::get', {}),
  config: (iii: ExtensionIii) => iii.trigger<ConfigInfo>('kanban::config::info', {}),
  agents: (iii: ExtensionIii) =>
    iii.trigger<{ agents: AgentProfile[]; source: string; error?: string }>('kanban::agent::list', {}),
  ticket: (iii: ExtensionIii, id: string) => iii.trigger<Ticket>('kanban::ticket::get', { id }),
  create: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::create', payload),
  update: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::update', payload),
  move: (iii: ExtensionIii, payload: Record<string, unknown>) => iii.trigger<Ticket>('kanban::ticket::move', payload),
  remove: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::delete', payload),
  restore: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::restore', payload),
  comment: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<{ comment: Comment; ticket: Ticket }>('kanban::comment::create', payload),
  links: (iii: ExtensionIii, id: string) => iii.trigger<TicketLinks>('kanban::ticket::links', { id }),
  projects: (iii: ExtensionIii, includeArchived = true) =>
    iii.trigger<{ projects: ProjectView[] }>('kanban::project::list', { include_archived: includeArchived }),
  createProject: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<ProjectView>('kanban::project::create', payload),
  updateProject: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<ProjectView>('kanban::project::update', payload),
  archiveProject: (iii: ExtensionIii, id: string) => iii.trigger<ProjectView>('kanban::project::archive', { id }),
  restoreProject: (iii: ExtensionIii, id: string) => iii.trigger<ProjectView>('kanban::project::restore', { id }),
  releases: (iii: ExtensionIii) => iii.trigger<{ releases: ReleaseView[] }>('kanban::release::list', {}),
  createRelease: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<ReleaseView>('kanban::release::create', payload),
  updateRelease: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<ReleaseView>('kanban::release::update', payload),
  /** `move_open_to`: a release id, `null` to take not-done tickets out of any release; required when some aren't done. */
  shipRelease: (iii: ExtensionIii, payload: { id: string; move_open_to?: string | null }) =>
    iii.trigger<{ release: ReleaseView; moved: TicketSummary[] }>('kanban::release::ship', payload),
  reopenRelease: (iii: ExtensionIii, id: string) => iii.trigger<ReleaseView>('kanban::release::reopen', { id }),
  deleteRelease: (iii: ExtensionIii, id: string) =>
    iii.trigger<{ deleted: boolean }>('kanban::release::delete', { id }),
  convertLabel: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<ConvertResult>('kanban::label::convert', payload),
}

type Listener = (event: ChangeEvent) => void

const hubListeners = new Set<Listener>()
let hubTeardown: (() => void) | null = null

function hubInit(iii: ExtensionIii): void {
  const handlerId = 'iii::kanban-ui::events'
  const offHandler = iii.on<ChangeEvent>(handlerId, (payload) => {
    for (const listener of [...hubListeners]) {
      try {
        listener(payload)
      } catch {
        undefined
      }
    }
  })
  const offTrigger = iii.registerTrigger({
    type: 'kanban:change',
    function_id: `${handlerId}::${iii.browserId}`,
    config: {},
  })
  hubTeardown = () => {
    offTrigger()
    offHandler()
  }
}

export function subscribeChanges(iii: ExtensionIii, listener: Listener): () => void {
  hubListeners.add(listener)
  if (hubListeners.size === 1) hubInit(iii)
  return () => {
    hubListeners.delete(listener)
    if (hubListeners.size === 0 && hubTeardown) {
      hubTeardown()
      hubTeardown = null
    }
  }
}

/* ── own writes ───────────────────────────────────────────────────────── */

const ownWrites = new Map<string, string>()
const ownListeners = new Set<() => void>()
let ownVersion = 0

/**
 * The newest `updated_at` this browser tab got back from a write of its own,
 * per ticket id. A ticket that has not changed since is the user's own edit,
 * so it never counts as "changed since your last visit"; a later edit by an
 * agent does.
 */
export const OWN_WRITES: ReadonlyMap<string, string> = ownWrites

/** Record a write this tab made (the ticket as the server returned it). */
export function noteOwnWrite(ticket: { id: string; updated_at: string }): void {
  const known = ownWrites.get(ticket.id)
  if (known !== undefined && Date.parse(known) >= Date.parse(ticket.updated_at)) return
  ownWrites.set(ticket.id, ticket.updated_at)
  ownVersion += 1
  for (const listener of ownListeners) listener()
}

/** Re-render when an own write is recorded; the value only changes to say so. */
export function useOwnWritesVersion(): number {
  return useSyncExternalStore(
    (listener) => {
      ownListeners.add(listener)
      return () => ownListeners.delete(listener)
    },
    () => ownVersion,
  )
}

export function summarizeTicket(ticket: Ticket): TicketSummary {
  return {
    id: ticket.id,
    key: ticket.key,
    title: ticket.title,
    status: ticket.status,
    priority: ticket.priority,
    assignee: ticket.assignee,
    labels: ticket.labels ?? [],
    order: ticket.order,
    created_at: ticket.created_at,
    updated_at: ticket.updated_at,
    deleted_at: ticket.deleted_at,
    comment_count: ticket.comments?.length ?? 0,
    parent_id: ticket.parent_id ?? null,
    project_id: ticket.project_id ?? null,
    release_id: ticket.release_id ?? null,
  }
}

export function statusLabel(columns: Column[], status: string): string {
  return columns.find((column) => column.id === status)?.label ?? status
}

export const PRIORITY_TONE: Record<string, 'neutral' | 'warning' | 'danger' | 'accent'> = {
  low: 'neutral',
  medium: 'accent',
  high: 'warning',
  urgent: 'danger',
}

export function priorityTone(priority: string): 'neutral' | 'warning' | 'danger' | 'accent' {
  return PRIORITY_TONE[priority] ?? 'neutral'
}

/** Display text for a configured priority value (`high` → `High`). */
export function priorityLabel(priority: string): string {
  return priority ? priority.charAt(0).toUpperCase() + priority.slice(1) : priority
}

export type StatusTone = 'ghost' | 'faint' | 'ink' | 'warn' | 'ok'

/**
 * The glyph for a column, by its position, since columns are configurable:
 * first = not started (dashed), last = done (check, ok), the one before last
 * on boards of 4+ = review (pie, warn), second on 4+ = ready (circle), the
 * rest = in progress (half, ink — accent is reserved for live state).
 */
export function statusGlyph(columns: Column[], status: string) {
  const i = columns.findIndex((column) => column.id === status)
  const n = columns.length
  if (i < 0) return { Icon: Circle, tone: 'ghost' as StatusTone }
  if (i === 0) return { Icon: CircleDashed, tone: 'ghost' as StatusTone }
  if (i === n - 1) return { Icon: CircleCheck, tone: 'ok' as StatusTone }
  if (n >= 4 && i === n - 2) return { Icon: ChartPie, tone: 'warn' as StatusTone }
  if (n >= 4 && i === 1) return { Icon: Circle, tone: 'faint' as StatusTone }
  return { Icon: Contrast, tone: 'ink' as StatusTone }
}

export function StatusGlyph({ columns, status }: { columns: Column[]; status: string }) {
  const { Icon, tone } = statusGlyph(columns, status)
  return (
    <span aria-hidden="true" className="kanban-status-glyph" data-tone={tone}>
      <Icon size={16} />
    </span>
  )
}

/**
 * A priority as a Chip: a tone dot on a muted tint, the word in ink. The most
 * urgent configured priority (last in config order) is alert, the next warn,
 * the rest neutral.
 */
export function PriorityChip({ priority, priorities }: { priority: string; priorities: string[] }) {
  const index = priorities.indexOf(priority)
  const fromTop = index < 0 ? -1 : priorities.length - 1 - index
  const tone = fromTop === 0 ? 'alert' : fromTop === 1 ? 'warn' : 'neutral'
  return (
    <Chip className="kanban-priority" data-tone={tone}>
      <span aria-hidden="true" className="kanban-priority__dot" />
      {priorityLabel(priority)}
    </Chip>
  )
}
