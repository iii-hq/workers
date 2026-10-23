/**
 * The ticket screen (design §6): opening it on a phone and on a wide screen,
 * the head, the activity threaded, comments and replies, the editors and the
 * pickers, delete and restore, the Done warning, and the links in Markdown.
 */
import { decodeMessage } from '@iii-dev/native-ui/testing'
import { describe, expect, it } from 'vitest'
import { callsOf, cards, onTop, sample, start, topCard } from './fake'

/** Opens KAN-4 (In review) from the board and lets its reads land. */
async function openReview(env: Record<string, unknown> = {}) {
  const started = await start({ env })
  const { host } = started
  await host.select('lanes', 0)
  await host.itemPress('lane:in_review', 't4')
  await host.settle()
  return started
}

describe('opening a ticket', () => {
  it('on a phone: the detail card and the card’s lane come in one batch', async () => {
    const { host } = await start()
    await host.select('lanes', 0)
    const from = host.messages.length
    await host.itemPress('lane:in_review', 't4')
    // The first message after the press carries both: showDetail and the lane (D6).
    const first = decodeMessage(host.messages[from], 'fromVm') as {
      op: string
      id?: number
      prop?: unknown
      value?: unknown
    }[]
    const set = (id: number, prop: number) => first.find((e) => e.op === 'set' && e.id === id && e.prop === prop)?.value
    // Lanes.selected is tag 18, ListDetail.showDetail tag 17 (catalog.json).
    expect(set(host.find('lanes').id, 18)).toBe(3)
    expect(set(host.find('board-split').id, 17)).toBe(true)
    expect(host.prop('board-split', 'showDetail')).toBe(true)
    expect(host.lanes('lanes').selected).toBe(3)
    await host.settle()
    expect(callsOf(host, 'ticket::get')).toEqual([{ id: 'KAN-4' }])
    expect(callsOf(host, 'ticket::links')).toEqual([{ id: 'KAN-4' }])
    expect(host.find('ticket').props).toMatchObject({ title: 'KAN-4', subtitle: 'In review' })
    // The open card shows selected.
    expect(cards(host, 'in_review')[0]).toMatchObject({ selected: true })
  })

  it('on a wide screen: the same, and Back (dismiss) closes it', async () => {
    const { host } = await openReview({ windowClass: 'wide' })
    expect(host.prop('board-split', 'showDetail')).toBe(true)
    expect(host.prop('board-split', 'listFill')).toBe(true)
    expect(host.lanes('lanes').selected).toBe(3)
    await host.emit('board-split', 'dismiss')
    expect(host.prop('board-split', 'showDetail')).not.toBe(true)
    expect(cards(host, 'in_review')[0]).toMatchObject({ selected: false })
  })
})

describe('the head', () => {
  it('shows the title, the chips with the agent and project colors, labels, pills and the description in Markdown', async () => {
    const { host } = await openReview()
    expect(host.prop('title', 'text')).toBe('Search index')
    expect(host.find('status').props).toMatchObject({ text: 'In review', icon: 'chart-pie', tone: 'warning' })
    expect(host.find('priority').props).toMatchObject({ text: 'Urgent', tone: 'danger' })
    expect(host.find('assignee').props).toMatchObject({ text: 'Backend Engineer', icon: 'bot', hue: 'teal' })
    expect(host.find('project').props).toMatchObject({ text: 'No project' })
    expect(host.findAll({ type: 'Chip', text: 'Prod', icon: 'tag' })).toHaveLength(1)
    // KAN-1 is linked (it mentions KAN-4 and KAN-4 mentions it): its key becomes a link.
    expect(host.prop('description', 'text')).toBe(
      '## Done\n\n- Built it\n- Tested it, see [the run](https://ci.example.com/run/7)\n\nPart of [KAN-1](#kanban-ticket/KAN-1 "Search v2 · In progress").',
    )
    expect(host.findAll({ type: 'ListRow', subtitle: 'In progress · both ways' })).toHaveLength(1)
  })

  it('threads the activity: a comment, its reply under it, then the filters', async () => {
    const { host } = await openReview()
    const items = host.items('activity') as Record<string, unknown>[]
    expect(items.map((i) => [i.kind, i.who, i.text])).toEqual([
      [0, 'You', 'You created this in In review'],
      [1, 'Backend Engineer', 'Pushed the fix; ready for review.'],
      [2, 'You', 'Looks good, **merging**.'],
    ])
    expect(host.find('activity-filter').props).toMatchObject({ options: ['All', 'Comments 2', 'Changes 1'] })
    await host.select('activity-filter', 2)
    expect((host.items('activity') as { kind: number }[]).map((i) => i.kind)).toEqual([0])
    await host.select('activity-filter', 1)
    expect((host.items('activity') as { kind: number }[]).map((i) => i.kind)).toEqual([1, 2])
  })
})

