// Self-check for src/plan-rules.ts: `node plan.check.ts` (Node 24 strips types).
import assert from 'node:assert/strict'
import {
  byProgress,
  columnSummary,
  compareReleases,
  driftTickets,
  formatDay,
  formatStamp,
  formatTarget,
  groupProjects,
  groupReleases,
  isIsoDay,
  isNextByDate,
  labelCounts,
  labelFamily,
  nameList,
  naturalCompare,
  nextRelease,
  relativeDay,
  releaseStatusText,
  resolveEntity,
  shipTargets,
  suggestName,
  topLabels,
  triggerScope,
  willAdd,
} from './src/plan-rules.ts'

const release = (name: string, target_at: string | null, extra: Record<string, unknown> = {}) =>
  ({ id: name, name, target_at, released_at: null, source_label: null, status: 'planned', ...extra }) as never
const ticket = (key: string, labels: string[], extra: Record<string, unknown> = {}) =>
  ({ id: key, key, status: 'backlog', labels, deleted_at: null, release_id: null, ...extra }) as never
const names = (list: { name: string }[]) => list.map((entry) => entry.name)

// natural order and the worker's release order
assert.ok(naturalCompare('Phase 2', 'Phase 10') < 0, 'Phase 2 before Phase 10')
assert.deepEqual(
  names(
    [
      release('Phase 10', null),
      release('Phase 2', null),
      release('Zeta', '2026-11-01'),
      release('Alpha', '2026-10-30'),
    ].sort(compareReleases),
  ),
  ['Alpha', 'Zeta', 'Phase 2', 'Phase 10'],
  'target first, undated last, natural name inside a day',
)

// sidebar groups: Released newest first
const groups = groupReleases([
  release('Phase 0', null, { status: 'released', released_at: '2026-09-01T10:00:00Z' }),
  release('Phase 1', null, { status: 'released', released_at: '2026-10-01T10:00:00Z' }),
  release('Phase 3', null, { status: 'active' }),
  release('Phase 2', '2026-10-30', { status: 'active' }),
  release('Phase 4', null),
])
assert.deepEqual(names(groups.active), ['Phase 2', 'Phase 3'])
assert.deepEqual(names(groups.planned), ['Phase 4'])
assert.deepEqual(names(groups.released), ['Phase 1', 'Phase 0'], 'released: newest first')

// the next release to move open tickets to
const all = [
  release('Phase 1', null, { released_at: '2026-10-01T10:00:00Z' }),
  release('Phase 2', '2026-10-30'),
  release('Phase 3', null),
  release('Phase 4', null),
]
assert.equal(nextRelease(all, 'Phase 2')?.name, 'Phase 3', 'next by target, released ones skipped')
assert.equal(nextRelease(all, 'Phase 3')?.name, 'Phase 4', 'the one after the current')
assert.equal(nextRelease(all, 'Phase 4')?.name, 'Phase 2', 'last: the first of the others')
assert.equal(nextRelease([all[1]], 'Phase 2'), null, 'no other release open')
assert.deepEqual(names(shipTargets(all, 'Phase 3')), ['Phase 2', 'Phase 4'], 'never itself or a released one')

// "next by target date" is claimed only for a dated release after the current one's own date
const dated = [
  release('Phase 2', '2026-10-30'),
  release('Phase 3', null),
  release('Phase 5', '2026-11-15'),
  release('Phase 1', '2026-10-01'),
]
const claim = (current: string) => {
  const [from] = dated.filter((entry: { id: string }) => entry.id === current)
  const to = nextRelease(dated, current)
  return to ? [to.name, isNextByDate(to, from)] : null
}
assert.deepEqual(claim('Phase 2'), ['Phase 5', true], 'the next dated release after this target, undated ones after it')
assert.deepEqual(claim('Phase 1'), ['Phase 2', true])
assert.deepEqual(claim('Phase 5'), ['Phase 3', false], 'only an undated one follows: it is the default, with no claim')
assert.deepEqual(claim('Phase 3'), ['Phase 1', false], 'wrapped to the first of the others: earlier, not "next"')
assert.equal(isNextByDate(release('A', '2026-10-30'), release('B', null)), false, 'nothing to be after')
assert.equal(isNextByDate(release('A', null), release('B', '2026-10-30')), false, 'no date, no claim')
assert.equal(isNextByDate(release('A', '2026-10-30'), release('B', '2026-10-30')), false, 'same day is not after')

