import type { ExtensionIii } from '@iii-dev/console-ui'

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

export type ConfigInfo = {
  configuration_id: string
  data_path: string
  data_path_resolved: string
  board_file: string
  project_root: string
  id_prefix: string
  default_status: string
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
  move: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::move', payload),
  remove: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::delete', payload),
  restore: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<Ticket>('kanban::ticket::restore', payload),
  comment: (iii: ExtensionIii, payload: Record<string, unknown>) =>
    iii.trigger<{ comment: Comment; ticket: Ticket }>('kanban::comment::create', payload),
}

type Listener = (event: ChangeEvent) => void

let hubListeners = new Set<Listener>()
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

/**
 * Apply one ticket event to the board columns. The worker renumbers a whole
 * column on every move, but an event carries only the moved ticket, so the
 * card is spliced in at its 1-based `order` and both columns are renumbered
 * locally — the same layout the next `kanban::board::get` would return.
 */
export function upsertSummary(columns: BoardColumn[], summary: TicketSummary): BoardColumn[] {
  const renumber = (tickets: TicketSummary[]) => tickets.map((ticket, index) => ({ ...ticket, order: index + 1 }))
  const stripped = columns.map((column) => ({
    ...column,
    tickets: renumber(column.tickets.filter((ticket) => ticket.id !== summary.id)),
  }))
  if (summary.deleted_at) return stripped
  return stripped.map((column) => {
    if (column.id !== summary.status) return column
    const tickets = [...column.tickets]
    const position = Math.max(0, Math.min(summary.order - 1, tickets.length))
    tickets.splice(position, 0, summary)
    return { ...column, tickets: renumber(tickets) }
  })
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

