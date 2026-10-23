/**
 * The pure rules, in Node (the flows run them in QuickJS too): the board
 * state (writes, reads, pages, partial lanes and their totals), the drop
 * index under filters, the filters, the activity threading, the card fields.
 */
import { describe, expect, it } from 'vitest'
import { timeline } from '../src/activity'
import {
  type Activity,
  type BoardRead,
  boardOf,
  type Comment,
  changeOf,
  type Summary,
  type Ticket,
  ticketOf,
} from '../src/api'
import { appendPage, applyRead, applyWrite, dropIndex, type Lane, newMemo, placeMove, upsert } from '../src/board'
import { cardRow } from '../src/cards'
import { coerceFilters, EMPTY_FILTERS, facetCounts, isChangedSinceVisit, matches } from '../src/filters'
import { ago, count, priorityRank, statusStep } from '../src/format'
import { doneWarning, pickRefusal, subtaskDefaults } from '../src/rules'

const at = (s: number) => new Date(Date.UTC(2026, 9, 1, 0, 0, s)).toISOString()

function card(id: string, status: string, order: number, fields: Partial<Summary> = {}): Summary {
  return {
    id,
    key: `KAN-${id.slice(1)}`,
    title: id,
    status,
    priority: 'medium',
    assignee: null,
    labels: [],
    pills: [],
    order,
    created_at: at(0),
    updated_at: at(order),
    deleted_at: null,
    comment_count: 0,
    parent_id: null,
    project_id: null,
    release_id: null,
    ...fields,
  }
}

const lane = (id: string, rows: Summary[], total = rows.length): Lane => ({ id, label: id, rows, total })
const ids = (l: Lane | undefined) => l?.rows.map((r) => r.id)
const by = (lanes: Lane[], id: string) => lanes.find((l) => l.id === id)

