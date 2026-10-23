/**
 * Live updates (design §8.4): the card of each `kanban:change` patches the
 * board (partial lanes too), the open ticket reads itself again once per
 * burst, `config.updated` reads the board again, and nothing is bound off
 * screen.
 */
import { describe, expect, it } from 'vitest'
import { callsOf, cards, keysOf, sample, start } from './fake'

describe('the board follows kanban:change', () => {
  it('creates, moves, updates, deletes and restores what others do', async () => {
    const { host, fake } = await start()
    const reads = callsOf(host, 'board::get').length
    await fake.elsewhere('ticket::create', { title: 'From an agent', status: 'todo' })
    expect(keysOf(host, 'todo')).toEqual(['KAN-2', 'KAN-6'])
    await fake.elsewhere('ticket::move', { id: 'KAN-6', status: 'in_review', index: 0 })
    expect(keysOf(host, 'todo')).toEqual(['KAN-2'])
    expect(keysOf(host, 'in_review')).toEqual(['KAN-6', 'KAN-4'])
    await fake.elsewhere('ticket::update', { id: 'KAN-6', title: 'Renamed by the agent' })
    expect(cards(host, 'in_review')[0]).toMatchObject({ title: 'Renamed by the agent' })
    await fake.elsewhere('ticket::delete', { id: 'KAN-6' })
    expect(keysOf(host, 'in_review')).toEqual(['KAN-4'])
    expect(host.lanes('lanes').counts[3]).toBe('1')
    await fake.elsewhere('ticket::restore', { id: 'KAN-6' })
    expect(keysOf(host, 'in_review')).toEqual(['KAN-6', 'KAN-4'])
    expect(host.lanes('lanes').counts.slice(1, 4)).toEqual(['1', '1', '2'])
    // Every change came by the event: no board read.
    expect(callsOf(host, 'board::get').length).toBe(reads)
  })

  it('keeps the counts of a partial lane: a card past its rows counts, one moved out of it too', async () => {
    const fake = sample()
    for (let i = 0; i < 60; i++) fake.add({ title: `Done ${i}`, status: 'done' })
    const { host } = await start({ fake })
    expect(host.lanes('lanes').counts[4]).toBe('61')
    // Created at the end of Done: counted, not drawn (it comes with the paging).
    await fake.elsewhere('ticket::create', { title: 'Late', status: 'done' })
    expect(host.lanes('lanes').counts[4]).toBe('62')
    expect(cards(host, 'done')).toHaveLength(50)
    // A card the app never read leaves Done for To do: drawn there, out of Done's count.
    await fake.elsewhere('ticket::move', { id: 'KAN-60', status: 'todo', index: 0 })
    expect(keysOf(host, 'todo')).toEqual(['KAN-60', 'KAN-2'])
    expect(host.lanes('lanes').counts.slice(1, 5)).toEqual(['2', '1', '1', '61'])
    // The next page still lines up.
    await host.press('more:done')
    await host.settle()
    expect(cards(host, 'done')).toHaveLength(61)
    expect(new Set(keysOf(host, 'done')).size).toBe(61)
  })

  it('an event older than what the board has, or repeated, changes nothing', async () => {
    const { host, fake } = await start({ live: false })
    const t = fake.get('KAN-5')
    if (!t) throw new Error('no KAN-5')
    const stale = { ...fake.summary(t), title: 'Stale title' }
    t.title = 'Fresh title'
    t.updated_at = fake.stamp()
    const event = (ticket: Record<string, any>) =>
      ({ event: 'ticket.updated', ticket_id: t.id, ticket_key: t.key, ticket, at: ticket.updated_at }) as never
    await host.emitTrigger('kanban:change', event(fake.summary(t)))
    await host.emitTrigger('kanban:change', event(stale))
    await host.emitTrigger('kanban:change', event(fake.summary(t)))
    await host.settle()
    expect(cards(host, 'backlog')[0]).toMatchObject({ title: 'Fresh title' })
    expect(keysOf(host, 'backlog')).toEqual(['KAN-5'])
  })

  it('config.updated reads the board again: new columns appear as lanes', async () => {
    const { host, fake } = await start()
    fake.columns.splice(4, 0, { id: 'qa', label: 'QA' })
    await host.emitTrigger('kanban:change', { event: 'config.updated', at: new Date().toISOString() })
    await host.settle()
    expect(host.lanes('lanes').titles).toEqual([
      'Backlog',
      'To do',
      'In progress',
      'In review',
      'QA',
      'Done',
      'Canceled',
    ])
    expect(host.findAll('lane:qa')).toHaveLength(1)
  })

  it('a card in a project the app has not heard of reads the board again, after the burst', async () => {
    const { host, fake } = await start()
    fake.projects.push({ id: 'p9', name: 'Brand new', color: 'green', lead: null, archived_at: null })
    const reads = callsOf(host, 'board::get').length
    await fake.elsewhere('ticket::update', { id: 'KAN-5', project: 'p9' })
    await fake.elsewhere('ticket::update', { id: 'KAN-2', project: 'p9' })
    expect(callsOf(host, 'board::get').length).toBe(reads)
    await host.tick(400)
    await host.settle()
    expect(callsOf(host, 'board::get').length).toBe(reads + 1)
  })

  it('binds nothing off screen, and binds again (then reads) when back', async () => {
    const { host, fake } = await start()
    await host.env({ visible: false })
    expect(host.subscriptions.size).toBe(0)
    fake.host = null
    fake.answer('kanban::ticket::create', { title: 'Missed', status: 'todo' })
    fake.host = host
    await host.env({ visible: true })
    await host.settle()
    expect(keysOf(host, 'todo')).toEqual(['KAN-2', 'KAN-6'])
  })
})

describe('the open ticket follows kanban:change', () => {
  it('reads itself again once per burst of its own events', async () => {
    const { host, fake } = await start()
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    expect(callsOf(host, 'ticket::get')).toHaveLength(1)
    await fake.elsewhere('comment::create', { ticket_id: 'KAN-4', body: 'Agent says hi', author: 'tech-lead' })
    await fake.elsewhere('ticket::update', { id: 'KAN-4', priority: 'high' })
    await fake.elsewhere('ticket::update', { id: 'KAN-4', labels: ['x'] })
    expect(callsOf(host, 'ticket::get')).toHaveLength(1)
    await host.tick(300)
    await host.settle()
    expect(callsOf(host, 'ticket::get')).toHaveLength(2)
    const items = host.items('activity') as Record<string, unknown>[]
    expect(items.find((i) => i.text === 'Agent says hi')).toMatchObject({ kind: 1, who: 'Tech Lead', hue: 2 })
    expect(host.find('priority').props).toMatchObject({ text: 'High' })
  })

  it('a subtask that changes updates its parent’s list and progress', async () => {
    const { host, fake } = await start()
    await host.itemPress('lane:in_progress', 't1')
    await host.settle()
    expect(host.findAll({ type: 'Text', text: '1 of 2' })).toHaveLength(1)
    await fake.elsewhere('ticket::move', { id: 'KAN-2', status: 'done' })
    await host.tick(300)
    await host.settle()
    expect(host.findAll({ type: 'Text', text: '2 of 2' })).toHaveLength(1)
  })

  it('an unrelated event reads nothing', async () => {
    const { host, fake } = await start()
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    await fake.elsewhere('ticket::update', { id: 'KAN-5', title: 'Other' })
    await host.tick(1000)
    await host.settle()
    expect(callsOf(host, 'ticket::get')).toHaveLength(1)
  })
})