assert.equal(releaseStatusText({ status: 'active', reopened: 0 }), 'Active')
assert.equal(releaseStatusText({ status: 'released', reopened: 0 }), 'Released')
assert.equal(releaseStatusText({ status: 'released', reopened: 2 }), 'Released · 2 reopened')
const shipped = { status: 'released' as const, released_at: '2026-10-12T09:00:00', reopened: 0 }
assert.equal(releaseStatusText(shipped, new Date(2026, 9, 14)), 'Released Oct 12', 'the day it shipped')
assert.equal(releaseStatusText({ ...shipped, reopened: 2 }, new Date(2026, 9, 14)), 'Released Oct 12 · 2 reopened')
assert.equal(
  releaseStatusText({ ...shipped, released_at: '2025-12-31T09:00:00' }, new Date(2026, 9, 14)),
  'Released Dec 31, 2025',
)
assert.equal(
  releaseStatusText({ ...shipped, status: 'active' }),
  'Active',
  'a date on a release that is not released is ignored',
)
assert.equal(
  releaseStatusText({ status: 'planned', reopened: 3 }),
  'Planned',
  'only a released release counts reopened',
)

// resolving a ref
assert.equal(resolveEntity(all, 'Phase 2')?.id, 'Phase 2')
assert.equal(resolveEntity(all, '  phase 3 ')?.id, 'Phase 3', 'by name, case and spaces ignored')
assert.equal(resolveEntity(all, 'nope'), undefined)
assert.equal(resolveEntity(all, ''), undefined)

// labels: counts merge case, deleted tickets do not count, a ticket counts once
const tickets = [
  ticket('K-1', ['phase-2', 'web']),
  ticket('K-2', ['Phase-2', 'phase-2']),
  ticket('K-3', ['phase-10']),
  ticket('K-4', ['phase-1', 'bug'], { deleted_at: '2026-10-01T00:00:00Z' }),
  ticket('K-5', ['phase-0', 'web'], { status: 'done' }),
]
assert.deepEqual(
  labelCounts(tickets),
  [
    { label: 'phase-0', count: 1 },
    { label: 'phase-2', count: 2 },
    { label: 'phase-10', count: 1 },
    { label: 'web', count: 2 },
  ],
  'case merged, deleted skipped (no bug, no phase-1), natural order',
)
assert.deepEqual(
  topLabels(tickets, 'PHASE-2').map((entry) => entry.label),
  ['web', 'phase-0', 'phase-10'],
  'most used first, the source label left out',
)

// label families
const labels = ['bug', 'phase-0', 'phase-2', 'phase-10', 'phase-1', 'qa-2026-10-14', 'sprint-3', 'v2']
assert.deepEqual(
  labelFamily('phase-2', labels),
  ['phase-0', 'phase-1', 'phase-2', 'phase-10'],
  'family in natural order',
)
assert.deepEqual(labelFamily('Phase-1', labels), ['phase-0', 'phase-1', 'phase-2', 'phase-10'], 'any member, any case')
assert.deepEqual(labelFamily('sprint-3', labels), [], 'a family needs two')
assert.deepEqual(labelFamily('bug', labels), [], 'no number, no family')
assert.deepEqual(labelFamily('2026', ['2026', '2027']), [], 'a stem needs a non-digit')
assert.deepEqual(labelFamily('qa-2026-10-14', [...labels, 'qa-2026-10-15']), ['qa-2026-10-14', 'qa-2026-10-15'])
assert.deepEqual(
  labelFamily('phase 2', ['phase 1', 'phase 2', 'phase_3']),
  ['phase 1', 'phase 2'],
  'a space separates too',
)
assert.deepEqual(labelFamily('sprint_2', ['sprint_1', 'sprint_2']), ['sprint_1', 'sprint_2'], 'so does an underscore')
// a number glued to a word is a name, not a sequence: ipv4/ipv6, es2020/es2022, h264/h265 are not families
assert.deepEqual(labelFamily('ipv4', ['ipv4', 'ipv6']), [])
assert.deepEqual(labelFamily('es2020', ['es2020', 'es2022']), [])
assert.deepEqual(labelFamily('h264', ['h264', 'h265']), [])
assert.deepEqual(labelFamily('v2', ['v1', 'v2']), [])
assert.deepEqual(
  labelFamily('phase-2', ['phase-1', 'phase-2', 'phase2']),
  ['phase-1', 'phase-2'],
  'a glued sibling is left out',
)