describe('the board state', () => {
  const base = () => [
    lane('todo', [card('t1', 'todo', 1), card('t2', 'todo', 2), card('t3', 'todo', 3)]),
    lane('done', [card('t4', 'done', 1), card('t5', 'done', 2)], 40),
  ]

  it('a moved card goes to its position and both lanes are renumbered; totals follow', () => {
    const next = upsert(base(), card('t1', 'done', 2, { updated_at: at(50) }), { moved: true })
    expect(ids(by(next, 'todo'))).toEqual(['t2', 't3'])
    expect(by(next, 'todo')?.rows.map((r) => r.order)).toEqual([1, 2])
    expect(ids(by(next, 'done'))).toEqual(['t4', 't1', 't5'])
    expect(by(next, 'done')?.rows.map((r) => r.order)).toEqual([1, 2, 3])
    expect([by(next, 'todo')?.total, by(next, 'done')?.total]).toEqual([2, 41])
  })

  it('a card past the rows of a partial lane only counts; in a whole lane it is appended', () => {
    // Moved to position 30 of Done (40 cards, 2 shown): not drawn.
    let next = upsert(base(), card('t1', 'done', 30, { updated_at: at(50) }), { moved: true })
    expect(ids(by(next, 'done'))).toEqual(['t4', 't5'])
    expect(by(next, 'done')?.total).toBe(41)
    // A status change by update goes to the end of a whole lane.
    next = upsert(base(), card('t4', 'todo', 9, { updated_at: at(60) }), {})
    expect(ids(by(next, 'todo'))).toEqual(['t1', 't2', 't3', 't4'])
    expect(by(next, 'done')?.total).toBe(39)
  })

  it('a card the app does not hold leaves the lane its event names, and enters by its number', () => {
    const lanes = base()
    // Unknown t9 moves from Done (unseen part) into To do at position 2.
    let next = upsert(lanes, card('t9', 'todo', 2, { updated_at: at(70) }), { moved: true, from: 'done' })
    expect(ids(by(next, 'todo'))).toEqual(['t1', 't9', 't2', 't3'])
    expect([by(next, 'todo')?.total, by(next, 'done')?.total]).toEqual([4, 39])
    // A reorder inside the unseen part of Done: no lane change, nothing drawn.
    next = upsert(lanes, card('t8', 'done', 20, { updated_at: at(71) }), { moved: true })
    expect(next.map((l) => l.total)).toEqual([3, 40])
    // Created at the end of a partial lane: counted, not drawn.
    next = upsert(lanes, card('t7', 'done', 41, { updated_at: at(72) }), { entered: true })
    expect([ids(by(next, 'done')), by(next, 'done')?.total]).toEqual([['t4', 't5'], 41])
    // Deleted: out of its lane.
    next = upsert(lanes, card('t2', 'todo', 2, { updated_at: at(73), deleted_at: at(73) }), {})
    expect([ids(by(next, 'todo')), by(next, 'todo')?.total]).toEqual([['t1', 't3'], 2])
  })

  it('a card counted but not drawn leaves the lane that counted it, once', () => {
    const placed = new Map<string, string>()
    // Moved to the end of the partial Done: counted there, not drawn, remembered.
    let next = placeMove(base(), 't1', 'done', undefined, placed)
    expect([by(next, 'todo')?.total, by(next, 'done')?.total, placed.get('t1')]).toEqual([2, 41, 'done'])
    // Its answer names the column it came from; the placed lane wins, so nothing counts twice.
    next = upsert(next, card('t1', 'done', 41, { updated_at: at(90) }), { moved: true, from: 'todo' }, placed)
    expect([by(next, 'todo')?.total, by(next, 'done')?.total]).toEqual([2, 41])
    // A later move out of Done takes it from there, and the lane it left is renumbered.
    next = upsert(next, card('t1', 'todo', 1, { updated_at: at(91) }), { moved: true }, placed)
    expect([ids(by(next, 'todo')), by(next, 'done')?.total, placed.has('t1')]).toEqual([['t1', 't2', 't3'], 40, false])
  })

  it('applies a write once, and never a version older than one it has, nor one from before a deletion', () => {
    const memo = newMemo()
    let lanes = base()
    lanes = applyWrite(lanes, card('t1', 'todo', 1, { title: 'new', updated_at: at(10) }), memo)
    expect(by(lanes, 'todo')?.rows[0].title).toBe('new')
    const same = applyWrite(lanes, card('t1', 'todo', 1, { title: 'old', updated_at: at(9) }), memo)
    expect(same).toBe(lanes)
    lanes = applyWrite(lanes, card('t2', 'todo', 2, { updated_at: at(20), deleted_at: at(20) }), memo)
    expect(ids(by(lanes, 'todo'))).toEqual(['t1', 't3'])
    // A late event of before the deletion does not bring it back; a restore does.
    expect(applyWrite(lanes, card('t2', 'todo', 2, { updated_at: at(15) }), memo)).toBe(lanes)
    lanes = applyWrite(lanes, card('t2', 'todo', 4, { updated_at: at(30) }), memo, { entered: true })
    expect(ids(by(lanes, 'todo'))).toEqual(['t1', 't3', 't2'])
  })

  it('a read drops what this app saw deleted after it, and replays the writes that came while it was out', () => {
    const memo = newMemo()
    memo.tombstones.set('t2', Date.parse(at(30)))
    const read: BoardRead = boardOf({
      columns: [
        { id: 'todo', label: 'To do', tickets: [card('t1', 'todo', 1), card('t2', 'todo', 2)], total: 2 },
        { id: 'done', label: 'Done', tickets: [], total: 0 },
      ],
      updated_at: at(40),
    })
    const lanes = applyRead(
      read,
      [{ summary: card('t1', 'done', 1, { updated_at: at(45) }), kind: { moved: true, from: 'todo' } }],
      memo,
    )
    expect(ids(by(lanes, 'todo'))).toEqual([])
    expect(ids(by(lanes, 'done'))).toEqual(['t1'])
    expect([by(lanes, 'todo')?.total, by(lanes, 'done')?.total]).toEqual([0, 1])
  })

  it('a page appends the rows after the ones held, skipping repeats and older versions', () => {
    const memo = newMemo()
    memo.versions.set('t7', Date.parse(at(99)))
    const lanes = base()
    const next = appendPage(
      lanes,
      'done',
      [card('t5', 'done', 2), card('t6', 'done', 3), card('t7', 'done', 4)],
      40,
      memo,
      false,
    )
    expect(ids(by(next, 'done'))).toEqual(['t4', 't5', 't6'])
    expect(by(next, 'done')?.total).toBe(40)
    // A write touched the lane meanwhile: its count is the newer one.
    const touched = appendPage(lanes, 'done', [card('t6', 'done', 3)], 12, newMemo(), true)
    expect(by(touched, 'done')?.total).toBe(40)
  })

  it('maps a drop among the visible cards to the column index the worker wants (console UI fullIndex)', () => {
    const column = ['a', 'b', 'c', 'd', 'e'].map((id, i) => card(id, 'todo', i + 1))
    // Filters show b and d; X dropped between them goes before d: index 3.
    expect(dropIndex(column, ['b', 'd'], 1, 'x')).toBe(3)
    expect(dropIndex(column, ['b', 'd'], 2, 'x')).toBe(4)
    expect(dropIndex(column, ['b', 'd'], 0, 'x')).toBe(1)
    expect(dropIndex(column, [], 0, 'x')).toBe(5)
    // A reorder: the dragged card is out of both.
    expect(dropIndex(column, ['a', 'b', 'c', 'd', 'e'], 0, 'c')).toBe(0)
    expect(dropIndex(column, ['b', 'c', 'd'], 2, 'c')).toBe(3)
  })

  it('the optimistic move places the card and renumbers; the end of a partial lane only counts', () => {
    let next = placeMove(base(), 't3', 'todo', 0)
    expect(ids(by(next, 'todo'))).toEqual(['t3', 't1', 't2'])
    next = placeMove(base(), 't1', 'done')
    expect([ids(by(next, 'done')), by(next, 'done')?.total]).toEqual([['t4', 't5'], 41])
    next = placeMove(base(), 't4', 'todo')
    expect(ids(by(next, 'todo'))).toEqual(['t1', 't2', 't3', 't4'])
    expect(placeMove(base(), 'nope', 'todo', 0)).toEqual(base())
  })
})

