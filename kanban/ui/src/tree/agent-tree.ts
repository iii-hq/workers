/**
 * The project tree: tickets, their subtasks, and the agent sessions that worked them.
 *
 * Kanban stores no session ids, and session-manager stores no ticket ids. What links them is how agents name
 * the sessions they spawn, which both shipped orchestrators do with the ticket's key:
 *
 * - the Tech Lead (`skills/tickets/ticket-orchestration.md`): `session_id: "kan8-backend"`,
 *   `display.name: "Backend · KAN-8"`;
 * - Deepwork (`kits/deepwork/agents/deepwork.md`): `session_id: "<own>-kan-12"`, `display.name: "KAN-12 · <title>"`,
 *   and `"<R> batch r1w1"` / `"<R> critic"` for its run ticket.
 *
 * The harness stamps the spawner on the child (`metadata.parent_session_id`) and uses the display name as the
 * title. So: a session whose title (or, failing that, id) names a key belongs to that ticket, its unnamed
 * descendants (a lead's phases) sit under it, and a session that is not named but spawned sessions that are (the
 * orchestrator chat) is attached where its children's tickets meet. Nothing here is guessed from text a model
 * wrote: only names an agent chose when it spawned, and the parent links the harness recorded.
 *
 * Pure: `buildTree` is checked by `tree.check.ts`.
 */

export type SessionStatus = 'idle' | 'working' | 'done' | 'error'

/** The fields of a `session::list` row the tree reads. */
export type SessionRow = {
  session_id: string
  title: string
  status: SessionStatus
  status_reason?: string
  kind?: string
  metadata?: Record<string, unknown> | null
  created_at: number
  updated_at: number
  message_count: number
}

/** A session as the tree draws it. */
export type AgentSession = {
  id: string
  /** The display name the spawner gave it, else the session title. */
  name: string
  status: SessionStatus
  statusReason: string | null
  parentId: string | null
  /** The agent profile it ran as (`metadata.agent_profile`). */
  agent: { id: string; name: string } | null
  /** `provider::model`, when the session recorded one. */
  model: string | null
  /** `metadata.subagent_display`: the spawner's icon and colour names. */
  icon: string | null
  color: string | null
  createdAt: number
  updatedAt: number
  messageCount: number
}

/** The ticket fields the tree reads (a board summary fits). */
export type TreeTicket = {
  id: string
  key: string
  title: string
  status: string
  order: number
  parent_id: string | null
  project_id: string | null
  assignee: string | null
  updated_at: string
}

/** Where a piece of work stands, for the badge and the "hide done" switch. */
export type WorkState = 'active' | 'pending' | 'done' | 'failed' | 'canceled'

export type SessionNode = {
  kind: 'session'
  id: string
  session: AgentSession
  /** Not named after a ticket: attached because the sessions it spawned are. */
  inferred: boolean
  state: WorkState
  children: SessionNode[]
  /** Sessions it spawned that sit under other tickets (their tickets show them). */
  elsewhere: number
}

export type TicketNode = {
  kind: 'ticket'
  id: string
  ticket: TreeTicket
  state: WorkState
  sessions: SessionNode[]
  subtasks: TicketNode[]
}

export type TreeNode = TicketNode | SessionNode

export type TreeStats = {
  tickets: number
  sessions: number
  running: number
  failed: number
}

export type ProjectTree = {
  roots: TicketNode[]
  stats: TreeStats
}

const asRecord = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
const asString = (value: unknown): string | null => (typeof value === 'string' && value.trim() ? value.trim() : null)

export function toAgentSession(row: SessionRow): AgentSession {
  const meta = asRecord(row.metadata) ?? {}
  const display = asRecord(meta.subagent_display)
  const profile = asRecord(meta.agent_profile)
  const profileId = asString(profile?.id)
  return {
    id: row.session_id,
    name: asString(display?.name) ?? asString(row.title) ?? row.session_id,
    status: row.status,
    statusReason: asString(row.status_reason),
    parentId: asString(meta.parent_session_id),
    agent: profileId ? { id: profileId, name: asString(profile?.name) ?? profileId } : null,
    model: asString(meta.model),
    icon: asString(display?.icon),
    color: asString(display?.color),
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    messageCount: row.message_count,
  }
}

const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/**
 * The ticket key a session is named after, `KAN-12`, or null. The name wins over the id: `KAN-7 batch r1w1` is
 * the run ticket even when its id says otherwise. In an id the key may lose its hyphen (`kan12-frontend`) and is
 * bounded by separators, so `kanban-…` and hex ids never match.
 */
export function ticketKeyOf(session: Pick<AgentSession, 'id' | 'name'>, prefix: string): string | null {
  const p = escapeRegExp(prefix)
  const inName = new RegExp(`(?:^|[^A-Za-z0-9])${p}-(\\d+)(?![0-9])`, 'i').exec(session.name)
  if (inName) return `${prefix.toUpperCase()}-${Number(inName[1])}`
  const inId = new RegExp(`(?:^|[^A-Za-z0-9])${p}-?(\\d+)(?=$|[^0-9])`, 'i').exec(session.id)
  if (inId) return `${prefix.toUpperCase()}-${Number(inId[1])}`
  return null
}

