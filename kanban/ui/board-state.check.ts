// Self-check for src/board-state.ts: `node board-state.check.ts` (Node 24 strips types).
import assert from 'node:assert/strict'
import {
  applyRead,
  applyWrite,
  buildSwimlanes,
  CELL_LIMIT,
  cellWindow,
  coerceGroup,
  dropRefusal,
  dropResurrected,
  flattenLanes,
  fullIndex,
  knownLaneIds,
  laneChange,
  laneIdOf,
  laneValueOf,
  REFUSE_HAS_SUBTASKS,
  REFUSE_OWN_PARENT,
  REFUSE_RELEASED,
  releaseState,
  shortDay,
  subtaskStats,
  type Tombstones,
  upsertSummary,
} from './src/board-state.ts'
import type { BoardData, ProjectView, ReleaseView, TicketSummary } from './src/shared.tsx'

const at = (ms: number) => new Date(Date.UTC(2026, 9, 14, 10, 0, 0, ms)).toISOString()
const tk = (
  id: string,
  status: string,
  order: number,
  ms: number,
  deleted = false,
  extra: Partial<TicketSummary> = {},
) =>
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
    parent_id: null,
    project_id: null,
    release_id: null,
    ...extra,
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

// the new facets travel with every write: a same-lane update replaces the whole summary, a move keeps them
{
  const d = board({ todo: [tk('a', 'todo', 1, 1), tk('b', 'todo', 2, 1)], doing: [] })
  const set = applyWrite(
    d,
    tk('a', 'todo', 1, 2, false, { release_id: 'r1', project_id: 'p1', parent_id: 'b' }),
    fresh(),
  )
  const a = set.columns[0].tickets[0]
  assert.deepEqual([a.release_id, a.project_id, a.parent_id], ['r1', 'p1', 'b'])
  const moved = applyWrite(
    set,
    tk('a', 'doing', 1, 3, false, { release_id: 'r1', project_id: 'p1', parent_id: 'b' }),
    fresh(),
    {
      moved: true,
    },
  )
  assert.equal(moved.columns.find((c) => c.id === 'doing')?.tickets[0].release_id, 'r1')
  const cleared = applyWrite(set, tk('a', 'todo', 1, 3, false, { release_id: null }), fresh())
  assert.equal(cleared.columns[0].tickets[0].release_id, null)
}

// ── swimlanes ──
const rel = (id: string, name: string, extra: Partial<ReleaseView> = {}) =>
  ({ id, name, target_at: null, released_at: null, ...extra }) as ReleaseView
const proj = (id: string, name: string, extra: Partial<ProjectView> = {}) =>
  ({ id, name, color: null, archived_at: null, ...extra }) as ProjectView
const ids = (list: TicketSummary[] | undefined) => (list ?? []).map((t) => t.id).join(' ')
const laneList = (l: ReturnType<typeof buildSwimlanes>) => l.map((x) => `${x.id}:${x.count}`).join(' ')
const everyone = (columns: BoardData['columns']) => new Set(columns.flatMap((c) => c.tickets.map((t) => t.id)))
const withRelease = (id: string, release: string | null, status: string, order: number) =>
  tk(id, status, order, 1, false, { release_id: release })