describe('comments', () => {
  it('sends a comment as the user, and the field keeps only what was typed after', async () => {
    const { host, fake } = await openReview()
    await host.type('composer', 'Merged, thanks!')
    await host.press('send')
    await host.settle()
    expect(callsOf(host, 'comment::create')).toEqual([
      { ticket_id: 't4', body: 'Merged, thanks!', parent_id: null, author: 'user' },
    ])
    expect(fake.get('KAN-4')?.comments).toHaveLength(3)
    expect(host.prop('composer', 'value')).toBe('')
    const last = (host.items('activity') as Record<string, unknown>[]).at(-1)
    expect(last).toMatchObject({ kind: 1, who: 'You', text: 'Merged, thanks!' })
    expect(cards(host, 'in_review')[0]).toMatchObject({ comments: '3' })
  })

  it('replies to a comment: the composer says to whom, the reply lands under its root', async () => {
    const { host } = await openReview()
    // Reply on the reply answers its root (replies are one level deep).
    await host.itemPress('activity', 'c:c2', { type: 'Button', text: 'Reply' })
    expect(host.find({ type: 'Text', text: 'Replying to You' })).toBeTruthy()
    expect(host.focusRequests.length).toBeGreaterThan(0)
    await host.type('composer', 'One more thing')
    await host.press('send')
    await host.settle()
    expect(callsOf(host, 'comment::create').at(-1)).toMatchObject({ parent_id: 'c1', body: 'One more thing' })
    expect(host.findAll('replying')).toHaveLength(0)
    const items = host.items('activity') as Record<string, unknown>[]
    expect(items.map((i) => i.kind)).toEqual([0, 1, 2, 2])
  })

  it('a failed comment keeps the text and says why', async () => {
    const { host, fake } = await openReview()
    fake.overrides['comment::create'] = () => {
      throw new Error('ticket not found: t4')
    }
    await host.type('composer', 'Lost?')
    await host.press('send')
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({ text: "Couldn't send the comment: ticket not found: t4", kind: 'error' })
    expect(host.prop('composer', 'value')).toBe('Lost?')
    expect(host.findAll('send')).toHaveLength(1)
  })
})

