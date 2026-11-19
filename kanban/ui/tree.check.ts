// Self-check for src/tree/agent-tree.ts: `node tree.check.ts` from kanban/ui (Node 24 strips types).
import assert from 'node:assert/strict'
import {
  type AgentSession,
  buildTree,
  formatSpan,
  modelName,
  type SessionRow,
  sessionState,
  type TreeTicket,
  ticketKeyOf,
  toAgentSession,
  walk,
  withoutDone,
} from './src/tree/agent-tree.ts'

const columns = [
  { id: 'backlog' },
  { id: 'todo' },
  { id: 'in_progress' },
  { id: 'in_review' },
  { id: 'done' },
  { id: 'canceled' },
]

let order = 0
const ticket = (key: string, status: string, extra: Partial<TreeTicket> = {}): TreeTicket => ({
  id: `id-${key}`,
  key,
  title: `Title ${key}`,
  status,
  order: ++order,
  parent_id: null,
  project_id: 'p1',
  assignee: null,
  updated_at: '2026-11-18T00:00:00.000Z',
  ...extra,
})

let clock = 1000
const session = (id: string, name: string, extra: Partial<AgentSession> = {}): AgentSession => ({
  id,
  name,
  status: 'done',
  statusReason: null,
  parentId: null,
  agent: null,
  model: null,
  icon: null,
  color: null,
  createdAt: (clock += 10),
  updatedAt: clock + 5,
  messageCount: 4,
  ...extra,
})

// ── naming conventions ──────────────────────────────────────────────────
assert.equal(ticketKeyOf({ id: 'kan8-backend', name: 'Backend · KAN-8' }, 'KAN'), 'KAN-8')
assert.equal(ticketKeyOf({ id: 'kan12-frontend', name: 'Frontend · Bookmarks' }, 'KAN'), 'KAN-12', 'key only in the id')
assert.equal(ticketKeyOf({ id: 'console-1-kan-12', name: 'KAN-12 · Persist filters' }, 'KAN'), 'KAN-12')
assert.equal(ticketKeyOf({ id: 'x', name: 'KAN-7 batch r1w1' }, 'KAN'), 'KAN-7', 'deepwork batch lead names the run')
assert.equal(ticketKeyOf({ id: 'x', name: 'kan-007 critic' }, 'KAN'), 'KAN-7', 'case and leading zeros')
assert.equal(ticketKeyOf({ id: 'kanban-ui-designer', name: 'Kanban designer' }, 'KAN'), null, 'kanban is not a key')
assert.equal(ticketKeyOf({ id: 'console-0fa3c1', name: 'Hello' }, 'KAN'), null)
assert.equal(ticketKeyOf({ id: 'x', name: 'XKAN-3' }, 'KAN'), null, 'bounded on the left')
assert.equal(ticketKeyOf({ id: 'x', name: 'KAN-31a' }, 'KAN'), 'KAN-31', 'a letter after the digits still ends the key')
assert.equal(ticketKeyOf({ id: 'x', name: 'WEB-4 · Header' }, 'WEB'), 'WEB-4', 'the board prefix, not KAN')

// ── metadata mapping ────────────────────────────────────────────────────
const row: SessionRow = {
  session_id: 'kan12-backend',
  title: 'Backend · Bookmarks',
  status: 'working',
  created_at: 1,
  updated_at: 2,
  message_count: 3,
  metadata: {
    parent_session_id: 'console-lead',
    agent_profile: { id: 'backend-engineer', name: 'Backend Engineer' },
    subagent_display: { name: 'Backend · Bookmarks', icon: 'terminal', color: 'blue' },
    model: 'anthropic::claude-opus-5',
  },
}
const mapped = toAgentSession(row)
assert.deepEqual(
  [mapped.parentId, mapped.agent?.name, mapped.icon, mapped.color, mapped.model],
  ['console-lead', 'Backend Engineer', 'terminal', 'blue', 'anthropic::claude-opus-5'],
)
assert.equal(toAgentSession({ ...row, metadata: null, title: '' }).name, 'kan12-backend', 'falls back to the id')

