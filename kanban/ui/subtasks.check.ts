// Self-check for src/subtask-rules.ts: `node subtasks.check.ts` from kanban/ui (Node 24 strips types).
import assert from 'node:assert/strict'
import {
  archivedProjectIn,
  attachOrder,
  attachReasons,
  columnBefore,
  doneWarning,
  groupSubtaskAdds,
  keyNumber,
  laneName,
  parentBlockedReason,
  parentFacet,
  parentOffer,
  parentOptions,
  parentSentence,
  planSentence,
  prefillHint,
  progressPercent,
  projectOptions,
  resolveDefaults,
  releaseGroups,
  shortDate,
  siblings,
  subtaskDefaults,
  subtaskSentence,
} from './src/subtask-rules.ts'

const columns = [
  { id: 'backlog', label: 'Backlog' },
  { id: 'in_review', label: 'In review' },
  { id: 'done', label: 'Done' },
]
const brief = (key: string, status = 'backlog', extra = {}) => ({
  id: `id-${key}`,
  key,
  title: `Title ${key}`,
  status,
  priority: 'medium',
  assignee: null,
  deleted_at: null,
  project_id: null,
  release_id: null,
  ...extra,
})
const row = (key: string, extra = {}) => ({
  ...brief(key),
  labels: [],
  order: 0,
  created_at: '',
  updated_at: '',
  comment_count: 0,
  parent_id: null,
  ...extra,
})

// dates and keys
assert.equal(shortDate('2026-10-30'), 'Oct 30')
assert.equal(shortDate('2026-01-05'), 'Jan 5')
assert.equal(shortDate(null), null)
assert.equal(shortDate('soon'), null)
assert.equal(keyNumber('KAN-182'), 182)
assert.equal(keyNumber('nope'), Number.POSITIVE_INFINITY)

// projects: active by name, an archived one stays only while the ticket is in it
const project = (id: string, name: string, archived = false) =>
  ({ id, name, archived_at: archived ? '2026-01-01T00:00:00Z' : null, total: 3 }) as never
const projects = [
  project('p2', 'iii cloud'),
  project('p1', 'Alpha 10'),
  project('p0', 'Alpha 2'),
  project('pa', 'Old', true),
]
assert.deepEqual(
  projectOptions(projects, null).map((o) => o.label),
  ['Alpha 2', 'Alpha 10', 'iii cloud'],
  'natural order, no archived',
)
assert.deepEqual(projectOptions(projects, 'pa').at(-1)?.label, 'Old (archived)')
assert.equal(projectOptions(projects, 'gone').at(-1)?.label, 'Unknown project')
assert.equal(projectOptions(projects, null)[0]?.description, '3 tickets')

// releases: released ones under their own heading, worker order kept
const release = (id: string, name: string, released: boolean, target: string | null = null) =>
  ({ id, name, released_at: released ? '2026-09-01T00:00:00Z' : null, target_at: target, total: 1 }) as never
const groups = releaseGroups(
  [release('r2', 'Phase 2', false, '2026-10-30'), release('r3', 'Phase 3', false), release('r1', 'Phase 1', true)],
  null,
)
assert.deepEqual(
  groups.map((g) => [g.label, g.options.map((o) => o.label)]),
  [
    ['', ['Phase 2', 'Phase 3']],
    ['Released', ['Phase 1']],
  ],
)
assert.equal(groups[0]?.options[0]?.description, 'Oct 30 · 1 ticket')
assert.equal(groups[0]?.options[1]?.description, '1 ticket')
assert.equal(releaseGroups([], 'rx')[0]?.options[0]?.label, 'Unknown release')

// parent: live top-level tickets, never itself; a deleted current parent stays listed
const board = [
  row('KAN-61'),
  row('KAN-9'),
  row('KAN-149', { parent_id: 'id-KAN-57' }),
  row('KAN-96', { deleted_at: 'x' }),
  row('KAN-57'),
]
assert.deepEqual(
  parentOptions(board, 'id-KAN-9', null).map((o) => o.value),
  ['KAN-57', 'KAN-61'],
  'by number, minus self, subtasks and deleted',
)
const gone = brief('KAN-3', 'done', { deleted_at: 'x' })
assert.deepEqual(parentOptions(board, 'id-KAN-9', gone)[0], {
  value: 'KAN-3',
  label: 'KAN-3 (deleted)',
  description: 'Title KAN-3',
})
assert.equal(parentBlockedReason({ children: [] }), null)
assert.equal(parentBlockedReason({ children: [brief('KAN-1')] }), "Has 1 subtask, so it can't be a subtask.")
assert.equal(
  parentFacet({ parent: brief('KAN-57', 'done', { release_id: 'r2' }), project_id: null, release_id: null }, 'release'),
  'r2',
)
assert.equal(
  parentFacet({ parent: brief('KAN-57', 'done', { release_id: 'r2' }), project_id: null, release_id: 'r3' }, 'release'),
  null,
  'its own release wins',
)
assert.equal(parentFacet({ parent: null, project_id: null, release_id: null }, 'project'), null)