describe('editing', () => {
  it('edits the description in its own page, and the Markdown shows the new text', async () => {
    const { host } = await openReview()
    await host.press('edit-description')
    expect(topCard(host)).toMatchObject({ name: 'text', params: { title: 'Edit description', multiline: true } })
    expect(host.prop(onTop(host, 'text-field'), 'mono')).toBe(true)
    await host.type(onTop(host, 'text-field'), 'New body, see KAN-1.')
    await host.press(onTop(host, 'text-save'))
    await host.settle()
    expect(callsOf(host, 'ticket::update').at(-1)).toEqual({
      id: 't4',
      description: 'New body, see KAN-1.',
      actor: 'user',
    })
    expect(host.prop('description', 'text')).toContain('New body, see [KAN-1](#kanban-ticket/KAN-1')
  })

  it('edits the title by tapping it; an unchanged or blank title writes nothing', async () => {
    const { host } = await openReview()
    await host.press('title')
    await host.press(onTop(host, 'text-save'))
    await host.settle()
    expect(callsOf(host, 'ticket::update')).toEqual([])
    await host.press('title')
    await host.type(onTop(host, 'text-field'), 'Search index v2')
    await host.submit(onTop(host, 'text-field'))
    await host.settle()
    expect(callsOf(host, 'ticket::update')).toEqual([{ id: 't4', title: 'Search index v2', actor: 'user' }])
    expect(host.prop('title', 'text')).toBe('Search index v2')
    expect(cards(host, 'in_review')[0]).toMatchObject({ title: 'Search index v2' })
  })

  it('moves by the status chip, and sets the priority by its menu', async () => {
    const { host, answers } = await openReview()
    answers.menus.push('Done')
    await host.press('status')
    await host.settle()
    expect(host.menus.at(-1)).toMatchObject({ title: 'Move KAN-4 to' })
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't4', status: 'done', actor: 'user' }])
    expect(host.find('ticket').props).toMatchObject({ subtitle: 'Done' })
    expect(cards(host, 'done').map((c) => c.ticketKey)).toEqual(['KAN-3', 'KAN-4'])
    answers.menus.push('Low')
    await host.press('priority')
    await host.settle()
    expect(callsOf(host, 'ticket::update')).toEqual([{ id: 't4', priority: 'low', actor: 'user' }])
  })

  it('assigns through the agent picker, and files under a project and a release', async () => {
    const { host } = await openReview()
    await host.press('assignee')
    expect(topCard(host)).toMatchObject({ name: 'pick', params: { kind: 'agent', current: 'backend-engineer' } })
    expect(
      (host.items(onTop(host, 'pick-list')) as { key: string; selected: boolean }[]).map((r) => [r.key, r.selected]),
    ).toEqual([
      ['none', false],
      ['tech-lead', false],
      ['backend-engineer', true],
      ['frontend-engineer', false],
    ])
    await host.itemPress(onTop(host, 'pick-list'), 'tech-lead')
    await host.settle()
    expect(callsOf(host, 'ticket::update').at(-1)).toEqual({ id: 't4', assignee: 'tech-lead', actor: 'user' })
    expect(host.find('assignee').props).toMatchObject({ text: 'Tech Lead', hue: 'purple' })

    await host.press('project')
    // The archived project is not offered.
    expect((host.items(onTop(host, 'pick-list')) as { title: string }[]).map((r) => r.title)).toEqual([
      'No project',
      'iii cloud',
    ])
    await host.itemPress(onTop(host, 'pick-list'), 'p1')
    await host.settle()
    expect(callsOf(host, 'ticket::update').at(-1)).toEqual({ id: 't4', project: 'p1', actor: 'user' })
    expect(host.find('project').props).toMatchObject({ text: 'iii cloud', hue: 'blue' })

    await host.press('release')
    const releases = host.items(onTop(host, 'pick-list')) as { title: string; kind: number }[]
    expect(releases.map((r) => r.title)).toEqual(['No release', 'Phase 2', 'Released', 'Phase 1'])
    await host.back()
    await host.settle()
    expect(callsOf(host, 'ticket::update')).toHaveLength(2)
  })

  it('adds a label (finding or creating it) and removes one from its chip', async () => {
    const { host, answers } = await openReview()
    await host.press('add-label')
    await host.settle()
    await host.type(onTop(host, 'pick-search'), 'Hotfix')
    await host.tick(300)
    expect((host.items(onTop(host, 'pick-list')) as { title: string }[]).map((r) => r.title)).toEqual([
      'Create label "Hotfix"',
    ])
    await host.itemPress(onTop(host, 'pick-list'), '+Hotfix')
    await host.settle()
    expect(callsOf(host, 'ticket::update').at(-1)).toEqual({ id: 't4', add_labels: ['Hotfix'], actor: 'user' })
    answers.menus.push('Remove label Hotfix')
    await host.press({ type: 'Chip', text: 'Hotfix' })
    await host.settle()
    expect(callsOf(host, 'ticket::update').at(-1)).toEqual({ id: 't4', remove_labels: ['Hotfix'], actor: 'user' })
  })
})

