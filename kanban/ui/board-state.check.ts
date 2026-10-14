// Self-check for src/board-state.ts: `node board-state.check.ts` (Node 24 strips types).
import assert from 'node:assert/strict'
import { applyRead, applyWrite, dropResurrected, type Tombstones, upsertSummary } from './src/board-state.ts'
import type { BoardData, TicketSummary } from './src/shared.tsx'

const at = (ms: number) => new Date(Date.UTC(2026, 9, 14, 10, 0, 0, ms)).toISOString()
const tk = (id: string, status: string, order: number, ms: number, deleted = false) =>
  ({
    id,
    key: id,
    title: id,
    status,
    priority: 'medium',
    assignee: null,
    labels: [],
    order,
    created_at: at(0),
    updated_at: at(ms),
    deleted_at: deleted ? at(ms) : null,
    comment_count: 0,
  }) as TicketSummary
const board = (lanes: Record<string, TicketSummary[]>, deleted = 0, ms = 1000) =>
  ({
    columns: Object.entries(lanes).map(([id, tickets]) => ({ id, label: id, tickets })),
    ticket_count: Object.values(lanes).flat().length,
    deleted_count: deleted,
    updated_at: at(ms),
  }) as BoardData
const lane = (data: BoardData, id: string) =>
  data.columns
    .find((c) => c.id === id)!
    .tickets.map((t) => `${t.id}${t.order}`)
    .join(' ')
const fresh = (): Tombstones => new Map()

// insertion by the server's `order`: gaps left by deletes are kept, not renumbered
{
  const todo = [tk('a', 'todo', 1, 1), tk('b', 'todo', 2, 1), tk('d', 'todo', 4, 1), tk('g', 'todo', 7, 1)]
  const put = (o: number) =>
    lane({ columns: upsertSummary(board({ todo }).columns, tk('n', 'todo', o, 2)) } as BoardData, 'todo')
  assert.equal(put(3), 'a1 b2 n3 d4 g7')
  assert.equal(put(5), 'a1 b2 d4 n5 g7')
  assert.equal(put(9), 'a1 b2 d4 g7 n9')
  assert.equal(put(0), 'n0 a1 b2 d4 g7')
  const same = upsertSummary(board({ todo }).columns, tk('b', 'todo', 2, 2))
  assert.equal(lane({ columns: same } as BoardData, 'todo'), 'a1 b2 d4 g7') // a card that stays keeps its slot
}

// a move mirrors the server: target lane renumbered 1..n, source lane compacted
{
  const d = board({
    backlog: [tk('x', 'backlog', 1, 1), tk('y', 'backlog', 2, 1), tk('z', 'backlog', 4, 1)],
    todo: [tk('a', 'todo', 1, 1), tk('b', 'todo', 2, 1), tk('c', 'todo', 5, 1)],
  })
  const m = applyWrite(d, tk('y', 'todo', 2, 2), fresh(), { moved: true })
  assert.equal(lane(m, 'todo'), 'a1 y2 b3 c4')
  assert.equal(lane(m, 'backlog'), 'x1 z2')
  assert.equal(lane(applyWrite(m, tk('c', 'todo', 1, 3), fresh(), { moved: true }), 'todo'), 'c1 a2 y3 b4') // within the lane
  assert.equal(lane(applyWrite(d, tk('x', 'todo', 99, 2), fresh(), { moved: true }), 'todo'), 'a1 b2 c3 x4') // clamped
}

// versions: older ignored, equal is a duplicate (same object back), newer applied
{
  const d = board({ todo: [tk('a', 'todo', 1, 5)] })
  assert.equal(applyWrite(d, { ...tk('a', 'todo', 1, 4), title: 'old' }, fresh()), d)
  assert.equal(applyWrite(d, { ...tk('a', 'todo', 1, 5), title: 'same' }, fresh()), d)
  assert.equal(applyWrite(d, { ...tk('a', 'todo', 1, 6), title: 'new' }, fresh()).columns[0].tickets[0].title, 'new')
}

// tombstones and deleted_count: only real transitions count, delete/restore in either order end correct
{
  const d = board({ todo: [tk('a', 'todo', 1, 1), tk('b', 'todo', 2, 1)] }, 3)
  const tomb = fresh()
  const del = applyWrite(d, tk('a', 'todo', 1, 3, true), tomb)
  assert.deepEqual([lane(del, 'todo'), del.deleted_count, del.ticket_count], ['b2', 4, 1])
  assert.equal(applyWrite(del, tk('a', 'todo', 1, 3, true), tomb), del) // duplicate delete
  assert.equal(applyWrite(del, tk('a', 'todo', 1, 2), tomb), del) // late reply cannot resurrect it
  const back = applyWrite(del, tk('a', 'todo', 1, 4), tomb, { restored: true })
  assert.deepEqual([lane(back, 'todo'), back.deleted_count, tomb.size], ['a1 b2', 3, 0])
  assert.equal(applyWrite(back, tk('a', 'todo', 1, 4), tomb, { restored: true }), back) // duplicate restore
  assert.equal(applyWrite(back, tk('a', 'todo', 1, 3, true), tomb), back) // late delete after the restore
  assert.equal(applyWrite(back, tk('a', 'todo', 1, 5), fresh()).deleted_count, 3) // plain update: no transition
  // deleted before this tab loaded: no tombstone, the restore flag drives the count
  const cold = applyWrite(board({ todo: [] }, 1), tk('c', 'todo', 1, 4), fresh(), { restored: true })
  assert.deepEqual([lane(cold, 'todo'), cold.deleted_count], ['c1', 0])
}

// a board read computed before a deletion cannot bring the ticket back
{
  const tomb: Tombstones = new Map([
    ['a', Date.parse(at(3))],
    ['r', Date.parse(at(3))],
  ])
  const snap = board({ todo: [tk('a', 'todo', 1, 1), tk('r', 'todo', 2, 4), tk('b', 'todo', 3, 1)] }, 2)
  const out = dropResurrected(snap, tomb)
  assert.deepEqual(
    [lane(out, 'todo'), out.deleted_count, out.ticket_count, [...tomb.keys()].join()],
    ['r2 b3', 3, 2, 'a'],
  ) // r was restored since
  assert.equal(dropResurrected(out, fresh()), out)
}

// restored into a status that is not a column while a read is in flight: counted once whichever side of the read it falls on
{
  const gone = tk('t', 'archived', 3, 6) // restored at ms 6, status has no column
  const live = (deleted: number) => board({ todo: [tk('a', 'todo', 1, 1)] }, deleted)
  const tomb = fresh()
  // another ticket stays deleted, so a wrong -1 shows (the count is clamped at 0)
  assert.equal(applyWrite(live(2), gone, tomb, { restored: true }).deleted_count, 1) // the live event
  const journal = [{ summary: gone, kind: { restored: true } }]
  assert.equal(applyRead({ ...live(1), updated_at: at(8) }, journal, tomb).deleted_count, 1) // read computed after the restore
  assert.equal(applyRead({ ...live(2), updated_at: at(5) }, journal, tomb).deleted_count, 1) // read computed before it
  assert.equal(applyRead({ ...live(1), updated_at: at(6) }, journal, tomb).deleted_count, 1) // same stamp: already in the read
  const inColumn = applyRead(
    { ...board({ todo: [tk('a', 'todo', 1, 1)] }, 0), updated_at: at(5) },
    [{ summary: tk('a', 'todo', 1, 7, true), kind: {} }],
    fresh(),
  )
  assert.deepEqual([lane(inColumn, 'todo'), inColumn.deleted_count], ['', 1]) // a delete after the read still counts
}

console.log('board-state.check: ok')