// release lanes: the worker's order, "No release", the released ones closed in a group; one lane per ticket, column order kept
{
  const columns = board({
    backlog: [
      withRelease('a', 'r2', 'backlog', 1),
      withRelease('b', 'r1', 'backlog', 2),
      withRelease('c', 'r2', 'backlog', 3),
      withRelease('n', null, 'backlog', 4),
    ],
    doing: [withRelease('d', 'r2', 'doing', 1)],
    done: [
      withRelease('e', 'r2', 'done', 1),
      withRelease('f', 'r0', 'done', 2),
      withRelease('g', null, 'done', 3),
      withRelease('ghost', 'deleted-release', 'done', 4),
    ],
  }).columns
  const releases = [
    rel('r1', 'Phase 1', { target_at: '2026-10-20' }),
    rel('r2', 'Phase 2'),
    rel('r3', 'Phase 3'),
    rel('r0', 'Phase 0', { released_at: '2026-09-01' }),
  ]
  const input = {
    by: 'release' as const,
    columns,
    shownIds: everyone(columns),
    releases,
    projects: [],
    doneStatus: 'done',
    showEmpty: false,
  }
  const lanes = buildSwimlanes(input)
  assert.equal(laneList(lanes), 'release:r1:1 release:r2:4 release::3 release:released:1', 'empty Phase 3 is hidden')
  const [r1, r2, none, group] = lanes
  assert.equal(ids(r2.cells.get('backlog')), 'a c', 'cells keep the order of the column')
  assert.deepEqual([r2.total, r2.done, r2.state, r2.refuse, r2.value], [4, 1, 'active', null, 'r2'])
  assert.equal(r1.state, 'planned', 'every ticket still in the first column')
  assert.equal(
    ids(none.cells.get('done')),
    'g ghost',
    'a release the board does not know yet is "No release", not gone',
  )
  assert.deepEqual([none.value, none.kind, none.name], [null, 'none', 'No release'])
  assert.deepEqual(
    [group.kind, group.name, group.refuse, group.lanes.map((l) => l.id)],
    ['group', 'Released · 1', "Released releases don't take tickets", ['release:r0']],
  )
  assert.equal(group.lanes[0].refuse, "Released releases don't take tickets")
  assert.equal(flattenLanes(lanes).length, 5, 'the group and its releases')
  const drawn = flattenLanes(lanes).flatMap((l) => [...l.cells.values()].flat())
  assert.equal(new Set(drawn.map((t) => t.id)).size, 9, 'every ticket is drawn')
  assert.equal(drawn.length, 9, 'and only once')

  const empty = buildSwimlanes({ ...input, showEmpty: true })
  assert.equal(
    laneList(empty),
    'release:r1:1 release:r2:4 release:r3:0 release::3 release:released:1',
    'show empty keeps Phase 3',
  )
  // a filter that leaves two tickets: progress still counts the whole release
  const some = buildSwimlanes({ ...input, shownIds: new Set(['a', 'e']) })
  assert.equal(laneList(some), 'release:r2:2', 'lanes without a match go, with their group')
  assert.deepEqual([some[0].total, some[0].done], [4, 1], 'progress ignores the filter')
  assert.equal(
    laneList(buildSwimlanes({ ...input, shownIds: new Set(['a']), showEmpty: true })),
    'release:r1:0 release:r2:1 release:r3:0 release::0 release:released:0',
    'show empty with a narrow filter keeps every lane',
  )
  assert.equal(
    laneList(
      buildSwimlanes({
        ...input,
        releases: [rel('r0', 'Phase 0', { released_at: 'x' })],
        shownIds: new Set(),
        showEmpty: true,
      }),
    ),
    'release::0 release:released:0',
    'no release at all still has its lane',
  )
}

// dropping on release lanes: a card writes its release only when drawn in another lane; a released one keeps its own
{
  const columns = board({
    backlog: [withRelease('a', 'r2', 'backlog', 1), withRelease('ghost', 'deleted-release', 'backlog', 2)],
    done: [withRelease('f', 'r0', 'done', 1), withRelease('n', null, 'done', 2)],
  }).columns
  const all = columns.flatMap((c) => c.tickets)
  const releases = [rel('r2', 'Phase 2'), rel('r0', 'Phase 0', { released_at: '2026-09-01' })]
  const lanes = buildSwimlanes({
    by: 'release',
    columns,
    shownIds: everyone(columns),
    releases,
    projects: [],
    doneStatus: 'done',
    showEmpty: true,
  })
  const lanesById = new Map(flattenLanes(lanes).map((l) => [l.id, l]))
  const [phase2, none, group, phase0] = ['release:r2', 'release:', 'release:released', 'release:r0'].map(
    (id) => lanesById.get(id) as (typeof lanes)[number],
  )
  const [a, ghost, f, n] = ['a', 'ghost', 'f', 'n'].map((id) => all.find((t) => t.id === id) as TicketSummary)
  const known = knownLaneIds('release', all, releases, [])
  assert.deepEqual([...known].sort(), ['r0', 'r2'])
  assert.equal(laneIdOf(ghost, 'release', known), 'release:', 'a release the board does not know is "No release"')
  assert.equal(laneIdOf(a, 'release', known), 'release:r2')
  // inside "No release" a card whose release is unknown keeps it: no write
  assert.equal(laneChange(ghost, none, known), undefined, 'ghost dropped in its own lane')
  assert.equal(laneChange(n, none, known), undefined)
  assert.equal(laneChange(a, phase2, known), undefined)
  assert.equal(laneChange(a, group, known), undefined, 'the released group takes nothing')
  assert.deepEqual(laneChange(a, none, known), { key: 'release', value: null, name: 'No release' })
  assert.deepEqual(laneChange(ghost, phase2, known), { key: 'release', value: 'r2', name: 'Phase 2' })
  // a released release takes no one, but the cards already in it can change status there
  assert.equal(dropRefusal(phase0, f, 'release:r0', false), null, 'its own lane')
  assert.equal(dropRefusal(group, f, 'release:r0', false), null, 'its own group')
  assert.equal(dropRefusal(phase0, a, 'release:r2', false), REFUSE_RELEASED)
  assert.equal(dropRefusal(group, a, 'release:r2', false), REFUSE_RELEASED)
  assert.equal(dropRefusal(phase2, a, 'release:r2', false), null)
  assert.equal(dropRefusal(none, f, 'release:r0', false), null)
}