// ── the Tech Lead shape: one orchestrator chat, two sessions on one ticket ─
{
  const tickets = [ticket('KAN-12', 'in_progress'), ticket('KAN-5', 'todo')]
  const sessions = [
    session('console-lead', 'the bookmarks tickets on the board', { agent: { id: 'tech-lead', name: 'Tech Lead' } }),
    session('kan12-backend', 'Backend · Bookmarks', { parentId: 'console-lead', status: 'working' }),
    session('kan12-frontend', 'Frontend · Bookmarks', { parentId: 'console-lead', status: 'error' }),
    session('kan5-backend-health', 'Backend · KAN-5', { parentId: 'console-other' }),
    session('console-unrelated', 'What is 2 + 2?'),
  ]
  const tree = buildTree({ tickets, sessions, prefix: 'KAN', columns, doneStatus: 'done', canceledStatus: 'canceled' })
  const k12 = tree.roots.find((node) => node.ticket.key === 'KAN-12')
  assert.ok(k12)
  assert.equal(k12.sessions.length, 1, 'the orchestrator is the one root')
  const lead = k12.sessions[0]
  assert.equal(lead.session.id, 'console-lead')
  assert.equal(lead.inferred, true)
  assert.deepEqual(
    lead.children.map((child) => child.session.id),
    ['kan12-backend', 'kan12-frontend'],
    'two sessions on one ticket, in the order they started',
  )
  const k5 = tree.roots.find((node) => node.ticket.key === 'KAN-5')
  assert.ok(k5)
  assert.deepEqual(
    k5.sessions.map((node) => node.session.id),
    ['kan5-backend-health'],
    'its spawner is not in the list: it is a root',
  )
  // KAN-5 sits in To do, but an agent worked it (done): stays pending; KAN-12 active.
  assert.equal(k5.state, 'pending')
  assert.equal(k12.state, 'active')
  assert.deepEqual(tree.stats, { tickets: 2, sessions: 4, running: 1, failed: 1, waiting: 0 })
  assert.equal(
    tree.roots.some((node) => node.sessions.some((s) => s.session.id === 'console-unrelated')),
    false,
    'a chat that names no ticket is not drawn',
  )
}