// the offer "Add to <parent's release>": a project or release that takes new tickets, by name
const offerReleases = [release('r2', 'Phase 2', false), release('r1', 'Phase 1', true)] as never[]
const offerProjects = [project('p2', 'iii cloud'), project('pa', 'Old', true)]
const under = (extra: object) => ({ parent: brief('KAN-57', 'done', extra), project_id: null, release_id: null })
assert.deepEqual(parentOffer(under({ release_id: 'r2' }), 'release', offerProjects, offerReleases), {
  id: 'r2',
  name: 'Phase 2',
})
assert.equal(parentOffer(under({ release_id: 'r1' }), 'release', offerProjects, offerReleases), null, 'released: none')
assert.equal(parentOffer(under({ release_id: 'gone' }), 'release', offerProjects, offerReleases), null)
assert.deepEqual(parentOffer(under({ project_id: 'p2' }), 'project', offerProjects, offerReleases), {
  id: 'p2',
  name: 'iii cloud',
})
assert.equal(parentOffer(under({ project_id: 'pa' }), 'project', offerProjects, offerReleases), null, 'archived: none')
assert.equal(
  parentOffer({ ...under({ release_id: 'r2' }), release_id: 'r1' }, 'release', offerProjects, offerReleases),
  null,
  'its own release wins',
)
assert.equal(parentOffer(under({ release_id: 'r2' }), 'release', [], []), null, 'board not read yet')

// add an existing ticket: why a row is disabled
const reasons = attachReasons(
  [
    row('KAN-57'),
    row('KAN-61'),
    row('KAN-62', { parent_id: 'id-KAN-61' }),
    row('KAN-149', { parent_id: 'id-KAN-57' }),
    row('KAN-96', { deleted_at: 'x' }),
    row('KAN-133'),
    row('KAN-70', { parent_id: 'id-KAN-99' }),
    row('KAN-80', { parent_id: 'id-KAN-96', deleted_at: 'x' }),
  ],
  'id-KAN-57',
)
assert.equal(reasons.get('id-KAN-57'), 'this ticket')
assert.equal(reasons.get('id-KAN-61'), 'has its own subtasks')
assert.equal(reasons.get('id-KAN-62'), 'already a subtask of KAN-61')
assert.equal(reasons.get('id-KAN-149'), 'already a subtask of KAN-57')
assert.equal(reasons.get('id-KAN-96'), 'deleted')
assert.equal(reasons.get('id-KAN-133'), null)
assert.equal(reasons.get('id-KAN-70'), 'already a subtask of another ticket', 'parent not on the board')
const ordered = attachOrder(
  [row('KAN-61'), row('KAN-133'), row('KAN-9')],
  new Map([['id-KAN-61', 'has its own subtasks']]),
)
assert.deepEqual(
  ordered.map((t) => t.key),
  ['KAN-9', 'KAN-133', 'KAN-61'],
  'free ones first, each by number',
)
// a deleted child does not make its parent a parent
assert.equal(
  attachReasons([row('KAN-1'), row('KAN-2', { parent_id: 'id-KAN-1', deleted_at: 'x' })], 'id-other').get('id-KAN-1'),
  null,
)