describe('reading worker data entry by entry', () => {
  it('skips bad cards and keeps the good ones', () => {
    const read = boardOf({
      columns: [
        {
          id: 'todo',
          label: 7,
          tickets: [
            null,
            'x',
            { id: 'a', key: 'KAN-1', status: 'todo', labels: 'no', comment_count: '3' },
            { key: 'KAN-2' },
            { id: 'b', key: 'KAN-3', status: 'done' },
          ],
        },
        { id: '' },
        'junk',
      ],
      priorities: 'high',
    })
    expect(read.columns.map((c) => [c.id, c.label, c.rows.map((r) => r.key), c.total])).toEqual([
      ['todo', 'todo', ['KAN-1'], 1],
    ])
    expect(read.columns[0].rows[0]).toMatchObject({ labels: [], comment_count: 0, title: '', pills: [] })
    expect(read.priorities).toEqual([])
    expect(boardOf(null).columns).toEqual([])
  })

  it('reads a ticket and an event defensively', () => {
    expect(ticketOf({ id: 'a' })).toBeNull()
    const t = ticketOf({
      id: 'a',
      key: 'KAN-1',
      status: 'todo',
      comments: [{ id: 'c1', body: 'hi' }, { body: 'no id' }],
      activity: [
        { id: 'x', type: 'ticket.created', changes: [{ field: 'status', to: 'todo' }, 'bad'] },
        { type: 'no id' },
      ],
      children: [{ id: 'k', key: 'KAN-2' }, {}],
    }) as Ticket
    expect([t.comments.length, t.activity.length, t.activity[0].changes.length, t.children.length]).toEqual([
      1, 1, 1, 1,
    ])
    expect(t.comments[0].author).toBe('user')
    expect(changeOf({ event: 'config.updated' })).toMatchObject({ event: 'config.updated', ticket: null })
    expect(changeOf({ event: 'ticket.moved', ticket: { id: 'a' } })).toBeNull()
    expect(
      changeOf({
        event: 'ticket.moved',
        at: at(5),
        ticket: { id: 'a', key: 'KAN-1', status: 'done' },
        activity: { at: at(5), changes: [{ field: 'status', from: 'todo', to: 'done' }] },
      })?.from,
    ).toBe('todo')
    // An older activity entry is not this write's.
    expect(
      changeOf({
        event: 'ticket.moved',
        at: at(6),
        ticket: { id: 'a', key: 'KAN-1', status: 'done' },
        activity: { at: at(5), changes: [{ field: 'status', from: 'todo', to: 'done' }] },
      })?.from,
    ).toBeNull()
  })
})