// ── the Deepwork shape: run ticket, subtasks, leads, phases ──────────────
{
  const run = ticket('KAN-20', 'in_progress')
  const a = ticket('KAN-21', 'done', { parent_id: run.id })
  const b = ticket('KAN-22', 'in_progress', { parent_id: run.id })
  const c = ticket('KAN-23', 'todo', { parent_id: run.id })
  const other = ticket('KAN-30', 'todo', { project_id: 'p2' })
  const sessions = [
    session('orch', 'Ship pricing page', { status: 'working' }),
    session('orch-kan-21', 'KAN-21 · Research competitors', { parentId: 'orch' }),
    session('orch-kan-21-work', 'work', { parentId: 'orch-kan-21' }),
    session('orch-kan-21-verify', 'verify', { parentId: 'orch-kan-21' }),
    session('orch-kan-22', 'KAN-22 · Build checkout', { parentId: 'orch', status: 'working' }),
    session('orch-kan-22-code', 'code', { parentId: 'orch-kan-22', status: 'working' }),
    session('orch-batch', 'KAN-20 batch r1w1', { parentId: 'orch', status: 'idle', messageCount: 0 }),
    session('orch-kan-30', 'KAN-30 · Other project', { parentId: 'orch' }),
  ]
  const tree = buildTree({
    tickets: [run, a, b, c, other],
    sessions,
    prefix: 'KAN',
    columns,
    doneStatus: 'done',
    canceledStatus: 'canceled',
    inScope: (t) => t.project_id === 'p1',
  })
  assert.deepEqual(
    tree.roots.map((node) => node.ticket.key),
    ['KAN-20'],
    'subtasks nest under the run; another project stays out',
  )
  const root = tree.roots[0]
  // Siblings meet at the run ticket, but KAN-30 (another parent) widens it: the orchestrator is not attached there…
  assert.equal(
    root.sessions.some((node) => node.session.id === 'orch'),
    false,
    'children under different parents leave the orchestrator unattached',
  )
  assert.deepEqual(
    root.sessions.map((node) => node.session.id),
    ['orch-batch'],
  )
  assert.equal(root.sessions[0].state, 'pending', 'idle with no messages has not started')
  assert.deepEqual(
    root.subtasks.map((node) => node.ticket.key),
    ['KAN-21', 'KAN-22', 'KAN-23'],
    'filed order, whatever their state',
  )
  // Finishing work never re-sorts siblings: a chart watched while it changes must not move cards around.
  const later = buildTree({
    tickets: [run, { ...a }, { ...b, status: 'done' }, { ...c, status: 'in_progress' }, other],
    sessions: sessions.map((s) => ({ ...s, status: 'done' as const })),
    prefix: 'KAN',
    columns,
    doneStatus: 'done',
    canceledStatus: 'canceled',
    inScope: (t) => t.project_id === 'p1',
  })
  assert.deepEqual(
    later.roots[0].subtasks.map((node) => node.ticket.key),
    ['KAN-21', 'KAN-22', 'KAN-23'],
    'the same order after states change',
  )
  const k21 = root.subtasks.find((node) => node.ticket.key === 'KAN-21')
  assert.deepEqual(
    k21?.sessions[0].children.map((node) => node.session.id),
    ['orch-kan-21-work', 'orch-kan-21-verify'],
  )
  // Hide done: KAN-21 (done, only done sessions) drops; KAN-22 stays with its running phase.
  const live = withoutDone(tree.roots)
  assert.deepEqual(
    live[0].subtasks.map((node) => node.ticket.key),
    ['KAN-22', 'KAN-23'],
  )
  const ids: string[] = []
  walk(tree.roots, (node, depth) => ids.push(`${depth}:${node.id}`))
  assert.ok(
    ids.includes('3:session:id-KAN-22:orch-kan-22-code'),
    'project > run > subtask > lead > phase is depth 3 below the run',
  )
}

// ── the orchestrator meets at the run when every child is a sibling ─────
{
  const run = ticket('KAN-40', 'in_progress')
  const a = ticket('KAN-41', 'todo', { parent_id: run.id })
  const b = ticket('KAN-42', 'todo', { parent_id: run.id })
  const sessions = [
    session('o', 'Goal'),
    session('o-a', 'KAN-41 · A', { parentId: 'o' }),
    session('o-b', 'KAN-42 · B', { parentId: 'o' }),
    session('o-r', 'KAN-40 critic', { parentId: 'o' }),
    session('loop', 'KAN-99 · cycles', { parentId: 'loop2' }),
    session('loop2', 'x', { parentId: 'loop' }),
  ]
  const tree = buildTree({
    tickets: [run, a, b],
    sessions,
    prefix: 'KAN',
    columns,
    doneStatus: 'done',
    canceledStatus: 'canceled',
  })
  const root = tree.roots[0]
  assert.deepEqual(
    root.sessions.map((node) => node.session.id),
    ['o'],
    'the orchestrator sits on the run ticket',
  )
  assert.deepEqual(
    root.sessions[0].children.map((node) => node.session.id),
    ['o-r'],
    'the critic is the run’s own',
  )
  assert.equal(root.sessions[0].elsewhere, 2, 'the two leads show under their subtasks')
  assert.equal(
    root.subtasks.every((node) => node.sessions.length === 1),
    true,
  )
}

assert.equal(formatSpan(43_000), '43s')
assert.equal(formatSpan(12 * 60_000), '12m')
assert.equal(formatSpan(125 * 60_000), '2h 5m')
assert.equal(formatSpan(3 * 86_400_000), '3d')
assert.equal(modelName('claude-code::claude-code/claude-sonnet-5'), 'claude-sonnet-5')
assert.equal(modelName('anthropic::claude-opus-5'), 'claude-opus-5')
assert.equal(modelName(null), null)