// the done warning
const kids = [
  brief('KAN-128', 'done'),
  brief('KAN-138'),
  brief('KAN-141', 'in_review'),
  brief('KAN-5', 'backlog', { deleted_at: 'x' }),
]
const warn = doneWarning({ status: 'done', children: kids, deleted_at: null }, 'done', columns)
assert.deepEqual(
  warn?.open.map((c) => c.key),
  ['KAN-138', 'KAN-141'],
  'in review is not done; deleted is not counted',
)
assert.equal(warn?.back?.label, 'In review')
assert.equal(doneWarning({ status: 'in_review', children: kids, deleted_at: null }, 'done', columns), null)
assert.equal(
  doneWarning({ status: 'done', children: [brief('KAN-1', 'done')], deleted_at: null }, 'done', columns),
  null,
)
assert.equal(doneWarning({ status: 'done', children: kids, deleted_at: 'x' }, 'done', columns), null)
assert.equal(
  doneWarning({ status: 'backlog', children: kids, deleted_at: null }, 'backlog', columns)?.back,
  null,
  'nothing before the first column',
)
assert.equal(columnBefore(columns, 'backlog'), null)
assert.equal(columnBefore(columns, 'nope'), null)
assert.deepEqual(progressPercent({ done: 2, total: 12 }), 17)
assert.deepEqual(progressPercent({ done: 0, total: 0 }), 0)

// siblings
const ids = [brief('KAN-1'), brief('KAN-2'), brief('KAN-3')]
assert.deepEqual(siblings(ids, 'id-KAN-2'), { previous: ids[0], next: ids[2] })
assert.deepEqual(siblings(ids, 'id-KAN-1'), { previous: null, next: ids[1] })
assert.deepEqual(siblings(ids, 'id-KAN-3'), { previous: ids[1], next: null })
assert.deepEqual(siblings(ids, 'id-missing'), { previous: null, next: null })

// a new subtask: prefilled once from the parent; an archived project and a released release are left out
const parent = { key: 'KAN-57', project_id: 'p2', release_id: 'r2' }
const releases = [release('r2', 'Phase 2', false), release('r1', 'Phase 1', true)]
assert.deepEqual(subtaskDefaults(parent, projects, releases, 'backlog'), {
  parent: 'KAN-57',
  status: 'backlog',
  project: 'p2',
  release: 'r2',
})
assert.deepEqual(subtaskDefaults({ ...parent, project_id: 'pa' }, projects, releases, 'backlog'), {
  parent: 'KAN-57',
  status: 'backlog',
  release: 'r2',
})
assert.deepEqual(subtaskDefaults({ ...parent, release_id: 'r1' }, projects, releases, 'backlog'), {
  parent: 'KAN-57',
  status: 'backlog',
  project: 'p2',
})
assert.deepEqual(subtaskDefaults({ ...parent, release_id: 'unread' }, projects, [], 'backlog'), {
  parent: 'KAN-57',
  status: 'backlog',
  project: 'p2',
  release: 'unread',
})
assert.deepEqual(subtaskDefaults({ key: 'KAN-1', project_id: null, release_id: null }, projects, releases, undefined), {
  parent: 'KAN-1',
})
assert.equal(
  prefillHint('Backlog', true, true),
  "New subtasks start in Backlog with this ticket's project and release — you can change them.",
)
assert.equal(
  prefillHint('Backlog', false, true),
  "New subtasks start in Backlog with this ticket's release — you can change them.",
)
assert.equal(prefillHint('Backlog', false, false), 'New subtasks start in Backlog.')

// the New ticket dialog: swimlane defaults by id, name or key; what is gone or closed to new tickets is dropped
const plan = {
  projects,
  releases: [release('r2', 'Phase 2', false), release('r1', 'Phase 1', true)] as never[],
  tickets: [row('KAN-57'), row('KAN-149', { parent_id: 'id-KAN-57' }), row('KAN-96', { deleted_at: 'x' })],
}
assert.deepEqual(resolveDefaults(plan, undefined), { project: null, release: null, parent: null })
assert.deepEqual(resolveDefaults(plan, { project: 'p2', release: 'r2', parent: 'id-KAN-57' }), {
  project: 'p2',
  release: 'r2',
  parent: 'KAN-57',
})
assert.deepEqual(
  resolveDefaults(plan, { project: ' III Cloud ', release: 'phase 2', parent: 'kan-57' }),
  { project: 'p2', release: 'r2', parent: 'KAN-57' },
  'names and keys ignore case',
)
assert.deepEqual(
  resolveDefaults(plan, { project: 'pa', release: 'r1', parent: 'KAN-149' }),
  { project: null, release: null, parent: null },
  'archived project, released release and a subtask are dropped (the board refuses new tickets there too)',
)
assert.deepEqual(resolveDefaults(plan, { project: null, release: '', parent: 'KAN-96' }), {
  project: null,
  release: null,
  parent: null,
})
assert.equal(laneName(plan, { project: 'p2', release: 'r2', parent: null }), 'Phase 2')
assert.equal(laneName(plan, { project: 'p2', release: null, parent: 'KAN-57' }), 'iii cloud')
assert.equal(laneName(plan, { project: null, release: null, parent: 'KAN-57' }), 'KAN-57')
assert.equal(laneName(plan, { project: null, release: null, parent: null }), null)
// the swimlane's own name wins: a parent lane that also carries a release is still "KAN-57", not "Phase 2"
assert.equal(laneName(plan, { project: 'p2', release: 'r2', parent: 'KAN-57' }, 'KAN-57'), 'KAN-57')
assert.equal(laneName(plan, { project: null, release: null, parent: null }, 'Phase 9'), 'Phase 9')
assert.equal(laneName(plan, { project: 'p2', release: 'r2', parent: null }, ''), 'Phase 2', 'a blank name falls back')
assert.equal(laneName(plan, { project: 'p2', release: 'r2', parent: null }, null), 'Phase 2')

