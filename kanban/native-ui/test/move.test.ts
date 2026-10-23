/**
 * Moving cards (design §7): drag and drop across lanes and inside one, the
 * visible index mapped to the worker's under filters, the long press menu,
 * the optimistic splice with its revert, one card's writes in order.
 */
import { describe, expect, it } from 'vitest'
import { callsOf, FakeKanban, keysOf, start } from './fake'

/** Five cards in To do (KAN-1 … KAN-5), labelled so a filter can show only some, and two in Doing. */
function board(): FakeKanban {
  const f = new FakeKanban()
  for (const [title, labels] of [
    ['A', []],
    ['B', ['ui']],
    ['C', []],
    ['D', ['ui']],
    ['E', []],
  ] as const)
    f.add({ title, status: 'todo', labels: [...labels] })
  f.add({ title: 'X', status: 'in_progress', labels: ['ui'] })
  f.add({ title: 'Y', status: 'in_progress' })
  return f
}

describe('drag and drop', () => {
  it('drops a card into another lane at the place shown, at once, then on the worker', async () => {
    const { host, fake } = await start({ fake: board(), live: false })
    fake.hold('ticket::move')
    await host.drop('lane:todo', 't6', 1, 'lane:in_progress')
    // On screen before the worker answers.
    expect(keysOf(host, 'todo')).toEqual(['KAN-1', 'KAN-6', 'KAN-2', 'KAN-3', 'KAN-4', 'KAN-5'])
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-7'])
    expect(host.lanes('lanes').counts.slice(1, 3)).toEqual(['6', '1'])
    fake.release('ticket::move')
    await host.settle()
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't6', status: 'todo', index: 1, actor: 'user' }])
    expect(fake.column('todo').map((t) => t.key)).toEqual(['KAN-1', 'KAN-6', 'KAN-2', 'KAN-3', 'KAN-4', 'KAN-5'])
    expect(keysOf(host, 'todo')).toEqual(['KAN-1', 'KAN-6', 'KAN-2', 'KAN-3', 'KAN-4', 'KAN-5'])
  })

  it('reorders inside a lane, and a drop where the card already is writes nothing', async () => {
    const { host } = await start({ fake: board(), live: false })
    await host.drop('lane:todo', 't5', 0)
    await host.settle()
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't5', status: 'todo', index: 0, actor: 'user' }])
    expect(keysOf(host, 'todo')).toEqual(['KAN-5', 'KAN-1', 'KAN-2', 'KAN-3', 'KAN-4'])
    await host.drop('lane:todo', 't2', 2)
    await host.settle()
    expect(callsOf(host, 'ticket::move')).toHaveLength(1)
  })

  it('with a filter on, maps the place among the visible cards to the place in the column', async () => {
    const { host } = await start({ fake: board(), live: false, storage: { filters: { labels: ['ui'] } } })
    expect(keysOf(host, 'todo')).toEqual(['KAN-2', 'KAN-4'])
    expect(host.lanes('lanes').counts.slice(1, 3)).toEqual(['2/5', '1/2'])
    // X between B and D: before D in the column (index 3), not at 1.
    await host.drop('lane:todo', 't6', 1, 'lane:in_progress')
    await host.settle()
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't6', status: 'todo', index: 3, actor: 'user' }])
    expect(keysOf(host, 'todo')).toEqual(['KAN-2', 'KAN-6', 'KAN-4'])
    // B after the last visible card: right after D in the column without B (A X C D E → index 4), not at the end.
    await host.drop('lane:todo', 't2', 2)
    await host.settle()
    expect(callsOf(host, 'ticket::move').at(-1)).toEqual({ id: 't2', status: 'todo', index: 4, actor: 'user' })
  })

  it('a failed move says why and puts the cards back where the worker has them', async () => {
    const { host, fake } = await start({ fake: board(), live: false })
    fake.overrides['ticket::move'] = () => {
      throw new Error('cannot move a deleted ticket')
    }
    await host.drop('lane:in_progress', 't1', 0, 'lane:todo')
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({ text: "Couldn't move KAN-1: cannot move a deleted ticket", kind: 'error' })
    expect(callsOf(host, 'board::get')).toHaveLength(2)
    expect(keysOf(host, 'todo')).toEqual(['KAN-1', 'KAN-2', 'KAN-3', 'KAN-4', 'KAN-5'])
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-6', 'KAN-7'])
  })

  it('when the board cannot be read either, the lanes go back to how they were before the drop', async () => {
    const { host, fake } = await start({ fake: board(), live: false })
    fake.overrides['ticket::move'] = () => {
      throw new Error('engine went away')
    }
    fake.overrides['board::get'] = () => {
      throw new Error('engine went away')
    }
    await host.drop('lane:in_progress', 't1', 0, 'lane:todo')
    await host.settle()
    expect(keysOf(host, 'todo')).toEqual(['KAN-1', 'KAN-2', 'KAN-3', 'KAN-4', 'KAN-5'])
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-6', 'KAN-7'])
    expect(host.lanes('lanes').counts.slice(1, 3)).toEqual(['5', '2'])
  })

  it('a hung move clears at the timeout and the board is read again', async () => {
    const { host, fake } = await start({ fake: board(), live: false })
    fake.hold('ticket::move')
    await host.drop('lane:in_progress', 't1', 0, 'lane:todo')
    await host.tick(30_000)
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({ text: "Couldn't move KAN-1: The engine didn't answer in time", kind: 'error' })
    expect(keysOf(host, 'todo')).toEqual(['KAN-1', 'KAN-2', 'KAN-3', 'KAN-4', 'KAN-5'])
  })

  it('two drops of one card go to the worker one after the other', async () => {
    const { host, fake } = await start({ fake: board(), live: false })
    fake.hold('ticket::move')
    await host.drop('lane:in_progress', 't1', 0, 'lane:todo')
    await host.drop('lane:done', 't1', 0, 'lane:in_progress')
    expect(fake.waiting('ticket::move')).toHaveLength(1)
    fake.release('ticket::move')
    await host.settle()
    expect(callsOf(host, 'ticket::move').map((p) => p.status)).toEqual(['in_progress', 'done'])
    expect(fake.get('KAN-1')?.status).toBe('done')
    expect(keysOf(host, 'done')).toEqual(['KAN-1'])
  })

  it('offline, a drop changes nothing and says why', async () => {
    const { host } = await start({ fake: board(), live: false })
    await host.env({ online: false })
    await host.drop('lane:in_progress', 't1', 0, 'lane:todo')
    await host.settle()
    expect(callsOf(host, 'ticket::move')).toEqual([])
    expect(host.toasts.at(-1)).toEqual({ text: 'The engine is not connected', kind: 'error' })
    expect(keysOf(host, 'todo')).toHaveLength(5)
  })
})

