import { useEffect, useState, type ReactNode } from 'react'
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

export function useContainerNarrow(below: number): [(node: HTMLElement | null) => void, boolean] {
  const [node, setNode] = useState<HTMLElement | null>(null)
  const [narrow, setNarrow] = useState(false)
  useEffect(() => {
    if (!node) return
    const measure = () => {
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < below)
    }
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(node)
    return () => observer.disconnect()
  }, [node, below])
  return [setNode, narrow]
}

export function usePaneState<T>(key: string, initial: T): [T, (next: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key)
      return raw === null ? initial : (JSON.parse(raw) as T)
    } catch {
      return initial
    }
  })
  const update = (next: T) => {
    setValue(next)
    try {
      window.localStorage.setItem(key, JSON.stringify(next))
    } catch {
      undefined
    }
  }
  return [value, update]
}

export function errorMessage(error: unknown): string {
  if (!error) return 'Something went wrong.'
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  const record = error as Record<string, unknown>
  if (typeof record.message === 'string') return record.message
  return JSON.stringify(error)
}

export function relativeTime(iso: string): string {
  const then = new Date(iso).getTime()
  if (!Number.isFinite(then)) return iso
  const seconds = Math.round((Date.now() - then) / 1000)
  if (seconds < 45) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.round(hours / 24)
  if (days < 30) return `${days}d ago`
  return new Date(iso).toLocaleDateString()
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

type IconProps = { className?: string }

function Glyph({ className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      aria-hidden
      className={className}
      fill="none"
      height="16"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
      width="16"
    >
      {children}
    </svg>
  )
}

export function ColumnsIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <rect height="18" rx="2" width="6" x="3" y="3" />
      <rect height="18" rx="2" width="6" x="15" y="3" />
    </Glyph>
  )
}

export function TicketIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M3 9V6a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v3a2 2 0 0 0 0 6v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-3a2 2 0 0 0 0-6Z" />
      <path d="M13 5v2M13 11v2M13 17v2" />
    </Glyph>
  )
}

export function PlusIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M5 12h14M12 5v14" />
    </Glyph>
  )
}

export function TrashIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6" />
      <path d="M10 11v6M14 11v6" />
    </Glyph>
  )
}

export function RefreshIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
      <path d="M21 3v5h-5" />
    </Glyph>
  )
}

export function BackIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M19 12H5M12 19l-7-7 7-7" />
    </Glyph>
  )
}

export function MessageIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M21 12a8 8 0 0 1-8 8H7l-4 3v-6.5A8 8 0 0 1 11 4h2a8 8 0 0 1 8 8Z" />
    </Glyph>
  )
}

export function ClockIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </Glyph>
  )
}

export function UserIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M20 21a8 8 0 0 0-16 0" />
      <circle cx="12" cy="7" r="4" />
    </Glyph>
  )
}

export function AlertIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M12 9v4M12 17h.01" />
      <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" />
    </Glyph>
  )
}

export function ChevronRightIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="m9 18 6-6-6-6" />
    </Glyph>
  )
}

export function EditIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </Glyph>
  )
}

export function ExpandIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" />
    </Glyph>
  )
}

export function CheckIcon({ className }: IconProps) {
  return (
    <Glyph className={className}>
      <path d="M20 6 9 17l-5-5" />
    </Glyph>
  )
}

export function CommentIcon({ className }: IconProps) {
  return <MessageIcon className={className} />
}