// the refusal the worker gives for a project that was archived after the screen read it
assert.equal(
  archivedProjectIn('project iii cloud is archived; restore it before filing tickets under it'),
  'iii cloud',
)
assert.equal(
  archivedProjectIn('kanban::ticket::create: project Phase 1 is archived; restore it before filing tickets'),
  'Phase 1',
  'the name survives a wrapper in front of the message',
)
assert.equal(archivedProjectIn('release Phase 2 is already released'), null)
assert.equal(archivedProjectIn('KAN-3 is deleted; restore it before making it a subtask'), null)

// sentences
assert.deepEqual(subtaskSentence('subtask.added', [{ field: 'subtask', from: null, to: 'KAN-182' }]), [
  'added subtask ',
  { ticket: 'KAN-182' },
])
assert.deepEqual(subtaskSentence('subtask.removed', [{ field: 'subtask', from: 'KAN-182', to: null }]), [
  'removed subtask ',
  { ticket: 'KAN-182' },
])
assert.deepEqual(subtaskSentence('subtask.added', []), ['added a subtask'])
assert.deepEqual(parentSentence({ field: 'parent', from: null, to: 'KAN-57' }), [
  'made this a subtask of ',
  { ticket: 'KAN-57' },
])
assert.deepEqual(parentSentence({ field: 'parent', from: 'KAN-57', to: null }), [
  'detached this from ',
  { ticket: 'KAN-57' },
])
assert.deepEqual(parentSentence({ field: 'parent', from: 'KAN-57', to: 'KAN-60' }), [
  'made this a subtask of ',
  { ticket: 'KAN-60' },
  ' instead of ',
  { ticket: 'KAN-57' },
])
assert.equal(planSentence({ field: 'release', from: null, to: 'Phase 2' }), 'added this to Phase 2')
assert.equal(planSentence({ field: 'release', from: 'Phase 2', to: 'Phase 3' }), 'moved this from Phase 2 to Phase 3')
assert.equal(planSentence({ field: 'project', from: 'iii cloud', to: null }), 'removed this from iii cloud')
assert.equal(planSentence({ field: 'project', from: null, to: null }), 'changed the project')

// grouping: a run by one actor, nothing else in between
const add = (n: number, actor = 'bot') => ({
  entry: {
    id: `a${n}`,
    ticket_id: 't',
    seq: n,
    type: 'subtask.added',
    actor,
    at: '',
    changes: [{ field: 'subtask', from: null, to: `KAN-${n}` }],
  },
})
const other = (n: number) => ({
  entry: { id: `o${n}`, ticket_id: 't', seq: n, type: 'ticket.updated', actor: 'bot', at: '' },
})
const shape = (items: ReturnType<typeof add>[]) =>
  groupSubtaskAdds(items).map((r) => (r.kind === 'row' ? r.item.entry.id : r.keys.join(',')))
assert.deepEqual(shape([add(128), add(132), add(167)]), ['KAN-128,KAN-132,KAN-167'])
assert.deepEqual(shape([add(128)]), ['a128'], 'one stays a plain row')
assert.deepEqual(shape([add(1), other(2), add(3), add(4)]), ['a1', 'o2', 'KAN-3,KAN-4'], 'a run ends at another entry')
assert.deepEqual(shape([add(1), add(2, 'user'), add(3, 'user')]), ['a1', 'KAN-2,KAN-3'], 'one actor per run')
assert.deepEqual(shape([]), [])
assert.equal(groupSubtaskAdds([add(1), add(2)])[0]?.kind, 'adds')

console.log('subtasks.check.ts: ok')