describe('the card menu', () => {
  it('Move to… sends the card to the end of the chosen column', async () => {
    const { host, answers } = await start({ fake: board(), live: false })
    answers.menus.push('Move to…', 'In progress')
    await host.itemLongPress('lane:todo', 't2')
    await host.settle()
    expect(host.menus[0]).toMatchObject({
      title: 'KAN-2',
      subtitle: 'B',
      actions: [
        { label: 'Open' },
        { label: 'Move to…' },
        { label: 'Assign…' },
        { label: 'Priority…' },
        { label: 'Copy key' },
        { label: 'Share' },
      ],
    })
    expect(host.menus[1]).toMatchObject({ title: 'Move KAN-2 to' })
    expect((host.menus[1] as { actions: { checked?: boolean }[] }).actions[1]).toMatchObject({ checked: true })
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't2', status: 'in_progress', actor: 'user' }])
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-6', 'KAN-7', 'KAN-2'])
  })

  it('Priority…, Copy key, Share and Open', async () => {
    const { host, answers } = await start({ fake: board(), live: false })
    answers.menus.push('Priority…', 'Urgent')
    await host.itemLongPress('lane:todo', 't3')
    await host.settle()
    expect(callsOf(host, 'ticket::update')).toEqual([{ id: 't3', priority: 'urgent', actor: 'user' }])
    answers.menus.push('Copy key')
    await host.itemLongPress('lane:todo', 't3')
    await host.settle()
    expect(host.clipboard).toEqual(['KAN-3'])
    answers.menus.push('Share')
    await host.itemLongPress('lane:todo', 't3')
    await host.settle()
    expect(host.shared).toEqual([{ text: 'KAN-3: C', title: 'KAN-3' }])
    answers.menus.push('Open')
    await host.itemLongPress('lane:todo', 't3')
    await host.settle()
    expect(host.prop('board-split', 'showDetail')).toBe(true)
  })

  it('Assign… goes through the agent picker', async () => {
    const { host, answers } = await start({ fake: board(), live: false })
    answers.menus.push('Assign…')
    await host.itemLongPress('lane:todo', 't1')
    await host.settle()
    await host.itemPress('pick-list', 'frontend-engineer')
    await host.settle()
    expect(callsOf(host, 'ticket::update')).toEqual([{ id: 't1', assignee: 'frontend-engineer', actor: 'user' }])
    expect(host.toasts.at(-1)).toEqual({ text: 'Assigned KAN-1 to Frontend Engineer', kind: 'info' })
  })
})