export function sessionState(session: AgentSession): WorkState {
  if (session.status === 'working') return 'active'
  if (session.status === 'error') return 'failed'
  if (session.status === 'done') return 'done'
  // Idle: answered and waiting, or never started.
  return session.messageCount > 0 ? 'done' : 'pending'
}

/** A ticket's state from its column; a pending ticket with an agent at work counts as active. */
export function ticketState(
  ticket: TreeTicket,
  doneStatus: string,
  canceledStatus: string | null,
  firstStatuses: ReadonlySet<string>,
): WorkState {
  if (ticket.status === doneStatus) return 'done'
  if (canceledStatus && ticket.status === canceledStatus) return 'canceled'
  return firstStatuses.has(ticket.status) ? 'pending' : 'active'
}

const STATE_RANK: Record<WorkState, number> = { active: 0, failed: 1, pending: 2, done: 3, canceled: 4 }

export type BuildInput = {
  tickets: readonly TreeTicket[]
  sessions: readonly AgentSession[]
  prefix: string
  columns: readonly { id: string }[]
  doneStatus: string
  canceledStatus: string | null
  /** The tickets to show (a project's); `undefined` = every ticket given. Links are resolved over all of them. */
  inScope?: (ticket: TreeTicket) => boolean
}

/**
 * The statuses before work starts: the board's first two columns (Backlog, To do) unless one of them is the
 * done or canceled column.
 */
function waitingStatuses(columns: readonly { id: string }[], doneStatus: string, canceledStatus: string | null) {
  return new Set(
    columns
      .slice(0, 2)
      .map((column) => column.id)
      .filter((id) => id !== doneStatus && id !== canceledStatus),
  )
}

export function buildTree(input: BuildInput): ProjectTree {
  const { tickets, sessions, prefix, doneStatus, canceledStatus } = input
  const waiting = waitingStatuses(input.columns, doneStatus, canceledStatus)
  const byKey = new Map(tickets.map((ticket) => [ticket.key.toUpperCase(), ticket]))
  const byId = new Map(tickets.map((ticket) => [ticket.id, ticket]))
  const sessionById = new Map(sessions.map((session) => [session.id, session]))
  const spawned = new Map<string, AgentSession[]>()
  for (const session of sessions) {
    if (!session.parentId || !sessionById.has(session.parentId)) continue
    const list = spawned.get(session.parentId) ?? []
    list.push(session)
    spawned.set(session.parentId, list)
  }

  // 1. Sessions named after a ticket.
  const owner = new Map<string, string>()
  for (const session of sessions) {
    const key = ticketKeyOf(session, prefix)
    const ticket = key ? byKey.get(key) : undefined
    if (ticket) owner.set(session.id, ticket.id)
  }

  // 2. An unnamed session that spawned named ones: where their tickets meet. One ticket, that ticket; siblings
  //    under one parent ticket (or that parent itself), the parent. Anything wider stays unattached.
  const inferred = new Set<string>()
  for (const session of sessions) {
    if (owner.has(session.id)) continue
    const named = (spawned.get(session.id) ?? []).map((child) => owner.get(child.id)).filter((id): id is string => !!id)
    if (named.length === 0) continue
    const meet = new Set(named.map((id) => byId.get(id)?.parent_id ?? id))
    const distinct = new Set(named)
    const target = distinct.size === 1 ? named[0] : meet.size === 1 ? [...meet][0] : null
    if (target && byId.has(target)) {
      owner.set(session.id, target)
      inferred.add(session.id)
    }
  }

  // 3. Unnamed descendants (a lead's phases) belong where their nearest owned ancestor does.
  const ownerOf = (session: AgentSession): string | null => {
    const seen = new Set<string>()
    let current: AgentSession | undefined = session
    while (current && !seen.has(current.id)) {
      seen.add(current.id)
      const own = owner.get(current.id)
      if (own) return own
      current = current.parentId ? sessionById.get(current.parentId) : undefined
    }
    return null
  }
  const ticketOfSession = new Map<string, string>()
  for (const session of sessions) {
    const own = ownerOf(session)
    if (own) ticketOfSession.set(session.id, own)
  }

  const visited = new Set<string>()
  const sessionNode = (session: AgentSession, ticketId: string): SessionNode => {
    visited.add(session.id)
    let elsewhere = 0
    const children: SessionNode[] = []
    for (const child of spawned.get(session.id) ?? []) {
      if (visited.has(child.id)) continue
      if (ticketOfSession.get(child.id) !== ticketId) {
        elsewhere += 1
        continue
      }
      children.push(sessionNode(child, ticketId))
    }
    children.sort(bySession)
    return {
      kind: 'session',
      id: `session:${session.id}`,
      session,
      inferred: inferred.has(session.id),
      state: sessionState(session),
      children,
      elsewhere,
    }
  }

  // A ticket's session roots: its sessions whose spawner is not also its own.
  const rootsByTicket = new Map<string, AgentSession[]>()
  for (const session of sessions) {
    const ticketId = ticketOfSession.get(session.id)
    if (!ticketId) continue
    const parentTicket = session.parentId ? ticketOfSession.get(session.parentId) : undefined
    if (parentTicket === ticketId) continue
    const list = rootsByTicket.get(ticketId) ?? []
    list.push(session)
    rootsByTicket.set(ticketId, list)
  }

  const inScope = input.inScope ?? (() => true)
  const shown = tickets.filter(inScope)
  const shownIds = new Set(shown.map((ticket) => ticket.id))
  const childrenOf = new Map<string, TreeTicket[]>()
  for (const ticket of shown) {
    if (!ticket.parent_id || !shownIds.has(ticket.parent_id)) continue
    const list = childrenOf.get(ticket.parent_id) ?? []
    list.push(ticket)
    childrenOf.set(ticket.parent_id, list)
  }

  const stats: TreeStats = { tickets: 0, sessions: 0, running: 0, failed: 0 }
  const count = (node: SessionNode) => {
    stats.sessions += 1
    if (node.session.status === 'working') stats.running += 1
    if (node.session.status === 'error') stats.failed += 1
    node.children.forEach(count)
  }

  const ticketNode = (ticket: TreeTicket): TicketNode => {
    stats.tickets += 1
    const sessionsHere = (rootsByTicket.get(ticket.id) ?? []).map((session) => sessionNode(session, ticket.id))
    sessionsHere.sort(bySession)
    sessionsHere.forEach(count)
    const subtasks = (childrenOf.get(ticket.id) ?? []).map(ticketNode)
    subtasks.sort(byTicket)
    let state = ticketState(ticket, doneStatus, canceledStatus, waiting)
    // Not started on the board, but an agent is at it: that is active work.
    if (state === 'pending' && sessionsHere.some(hasActive)) state = 'active'
    return { kind: 'ticket', id: `ticket:${ticket.id}`, ticket, state, sessions: sessionsHere, subtasks }
  }

  const roots = shown.filter((ticket) => !ticket.parent_id || !shownIds.has(ticket.parent_id)).map(ticketNode)
  roots.sort(byTicket)
  return { roots, stats }
}