describe('delete and restore', () => {
  it('deletes only after the destructive confirmation; the board drops the card; restore brings it back', async () => {
    const { host, answers } = await openReview()
    answers.menus.push('Delete ticket…')
    answers.confirms.push(false)
    await host.press('ticket-menu')
    await host.settle()
    expect(host.dialogs.at(-1)).toMatchObject({ title: 'Delete this ticket?', confirm: 'Delete', destructive: true })
    expect(callsOf(host, 'ticket::delete')).toEqual([])

    answers.menus.push('Delete ticket…')
    answers.confirms.push(true)
    await host.press('ticket-menu')
    await host.settle()
    expect(callsOf(host, 'ticket::delete')).toEqual([{ id: 't4', actor: 'user' }])
    expect(host.find('deleted').props).toMatchObject({ headline: 'This ticket was deleted' })
    expect(host.findAll('composer')).toHaveLength(0)
    expect(host.prop('status', 'enabled')).toBe(false)
    expect(cards(host, 'in_review')).toEqual([])
    expect(host.lanes('lanes').counts[3]).toBe('0')

    answers.confirms.push(true)
    await host.press('restore')
    await host.settle()
    expect(host.dialogs.at(-1)).toMatchObject({ title: 'Restore this ticket?', confirm: 'Restore' })
    expect(callsOf(host, 'ticket::restore')).toEqual([{ id: 't4', actor: 'user' }])
    expect(host.findAll('deleted')).toHaveLength(0)
    expect(cards(host, 'in_review').map((c) => c.ticketKey)).toEqual(['KAN-4'])
  })
})

describe('the Done warning', () => {
  it('a parent in Done with open subtasks offers the three ways out of the console', async () => {
    const fake = sample()
    const parent = fake.get('KAN-1')
    if (parent) parent.status = 'done'
    const { host, answers } = await start({ fake })
    await host.itemPress('lane:done', 't1')
    await host.settle()
    expect(host.find('done-warning').props).toMatchObject({
      headline: 'Done with 1 open subtask',
      detail:
        'KAN-1 is in Done while 1 of its 2 subtasks are not. Progress shows 1 of 2 because only the Done column counts.',
    })
    expect(host.prop('move-back', 'text')).toBe('Move back to In review')
    answers.confirms.push(true)
    await host.press('detach-subtasks')
    await host.settle()
    expect(host.dialogs.at(-1)).toMatchObject({ title: 'Detach 1 subtask?' })
    expect(callsOf(host, 'ticket::update')).toEqual([{ id: 't2', parent: null, actor: 'user' }])
    expect(host.findAll('done-warning')).toHaveLength(0)
  })

  it('Move back takes the parent back a column', async () => {
    const fake = sample()
    const parent = fake.get('KAN-1')
    if (parent) parent.status = 'done'
    const { host } = await start({ fake })
    await host.itemPress('lane:done', 't1')
    await host.settle()
    await host.press('move-back')
    await host.settle()
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't1', status: 'in_review', actor: 'user' }])
    expect(host.findAll('done-warning')).toHaveLength(0)
  })
})

describe('links in Markdown', () => {
  it('a ticket key opens that ticket as a page; a web link asks to open it; anything else is refused', async () => {
    const { host } = await openReview()
    await host.link('description', '#kanban-ticket/KAN-1')
    await host.settle()
    expect(topCard(host)).toMatchObject({ name: 'ticket', params: { key: 'KAN-1' } })
    expect(host.prop(onTop(host, 'ticket'), 'title')).toBe('KAN-1')
    // The pushed page has the subtasks of KAN-1; one opens on top.
    await host.press(onTop(host, { type: 'ListRow', title: 'KAN-2  Persist filters in the URL' }))
    await host.settle()
    expect(topCard(host)).toMatchObject({ name: 'ticket', params: { key: 'KAN-2' } })
    expect(host.prop(onTop(host, 'parent'), 'visible')).not.toBe(false)
    await host.back()
    await host.back()
    await host.settle()
    await host.link('description', 'https://ci.example.com/run/7')
    expect(host.opened).toEqual(['https://ci.example.com/run/7'])
  })

  it('a link in a comment works the same', async () => {
    const fake = sample()
    const review = fake.get('KAN-4')
    if (review) review.comments[0].body = 'See [the trace](https://ci.example.com/t/1) and [this](javascript:alert(1)).'
    const { host } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    await host.itemEmit('activity', 'c:c1', { type: 'Markdown' }, 'link', ['https://ci.example.com/t/1'])
    expect(host.opened).toEqual(['https://ci.example.com/t/1'])
    await host.itemEmit('activity', 'c:c1', { type: 'Markdown' }, 'link', ['javascript:alert(1)'])
    expect(host.opened).toHaveLength(1)
    expect(host.toasts.at(-1)).toEqual({ text: "This link can't be opened here", kind: 'info' })
  })
})