// ── exact links (kanban::sessions::links) win over names ────────────────
{
  const run = ticket('KAN-50', 'in_progress')
  const a = ticket('KAN-51', 'in_progress', { parent_id: run.id })
  const b = ticket('KAN-52', 'todo', { parent_id: run.id })
  const sessions = [
    // A planner that only filed the two subtasks: it sits where they meet, the run.
    session('planner', 'Plan the work'),
    // A plain chat named after nothing, that worked KAN-51 and KAN-52: under both, counted once.
    session('console-abc', 'can you fix the flaky tests', { status: 'working' }),
    // Named after KAN-52, but its calls say it worked KAN-51: the record wins over the name.
    session('kan52-misnamed', 'KAN-52 · something', { status: 'done' }),
    // A phase with no calls of its own follows its parent.
    session('console-abc-verify', 'verify', { parentId: 'console-abc' }),
    // History from before sessions were recorded: still placed by its name.
    session('kan52-legacy', 'Backend · KAN-52'),
  ]
  const links = [
    { ticket_id: a.id, session_id: 'planner', writes: 1, last_action: 'ticket.created' },
    { ticket_id: b.id, session_id: 'planner', writes: 1, last_action: 'ticket.created' },
    { ticket_id: a.id, session_id: 'console-abc', writes: 3, last_action: 'comment.created' },
    { ticket_id: b.id, session_id: 'console-abc', writes: 1, last_action: 'ticket.moved' },
    { ticket_id: a.id, session_id: 'kan52-misnamed', writes: 2, last_action: 'ticket.updated' },
  ]
  const tree = buildTree({
    tickets: [run, a, b],
    sessions,
    links,
    prefix: 'KAN',
    columns,
    doneStatus: 'done',
    canceledStatus: 'canceled',
  })
  const root = tree.roots[0]
  const under = (key: string) => {
    const node = key === 'KAN-50' ? root : root.subtasks.find((n) => n.ticket.key === key)
    return node?.sessions.map((s) => s.session.id).sort() ?? []
  }
  assert.deepEqual(under('KAN-50'), ['planner'], 'a planner sits where the tickets it filed meet')
  assert.equal(root.sessions[0].inferred, true)
  assert.deepEqual(under('KAN-51'), ['console-abc', 'kan52-misnamed'], 'any chat that worked it, whatever its name')
  assert.deepEqual(
    under('KAN-52'),
    ['console-abc', 'kan52-legacy'],
    'the record beats the name; old history keeps its name',
  )
  const abc = root.subtasks.find((n) => n.ticket.key === 'KAN-51')?.sessions.find((s) => s.session.id === 'console-abc')
  assert.equal(abc?.exact, true)
  assert.deepEqual(
    abc?.children.map((c) => c.session.id),
    ['console-abc-verify'],
    'a phase follows its parent',
  )
  assert.equal(tree.stats.sessions, 5, 'a session under two tickets counts once')
  assert.equal(tree.stats.running, 1)
}

// ── a question waiting on the person ────────────────────────────────────
{
  assert.equal(sessionState(session('q', 'Asks', { status: 'done', statusReason: 'awaiting answer' })), 'waiting')
  assert.equal(sessionState(session('d', 'Done', { status: 'done' })), 'done', 'a finished turn is not waiting')
  const tree = buildTree({
    tickets: [ticket('KAN-70', 'in_progress')],
    sessions: [session('kan70-lead', 'KAN-70 · Lead', { status: 'done', statusReason: 'awaiting answer' })],
    prefix: 'KAN',
    columns,
    doneStatus: 'done',
    canceledStatus: 'canceled',
  })
  assert.equal(tree.roots[0].sessions[0].state, 'waiting')
  assert.equal(tree.stats.waiting, 1)
  assert.equal(tree.stats.running, 0)
}

console.log('tree.check: ok')