describe('filters', () => {
  const ctx = { lastVisit: at(10), now: Date.parse(at(100)), own: new Map([['own', at(50)]]) }
  const rows = [
    card('t1', 'todo', 1, { title: 'Acentuação fix', labels: ['a', 'b'], assignee: 'x' }),
    card('t2', 'todo', 20, { key: 'KAN-20', labels: ['b'], project_id: 'p' }),
    card('own', 'todo', 50, { key: 'KAN-50' }),
  ]
  it('matches text without a regex, keys by number, accents folded', () => {
    const q = (text: string) => rows.filter((r) => matches(r, { ...EMPTY_FILTERS, q: text }, ctx)).map((r) => r.id)
    expect(q('acentuacao')).toEqual(['t1'])
    expect(q('#2')).toEqual(['t2'])
    expect(q('(.*)+$')).toEqual([])
    expect(q('KAN-5')).toEqual(['own'])
  })
  it('counts facets and keeps own writes out of "since your last visit"', () => {
    expect([...facetCounts(rows, { ...EMPTY_FILTERS, labels: ['a'] }, ctx, 'labels')]).toEqual([
      ['a', 1],
      ['b', 2],
    ])
    expect(rows.map((r) => isChangedSinceVisit(r, ctx))).toEqual([false, true, false])
    expect(facetCounts(rows, EMPTY_FILTERS, ctx, 'projects').get('')).toBe(2)
  })
  it('reads stored filters defensively', () => {
    expect(coerceFilters('x')).toEqual(EMPTY_FILTERS)
    expect(coerceFilters({ q: 3, labels: ['a', 2], updated: 'decade', labelMode: 'all' })).toEqual({
      ...EMPTY_FILTERS,
      labels: ['a'],
      labelMode: 'all',
    })
  })
})

describe('the activity', () => {
  const comment = (id: string, parent: string | null, author: string, s: number): Comment => ({
    id,
    parent_id: parent,
    body: id,
    author,
    created_at: at(s),
  })
  const entry = (id: string, seq: number, type: string, fields: Partial<Activity> = {}): Activity => ({
    id,
    seq,
    type,
    actor: 'tech-lead',
    at: at(seq),
    changes: [],
    comment_id: null,
    ...fields,
  })
  const ticket = {
    ...card('t1', 'todo', 1),
    description: '',
    created_by: 'user',
    parent: null,
    children: [],
    progress: { done: 0, total: 0 },
    activity_total: 9,
    comments: [comment('c1', null, 'tech-lead', 2), comment('c2', null, 'user', 3), comment('r1', 'c1', 'user', 4)],
    activity: [
      entry('e1', 1, 'ticket.created', { changes: [{ field: 'status', from: null, to: 'todo' }] }),
      entry('e2', 2, 'comment.created', { comment_id: 'c1' }),
      entry('e3', 3, 'comment.created', { comment_id: 'c2' }),
      entry('e4', 4, 'comment.created', { comment_id: 'r1' }),
      entry('e5', 5, 'subtask.added', { changes: [{ field: 'subtask', from: null, to: 'KAN-2' }] }),
      entry('e6', 6, 'subtask.added', { changes: [{ field: 'subtask', from: null, to: 'KAN-3' }] }),
      entry('e7', 7, 'ticket.updated', {
        changes: [
          { field: 'labels', from: ['a'], to: ['a', 'b'] },
          { field: 'assignee', from: null, to: 'tech-lead' },
        ],
      }),
    ],
  } as Ticket
  const who = { name: (id: string) => (id === 'user' ? 'You' : 'Tech Lead'), hue: () => 'none' }
  const columns = [{ id: 'todo', label: 'To do' }]
  it('threads replies under their root and folds subtask additions', () => {
    const rows = timeline(ticket, 'all', columns, who, Date.parse(at(100)), true)
    expect(rows.map((r) => [r.kind, r.text])).toEqual([
      [0, 'Tech Lead created this in To do'],
      [1, 'c1'],
      [2, 'r1'],
      [1, 'c2'],
      [0, 'Tech Lead added 2 subtasks KAN-2 … KAN-3'],
      [0, 'Tech Lead added the label b, assigned this to Tech Lead'],
    ])
    expect(rows[2]).toMatchObject({ root: 'c1', comment: 'r1', who: 'You', icon: 'user' })
    expect(timeline(ticket, 'comments', columns, who, 0, true).map((r) => r.key)).toEqual(['c:c1', 'c:r1', 'c:c2'])
    expect(timeline(ticket, 'changes', columns, who, 0, true).every((r) => r.kind === 0)).toBe(true)
  })
})