// project lanes: active by name (natural), "No project", archived last and closed to new tickets
{
  const mk = (id: string, project: string | null, status = 'backlog') =>
    tk(id, status, 1, 1, false, { project_id: project })
  const columns = board({
    backlog: [mk('a', 'p10'), mk('b', 'p2'), mk('c', null), mk('d', 'old'), mk('e', 'p2')],
  }).columns
  const projects = [
    proj('p10', 'Team 10'),
    proj('old', 'Legacy', { archived_at: '2026-01-01' }),
    proj('p2', 'Team 2', { color: 'teal' }),
  ]
  const lanes = buildSwimlanes({
    by: 'project',
    columns,
    shownIds: everyone(columns),
    releases: [],
    projects,
    doneStatus: 'done',
    showEmpty: false,
  })
  assert.equal(
    laneList(lanes),
    'project:p2:2 project:p10:1 project::1 project:old:1',
    'Team 2 before Team 10, archived after "No project"',
  )
  assert.deepEqual(
    [lanes[0].color, lanes[3].state, lanes[3].refuse],
    ['teal', 'archived', "Archived projects don't take tickets"],
  )
  assert.equal(
    laneList(
      buildSwimlanes({
        by: 'project',
        columns,
        shownIds: new Set(['a']),
        releases: [],
        projects,
        doneStatus: 'done',
        showEmpty: true,
      }),
    ),
    'project:p2:0 project:p10:1 project::0 project:old:0',
  )
}

// parent lanes: tickets with live children by number, then "No parent" (the parents are in it); a missing parent is "No parent"
{
  const kid = (id: string, parent: string, status: string) => tk(id, status, 1, 1, false, { parent_id: parent })
  const columns = board({
    backlog: [
      tk('KAN-57', 'backlog', 1, 1),
      kid('c2', 'KAN-57', 'backlog'),
      kid('o', 'KAN-1', 'backlog'),
      tk('x', 'backlog', 4, 1),
    ],
    done: [kid('c1', 'KAN-57', 'done'), tk('KAN-9', 'done', 2, 1), kid('k9', 'KAN-9', 'done')],
  }).columns
  const input = {
    by: 'parent' as const,
    columns,
    shownIds: everyone(columns),
    releases: [],
    projects: [],
    doneStatus: 'done',
    showEmpty: false,
  }
  const lanes = buildSwimlanes(input)
  assert.equal(laneList(lanes), 'parent:KAN-9:1 parent:KAN-57:2 parent::4', 'KAN-9 before KAN-57; no empty parents')
  assert.deepEqual([lanes[1].short, lanes[1].total, lanes[1].done, lanes[1].parent?.id], ['KAN-57', 2, 1, 'KAN-57'])
  assert.equal(ids(lanes[2].cells.get('backlog')), 'KAN-57 o x')
  assert.equal(
    laneList(buildSwimlanes({ ...input, shownIds: new Set(['c1']) })),
    'parent:KAN-57:1',
    'a filter drops the other parents and "No parent"',
  )
  // drops: the worker refuses a ticket as its own parent, and one with subtasks as a subtask
  const all = columns.flatMap((c) => c.tickets)
  const [kan9, kan57] = lanes
  const parent = (id: string) => all.find((t) => t.id === id) as TicketSummary
  const known = knownLaneIds('parent', all, [], [])
  const counts = subtaskStats(all, 'done')
  assert.equal(laneIdOf(parent('o'), 'parent', known), 'parent:', 'a parent that is gone is "No parent"')
  assert.equal(laneChange(parent('o'), lanes[2], known), undefined, 'and a drop in "No parent" keeps it')
  assert.deepEqual(laneChange(parent('x'), kan57, known), { key: 'parent', value: 'KAN-57', name: 'KAN-57' })
  assert.equal(dropRefusal(kan57, parent('KAN-57'), 'parent:', counts.has('KAN-57')), REFUSE_OWN_PARENT)
  assert.equal(dropRefusal(kan9, parent('KAN-57'), 'parent:', counts.has('KAN-57')), REFUSE_HAS_SUBTASKS)
  assert.equal(dropRefusal(lanes[2], parent('KAN-57'), 'parent:', true), null, 'back to "No parent"')
  assert.equal(dropRefusal(kan9, parent('c1'), 'parent:KAN-57', false), null, 'a subtask can change parent')
  assert.equal(dropRefusal(kan57, parent('x'), 'parent:', false), null)
  const stats = subtaskStats(
    columns.flatMap((c) => c.tickets),
    'done',
  )
  assert.deepEqual(
    [stats.get('KAN-57'), stats.get('KAN-9'), stats.get('KAN-1'), stats.has('x')],
    [{ done: 1, total: 2 }, { done: 1, total: 1 }, { done: 0, total: 1 }, false],
  )
}

