/** The board (design §3–§5, §8.2, §9): lanes, cards, paging, the lane on screen, refresh, the empty and error states. */
import { describe, expect, it } from 'vitest'
import { callsOf, cards, FakeKanban, HOUR, HUE, keysOf, sample, shown, start, T, VARIANT } from './fake'

describe('the board', () => {
  it('binds kanban:change, then reads the board with a first page per column', async () => {
    const { host } = await start()
    const subs = [...host.subscriptions.values()]
    expect(subs).toEqual([{ trigger: 'kanban:change', config: { ticket: 'summary' } }])
    expect(callsOf(host, 'board::get')).toEqual([{ limit: 50 }])
    expect(host.tree()?.props).toMatchObject({ title: 'Kanban', subtitle: '5 tickets' })
    const lanes = host.lanes('lanes')
    expect(lanes.titles).toEqual(['Backlog', 'To do', 'In progress', 'In review', 'Done', 'Canceled'])
    expect(lanes.counts).toEqual(['1', '1', '1', '1', '1', '0'])
    expect(host.find('board-split').props).toMatchObject({ listFill: true })
    expect(host.prop('board-split', 'showDetail')).not.toBe(true)
    expect(host.find('lane:todo').props).toMatchObject({ dragGroup: 'board' })
  })

  it('draws a card with what the console card shows, computed in the script', async () => {
    const { host } = await start()
    expect(cards(host, 'in_progress')[0]).toMatchObject({
      ticketKey: 'KAN-1',
      title: 'Search v2',
      hasParent: false,
      l0: 'search',
      l1: 'frontend',
      l2: 'backend',
      more: '+1',
      hasMore: true,
      priority: 'High',
      priorityVariant: VARIANT.warn,
      subs: '1/2',
      hasSubs: true,
      subsWarn: false,
      assigneeIcon: 'bot',
      assigneeHue: HUE.purple,
    })
    // A subtask names its parent; an agent whose color is not a hue has none.
    expect(cards(host, 'todo')[0]).toMatchObject({ parent: 'Subtask of KAN-1', hasParent: true, assigneeHue: HUE.none })
    // Pills: an icon the app does not know becomes a tag; comments counted; urgent is the alert badge.
    expect(cards(host, 'in_review')[0]).toMatchObject({
      p0: 'Fri 17:00',
      p0Icon: 'calendar',
      p1Icon: 'tag',
      comments: '2',
      hasComments: true,
      priorityVariant: VARIANT.alert,
    })
    // Low priority shows no badge; nobody assigned is the dashed circle.
    expect(cards(host, 'backlog')[0]).toMatchObject({ hasPriority: false, assigneeIcon: 'circle-dashed' })
    // The template binds the parts: one Card with the hue on the assignee icon.
    const icons = host.findAll({ type: 'Icon' }, { templates: true }).map((n) => n.props?.hue)
    expect(icons).toContainEqual({ $item: 'assigneeHue' })
  })

  it('pages a long column: the first rows, Load more and the end of the list read the rest', async () => {
    const fake = sample()
    for (let i = 0; i < 120; i++) fake.add({ title: `Done ${i}`, status: 'done' })
    const { host } = await start({ fake })
    expect(host.lanes('lanes').counts[4]).toBe('121')
    expect(cards(host, 'done')).toHaveLength(50)
    expect(host.find('more:done').props).toMatchObject({ text: 'Load more' })
    expect(host.findAll({ type: 'Text', text: 'Showing 50 of 121' })).toHaveLength(1)
    await host.emit('lane:done', 'endReached')
    await host.settle()
    expect(callsOf(host, 'ticket::list')).toEqual([{ status: 'done', offset: 50, limit: 100 }])
    expect(cards(host, 'done')).toHaveLength(121)
    // KAN-3 is the first done ticket; the page boundary falls between KAN-54 and KAN-55, nothing lost or repeated.
    expect(keysOf(host, 'done').slice(48, 52)).toEqual(['KAN-53', 'KAN-54', 'KAN-55', 'KAN-56'])
    expect(new Set(keysOf(host, 'done')).size).toBe(121)
    expect(host.findAll('more:done')).toHaveLength(0)
    // A whole column asks for nothing more.
    await host.emit('lane:done', 'endReached')
    await host.settle()
    expect(callsOf(host, 'ticket::list')).toHaveLength(1)
  })

  it('a failed page says why and keeps the rows', async () => {
    const fake = sample()
    for (let i = 0; i < 60; i++) fake.add({ title: `Done ${i}`, status: 'done' })
    fake.overrides['ticket::list'] = () => {
      throw new Error('board file is locked')
    }
    const { host } = await start({ fake })
    await host.press('more:done')
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({ text: "Couldn't read more of Done: board file is locked", kind: 'error' })
    expect(cards(host, 'done')).toHaveLength(50)
    expect(host.prop('more:done', 'loading')).not.toBe(true)
  })

  it('keeps the lane the person picks, and opens the next session on it', async () => {
    const { host } = await start()
    await host.select('lanes', 3)
    expect(host.lanes('lanes').selected).toBe(3)
    expect(host.storage.get('lane')).toBe('in_review')
    const again = await start({ storage: { lane: 'done' } })
    expect(again.host.lanes('lanes').selected).toBe(4)
  })

  it('pull to refresh reads the board again', async () => {
    const { host, fake } = await start()
    fake.add({ title: 'Made elsewhere', status: 'todo' })
    await host.emit('lane:todo', 'refresh')
    await host.settle()
    expect(keysOf(host, 'todo')).toEqual(['KAN-2', 'KAN-6'])
    expect(host.prop('lane:todo', 'refreshing')).not.toBe(true)
  })

  it('reads again when the app comes back on screen and when the engine comes back', async () => {
    const { host, fake } = await start()
    await host.env({ visible: false })
    expect(host.subscriptions.size).toBe(0)
    // The visit is stamped when the app leaves the screen.
    expect(typeof host.storage.get('visit:KAN')).toBe('string')
    fake.add({ title: 'While away', status: 'backlog' })
    await host.env({ visible: true })
    await host.settle()
    expect(host.subscriptions.size).toBe(1)
    expect(keysOf(host, 'backlog')).toEqual(['KAN-5', 'KAN-6'])
    const reads = callsOf(host, 'board::get').length
    await host.env({ online: false })
    await host.env({ online: true })
    await host.settle()
    expect(callsOf(host, 'board::get').length).toBe(reads + 1)
  })

  it('an empty lane offers a new ticket there; a board with no columns says so', async () => {
    const { host } = await start()
    const empty = host.findAll({ type: 'EmptyState', title: 'No tickets in Canceled' })
    expect(empty).toHaveLength(1)
    const fake = new FakeKanban()
    fake.columns = []
    const bare = await start({ fake })
    expect(bare.host.find('no-columns').props).toMatchObject({ title: 'No columns configured' })
  })

  it('a board that cannot be read says why, in words, with Retry', async () => {
    const fake = sample()
    let fail = true
    fake.overrides['board::get'] = (p) => {
      if (fail) throw new Error('{"message":"board.json: permission denied"}')
      delete fake.overrides['board::get']
      return fake.answer('kanban::board::get', p)
    }
    const { host } = await start({ fake })
    expect(host.find('board-error').props).toMatchObject({
      headline: "Couldn't read the board",
      detail: 'board.json: permission denied',
    })
    fail = false
    await host.press('board-retry')
    await host.settle()
    expect(shown(host, 'board-error')).toBe(false)
    expect(keysOf(host, 'todo')).toEqual(['KAN-2'])
  })

  it('a worker that is not running is named', async () => {
    const fake = sample()
    fake.overrides['board::get'] = () => {
      throw Object.assign(new Error('nope'), { code: 'function_not_found' })
    }
    const { host } = await start({ fake })
    expect(host.find('board-error').props?.detail).toBe("The kanban worker isn't running on the engine")
  })

  it('marks what changed since the last visit, but not this app’s own writes', async () => {
    const fake = sample()
    const old = fake.add({ title: 'Old news', status: 'todo', updated_at: T(-48 * HOUR) })
    const { host } = await start({ fake, storage: { 'visit:KAN': T(-24 * HOUR) } })
    const row = (key: string) => cards(host, 'todo').find((c) => c.ticketKey === key)
    expect(row(old.key)).toMatchObject({ changed: false })
    expect(row('KAN-2')).toMatchObject({ changed: true, changedAt: 'now' })
    // An edit made here is not news.
    await host.itemLongPress('lane:todo', 't2')
    expect(host.menus.at(-1)).toMatchObject({ title: 'KAN-2' })
  })
})
