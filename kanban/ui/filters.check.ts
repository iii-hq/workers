// Self-check for src/filters.ts: `node filters.check.ts` (Node 24 strips types).
import assert from 'node:assert/strict'
import {
  coerceViews,
  EMPTY_FILTERS,
  type Filters,
  facetCounts,
  isChangedSinceVisit,
  matches,
  sameFilters,
  updatedSince,
  validVisit,
} from './src/filters.ts'

const ticket = (key: string, labels: string[], extra: Record<string, unknown> = {}) =>
  ({
    id: key,
    key,
    title: `Title of ${key}`,
    status: 'backlog',
    priority: 'medium',
    assignee: null,
    labels,
    order: 1,
    created_at: '2026-10-01T00:00:00Z',
    updated_at: '2026-10-14T10:00:00Z',
    deleted_at: null,
    comment_count: 0,
    ...extra,
  }) as never

const ctx = { lastVisit: '2026-10-14T12:00:00Z', now: Date.parse('2026-10-14T14:00:00Z') }
const f = (patch: Partial<Filters>): Filters => ({ ...EMPTY_FILTERS, ...patch })
const a = ticket('KAN-12', ['bug', 'web'])
const b = ticket('KAN-120', ['web'], { assignee: 'pm', priority: 'high', updated_at: '2026-10-14T13:00:00Z' })

assert.ok(matches(a, f({ q: '#12' }), ctx) && matches(b, f({ q: '12' }), ctx), 'number prefix matches keys')
assert.ok(matches(b, f({ q: 'kan-12 ' }), ctx), 'key substring matches')
assert.ok(matches(a, f({ labels: ['bug', 'x'] }), ctx), 'any-of labels')
assert.ok(!matches(a, f({ labels: ['bug', 'x'], labelMode: 'all' }), ctx), 'all-of labels')
assert.ok(matches(a, f({ assignees: [''] }), ctx) && !matches(b, f({ assignees: [''] }), ctx), 'unassigned is ""')
assert.ok(!matches(a, f({ updated: 'visit' }), ctx) && matches(b, f({ updated: 'visit' }), ctx), 'since last visit')
assert.ok(matches(a, f({ updated: 'visit' }), { ...ctx, lastVisit: null }), 'first visit filters nothing')

// An own write covers the ticket up to the `updated_at` it returned; a later edit by someone else counts again.
const own = (at: string) => ({ ...ctx, own: new Map([[b.id, at]]) })
assert.ok(
  isChangedSinceVisit(b, ctx) &&
    !isChangedSinceVisit(b, own(b.updated_at)) &&
    !matches(b, f({ updated: 'visit' }), own(b.updated_at)) &&
    !facetCounts([b], EMPTY_FILTERS, own(b.updated_at), 'updated').visit,
  'own edits are not "changed since your last visit"',
)
const later = ticket('KAN-120', ['web'], { assignee: 'pm', priority: 'high', updated_at: '2026-10-14T13:30:00Z' })
assert.ok(
  isChangedSinceVisit(later, own(b.updated_at)) && matches(later, f({ updated: 'visit' }), own(b.updated_at)),
  'a later edit by an agent on a ticket I edited is changed again',
)
assert.ok(!isChangedSinceVisit(later, own(later.updated_at)), 'an own write at the same updated_at covers it')

// facetCounts: no prototype (labels named like Object.prototype keys), one count per ticket, `all` keeps the label filter.
const counts = facetCounts([a, b], f({ labels: ['bug'], priorities: ['high'] }), ctx, 'labels')
assert.deepEqual({ ...counts }, { web: 1 }, 'label facets ignore the label filter but keep the others')
assert.deepEqual({ ...facetCounts([a, b], EMPTY_FILTERS, ctx, 'updated') }, { any: 2, visit: 1, today: 2, week: 2 })
assert.equal(
  facetCounts([ticket('KAN-1', ['constructor', '__proto__'])], EMPTY_FILTERS, ctx, 'labels').constructor,
  1,
  'label "constructor"',
)
assert.equal(
  Object.getOwnPropertyDescriptor(
    facetCounts([ticket('KAN-1', ['__proto__'])], EMPTY_FILTERS, ctx, 'labels'),
    '__proto__',
  )?.value,
  1,
  'label "__proto__"',
)
assert.equal(
  facetCounts([], EMPTY_FILTERS, ctx, 'labels').toString,
  undefined,
  'absent option is undefined, not a prototype function',
)
assert.equal(
  facetCounts([ticket('KAN-1', ['bug', 'bug'])], EMPTY_FILTERS, ctx, 'labels').bug,
  1,
  'a repeated label counts once',
)
const pair = [a, ticket('KAN-2', ['bug']), ticket('KAN-3', ['web'])]
assert.deepEqual(
  { ...facetCounts(pair, f({ labels: ['bug', 'web'], labelMode: 'all' }), ctx, 'labels') },
  { bug: 1, web: 1 },
  'all-of: the label facet keeps the label filter',
)
assert.deepEqual(
  { ...facetCounts(pair, f({ labels: ['bug', 'web'] }), ctx, 'labels') },
  { bug: 2, web: 2 },
  'any-of: the label facet skips the label filter',
)

// An unparseable last visit is no last visit.
const junk = { ...ctx, lastVisit: '1760000000000' }
assert.equal(updatedSince('visit', junk), null, 'garbage lastVisit never gives NaN')
assert.ok(
  matches(a, f({ updated: 'visit' }), junk) && !isChangedSinceVisit(a, junk),
  'garbage lastVisit filters nothing',
)
assert.equal(validVisit('garbage'), null)
assert.equal(validVisit('2026-10-14T12:00:00Z'), '2026-10-14T12:00:00Z')
assert.ok(
  matches(
    ticket('KAN-5', [], { updated_at: new Date(ctx.now - 7 * 864e5).toISOString() }),
    f({ updated: 'week' }),
    ctx,
  ),
  'last 7 days includes now-7d',
)

// Search ignores accents and case in titles.
const sp = ticket('KAN-7', [], { title: 'Acentuação em São Paulo' })
assert.ok(
  matches(sp, f({ q: 'acentuacao' }), ctx) &&
    matches(sp, f({ q: 'SAO paulo' }), ctx) &&
    matches(sp, f({ q: 'ACENTUAÇÃO' }), ctx),
)
assert.ok(!matches(sp, f({ q: 'sao  paulo' }), ctx), 'no other fuzziness')

assert.ok(sameFilters(f({ labels: ['a', 'b'] }), f({ labels: ['b', 'a'] })), 'selection order is irrelevant')
assert.ok(
  sameFilters(f({ labels: ['a'], labelMode: 'all' }), f({ labels: ['a'] })),
  'mode is irrelevant with one label',
)
assert.ok(
  sameFilters(f({ labels: ['a', 'a'] }), f({ labels: ['a'] })) && sameFilters(f({ q: 'Bug' }), f({ q: 'bug' })),
  'repeats and case',
)
assert.deepEqual(
  coerceViews([{ id: 'v1', name: 'Bugs', filters: { labels: ['bug', 3] } }, { id: 'all', name: 'x' }, null]),
  [{ id: 'v1', name: 'Bugs', filters: f({ labels: ['bug'] }) }],
)
assert.deepEqual(
  coerceViews([
    { id: 'a', name: 'x' },
    { id: 'a', name: 'y' },
    { id: 'b', name: 'z' },
  ]).map((v) => v.name),
  ['x', 'z'],
  'duplicate view ids keep the first',
)
console.log('filters: ok')