describe('cards, steps and words', () => {
  it('a card computes what it shows', () => {
    const row = cardRow(
      card('t1', 'done', 1, {
        labels: ['x', 'y', 'z', 'w', 'x'],
        pills: [
          { icon: 'calendar', text: 'Fri' },
          { icon: 'unknown-glyph', text: 'P' },
          { icon: 'tag', text: 'third' },
        ],
        priority: 'urgent',
        assignee: 'a1',
        parent_id: 'p',
        comment_count: 2,
        updated_at: at(50),
      }),
      {
        match: { lastVisit: at(10), now: Date.parse(at(50)) + 2 * 3_600_000 },
        agents: new Map([['a1', { id: 'a1', name: 'A', color: 'teal' }]]),
        keys: new Map([['p', 'KAN-9']]),
        subs: new Map([['t1', { done: 1, total: 3 }]]),
        highlight: ['w'],
        priorities: ['low', 'medium', 'high', 'urgent'],
        done: 'done',
        selected: 'KAN-1',
      },
    )
    expect(row).toMatchObject({
      parent: 'Subtask of KAN-9',
      l0: 'w',
      sel0: true,
      more: '+1',
      p1Icon: 'tag',
      hasP1: true,
      priority: 'Urgent',
      priorityVariant: 'alert',
      changed: true,
      changedAt: '2h',
      subs: '1/3',
      subsWarn: true,
      assigneeHue: 'teal',
      selected: true,
    })
  })
  it('names steps, priorities, counts and times like the console', () => {
    const cols = ['backlog', 'todo', 'doing', 'review', 'done', 'x'].map((id) => ({ id, label: id }))
    expect(cols.map((c) => statusStep(cols, c.id, 'done', 'x'))).toEqual([
      'start',
      'ready',
      'progress',
      'review',
      'done',
      'canceled',
    ])
    expect(
      ['low', 'medium', 'high', 'urgent'].map((p) => priorityRank(p, ['low', 'medium', 'high', 'urgent'])),
    ).toEqual([-1, -1, 1, 0])
    expect([count(999), count(1204), count(12_400)]).toEqual(['999', '1.2k', '12k'])
    const now = Date.parse('2026-10-23T12:00:00Z')
    expect([
      ago('2026-10-23T11:59:30Z', now),
      ago('2026-10-23T09:00:00Z', now),
      ago('2026-10-01T00:00:00Z', now),
      ago('x', now),
    ]).toEqual(['now', '3h', 'Oct 1', ''])
  })
  it('the Done warning, subtask defaults and the pick rules follow the console', () => {
    const cols = [
      { id: 'review', label: 'In review' },
      { id: 'done', label: 'Done' },
      { id: 'x', label: 'Canceled' },
    ]
    const children = [
      { id: 'a', key: 'KAN-2', title: '', status: 'review', deleted_at: null, project_id: null, release_id: null },
      { id: 'b', key: 'KAN-3', title: '', status: 'x', deleted_at: null, project_id: null, release_id: null },
    ]
    expect(doneWarning({ status: 'done', children, deleted_at: null }, 'done', 'x', cols)).toEqual({
      open: [children[0]],
      back: cols[0],
    })
    expect(doneWarning({ status: 'review', children, deleted_at: null }, 'done', 'x', cols)).toBeNull()
    const projects = [{ id: 'p', name: 'P', color: null, lead: null, archived: true, total: 0, done: 0 }]
    const releases = [{ id: 'r', name: 'R', target_at: null, released: false, status: 'planned', total: 0, done: 0 }]
    expect(subtaskDefaults({ key: 'KAN-1', project_id: 'p', release_id: 'r' }, projects, releases, 'backlog')).toEqual({
      parent: 'KAN-1',
      status: 'backlog',
      release: 'r',
    })
    const keys = new Map([['p1', 'KAN-1']])
    const parents = new Set(['p1'])
    expect(pickRefusal(card('p1', 'todo', 1), 'me', 'attach', parents, keys)).toBe('has its own subtasks')
    expect(pickRefusal(card('c', 'todo', 1, { parent_id: 'p1' }), 'me', 'parent', parents, keys)).toBe(
      'a subtask of KAN-1',
    )
    expect(pickRefusal(card('me', 'todo', 1), 'me', 'parent', parents, keys)).toBe('this ticket')
    expect(pickRefusal(card('ok', 'todo', 1), 'me', 'parent', parents, keys)).toBeNull()
  })
})