// a release is active once something left the first column
assert.equal(releaseState({ released_at: null }, [], 'backlog'), 'planned', 'an empty release is planned')
assert.equal(releaseState({ released_at: null }, [tk('a', 'backlog', 1, 1)], 'backlog'), 'planned')
assert.equal(
  releaseState({ released_at: null }, [tk('a', 'backlog', 1, 1), tk('b', 'todo', 1, 1)], 'backlog'),
  'active',
)
assert.equal(releaseState({ released_at: '2026-10-01' }, [tk('a', 'backlog', 1, 1)], 'backlog'), 'released')

// cells: a cell that would hide one card draws it
assert.deepEqual(
  [cellWindow(0, false), cellWindow(CELL_LIMIT + 1, false), cellWindow(CELL_LIMIT + 2, false)],
  [
    { shown: 0, hidden: 0 },
    { shown: CELL_LIMIT + 1, hidden: 0 },
    { shown: CELL_LIMIT, hidden: 2 },
  ],
)
assert.deepEqual(cellWindow(26, true), { shown: 26, hidden: 0 }, 'expanded shows all')

// drop position inside the whole column, from the cards drawn in the target cell
{
  const column = ['a', 'b', 'c', 'd', 'e'].map((id, i) => tk(id, 'todo', i + 1, 1))
  assert.equal(fullIndex(column, ['b', 'd'], 0, 'x'), 1, 'before the first drawn card')
  assert.equal(fullIndex(column, ['b', 'd'], 1, 'x'), 3, 'before the second')
  assert.equal(fullIndex(column, ['b', 'd'], 2, 'x'), 4, 'after the last drawn card, before the hidden ones')
  assert.equal(fullIndex(column, [], 0, 'x'), 5, 'an empty cell appends')
  // the dragged card is in the column: its own slot does not count
  assert.equal(fullIndex(column, ['a', 'c'], 1, 'b'), 1, 'b dropped before c')
  assert.equal(fullIndex(column, ['a', 'c'], 2, 'b'), 2, 'b dropped after c')
  assert.equal(fullIndex(column, ['b', 'c'], 0, 'b'), 1, 'a neighbour list that still names the dragged card')
}

assert.deepEqual(
  [shortDay('2026-10-30', Date.UTC(2026, 9, 14)), shortDay('2027-01-05', Date.UTC(2026, 9, 14)), shortDay('soon', 0)],
  ['Oct 30', 'Jan 5, 2027', 'soon'],
  'a target date is short, with the year only when it is another one',
)
assert.equal(
  laneValueOf(tk('a', 'todo', 1, 1, false, { release_id: 'r', project_id: 'p', parent_id: 't' }), 'project'),
  'p',
)
assert.equal(laneValueOf(tk('a', 'todo', 1, 1), 'parent'), null)
assert.deepEqual(['release', 'nope', null, undefined].map(coerceGroup), ['release', 'none', 'none', 'none'])

console.log('board-state.check: ok')