const hasActive = (node: SessionNode): boolean => node.state === 'active' || node.children.some(hasActive)

/** Running first, then by when it started: the order the work happened in. */
function bySession(a: SessionNode, b: SessionNode): number {
  return STATE_RANK[a.state] - STATE_RANK[b.state] || a.session.createdAt - b.session.createdAt
}

/** Active work first, then waiting, then finished; the board's order inside each. */
function byTicket(a: TicketNode, b: TicketNode): number {
  return (
    STATE_RANK[a.state] - STATE_RANK[b.state] ||
    a.ticket.order - b.ticket.order ||
    a.ticket.key.localeCompare(b.ticket.key, 'en', { numeric: true })
  )
}

/** Every node id in the tree, depth first: "expand all", and the default open set. */
export function walk(roots: readonly TreeNode[], visit: (node: TreeNode, depth: number) => void, depth = 0): void {
  for (const node of roots) {
    visit(node, depth)
    walk(childrenOfNode(node), visit, depth + 1)
  }
}

export function childrenOfNode(node: TreeNode): TreeNode[] {
  return node.kind === 'ticket' ? [...node.sessions, ...node.subtasks] : node.children
}

/** Finished and canceled tickets and finished sessions drop out; their active descendants keep them. */
export function withoutDone<T extends TreeNode>(nodes: readonly T[]): T[] {
  const out: T[] = []
  for (const node of nodes) {
    if (node.kind === 'ticket') {
      const sessions = withoutDone(node.sessions)
      const subtasks = withoutDone(node.subtasks)
      const finished = node.state === 'done' || node.state === 'canceled'
      if (finished && sessions.length === 0 && subtasks.length === 0) continue
      out.push({ ...node, sessions, subtasks })
    } else {
      const children = withoutDone(node.children)
      if (node.state === 'done' && children.length === 0) continue
      out.push({ ...node, children })
    }
  }
  return out
}

/** `43s`, `12m`, `2h 5m`, `3d`: how long a session has run, from its first to its latest write. */
export function formatSpan(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000))
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return m % 60 ? `${h}h ${m % 60}m` : `${h}h`
  return `${Math.floor(h / 24)}d`
}

/** `anthropic::claude-opus-5` → `claude-opus-5`; `claude-code::claude-code/claude-sonnet-5` → `claude-sonnet-5`. */
export function modelName(model: string | null): string | null {
  if (!model) return null
  const tail = model.split('::').pop() ?? model
  return tail.split('/').pop() || tail
}