// names, drift, buckets, summaries
assert.equal(suggestName('phase-2'), 'Phase 2')
assert.equal(suggestName('sprint_12'), 'Sprint 12')
assert.equal(suggestName('iii-cloud'), 'iii cloud')
assert.deepEqual(
  driftTickets({ source_label: 'phase-2' }, [
    ticket('K-1', ['phase-2']),
    ticket('K-2', ['phase-2'], { release_id: 'r1' }),
    ticket('K-3', ['phase-2'], { deleted_at: 'x' }),
    ticket('K-4', ['web']),
  ]).map((entry: { key: string }) => entry.key),
  ['K-1'],
  'only live, unreleased tickets with the label',
)
assert.deepEqual(driftTickets({ source_label: null }, [ticket('K-1', ['phase-2'])]), [])
const buckets = { added: ['a', 'b'], already_here: ['c'], in_other: ['d', 'e', 'f'], deleted: [] }
assert.equal(willAdd(buckets, false), 2)
assert.equal(willAdd(buckets, true), 5, '"Move them here" adds the ones in another release')
const columns = [
  { id: 'backlog', label: 'Backlog' },
  { id: 'doing', label: 'Doing' },
  { id: 'done', label: 'Done' },
]
assert.equal(
  columnSummary([{ status: 'done' }, { status: 'backlog' }, { status: 'done' }], columns),
  '1 Backlog · 2 Done',
)
assert.equal(columnSummary([], columns), '')
assert.deepEqual(
  [
    ticket('K-2', [], { status: 'backlog' }),
    ticket('K-10', [], { status: 'doing' }),
    ticket('K-9', [], { status: 'backlog' }),
  ]
    .sort(byProgress(columns))
    .map((entry: { key: string }) => entry.key),
  ['K-10', 'K-2', 'K-9'],
  'furthest column first, then key',
)
assert.equal(nameList(['Phase 0']), 'Phase 0')
assert.equal(nameList(['Phase 0', 'Phase 1']), 'Phase 0 and Phase 1')
assert.equal(nameList(['A', 'B', 'C', 'D']), 'A, B and 2 more')
assert.deepEqual(
  (({ active, archived }) => [names(active), names(archived)])(
    groupProjects([
      { name: 'zed', status: 'active' },
      { name: 'old', status: 'archived' },
      { name: 'Alpha', status: 'active' },
    ] as never),
  ),
  [['Alpha', 'zed'], ['old']],
)

// dates
assert.ok(isIsoDay('2026-10-30') && isIsoDay('2028-02-29'))
assert.ok(
  !isIsoDay('2026-02-30') &&
    !isIsoDay('2026-13-01') &&
    !isIsoDay('30/10/2026') &&
    !isIsoDay('2026-1-1') &&
    !isIsoDay(''),
)
const today = new Date(2026, 9, 14, 23, 30)
assert.equal(relativeDay('2026-10-30', today), 'in 16 days')
assert.equal(relativeDay('2026-10-14', today), 'today')
assert.equal(relativeDay('2026-10-15', today), 'tomorrow')
assert.equal(relativeDay('2026-10-13', today), 'yesterday')
assert.equal(relativeDay('2026-10-01', today), '13 days ago')
assert.equal(relativeDay('2026-11-02', new Date(2026, 9, 30, 0, 5)), 'in 3 days', 'across a month end')
assert.equal(formatDay('2026-10-30'), 'Oct 30, 2026')
assert.equal(formatDay('2026-10-30', false), 'Oct 30')
assert.equal(formatTarget('2026-10-30', today), 'Oct 30', 'this year: no year')
assert.equal(formatTarget('2027-01-05', today), 'Jan 5, 2027', 'another year keeps it')

assert.equal(formatStamp('2026-10-14T12:00:00', today), 'Oct 14', 'this year: no year')
assert.equal(formatStamp('2025-12-31T12:00:00', today), 'Dec 31, 2025')
assert.equal(formatStamp('nope'), '')

// what a trigger binding fires on
assert.equal(triggerScope('kanban:change', {}), 'any ticket')
assert.equal(triggerScope('kanban:change', { ticket_id: 'KAN-3' }), 'KAN-3')
assert.equal(triggerScope('kanban:change', { parent_id: 'KAN-20' }), 'KAN-20 and its subtasks')
assert.equal(triggerScope('kanban:change', { release_id: 'Phase 2' }), 'tickets in Phase 2')
assert.equal(
  triggerScope('kanban:change', { parent_id: 'KAN-20', release_id: 'Phase 2' }),
  'KAN-20 and its subtasks · tickets in Phase 2',
)
assert.equal(
  triggerScope('kanban:comment', { parent_id: 'KAN-30', release_id: 'Phase 2' }),
  'KAN-30 and its subtasks',
  'comments have no release filter',
)
assert.equal(
  triggerScope('kanban:change', { ticket_id: '  ', release_id: 7 }),
  'any ticket',
  'blank and non-string filters are no filter',
)

console.log('plan: ok')
