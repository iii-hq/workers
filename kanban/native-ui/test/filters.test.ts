/** The filters (design §8.2, §8.5, §8.6): a whole read when they start, the facets, storage, and the releases and projects pages. */
import { describe, expect, it } from 'vitest'
import { callsOf, keysOf, onTop, sample, start, topCard } from './fake'

describe('filters', () => {
  it('opening them reads the whole board, and the chips count what each option keeps', async () => {
    const fake = sample()
    for (let i = 0; i < 60; i++) fake.add({ title: `Done ${i}`, status: 'done', assignee: 'tech-lead' })
    const { host } = await start({ fake })
    await host.press('filters')
    await host.settle()
    expect(callsOf(host, 'board::get').at(-1)).toEqual({})
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('65 of 65 tickets')
    const chip = (text: string) =>
      host.findAll({ type: 'Chip', text }, { within: host.roots.get(topCard(host)?.inst ?? -1) })
    expect(chip('Tech Lead 61')).toHaveLength(1)
    expect(chip('Unassigned 61')).toHaveLength(0)
    expect(chip('Unassigned 1')).toHaveLength(1)
    await host.press(chip('Backend Engineer 2')[0].id)
    await host.settle()
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('2 of 65 tickets')
    expect(host.prop(onTop(host, 'show-tickets'), 'text')).toBe('Show 2 tickets')
    await host.press(onTop(host, 'show-tickets'))
    await host.settle()
    expect(topCard(host)?.name).toBe('root')
    expect(host.tree()?.props).toMatchObject({ subtitle: '2 of 65 tickets' })
    expect(host.lanes('lanes').counts).toEqual(['0/1', '0/1', '0/1', '1/1', '1/61', '0'])
    expect(host.prop('filters', 'active')).toBe(true)
    // The lanes that page no more (every card is here) show no Load more.
    expect(host.findAll('more:done')).toHaveLength(0)
  })

  it('searches key or title as text (a regex is just text), kept on the device', async () => {
    const { host } = await start()
    await host.press('filters')
    await host.settle()
    await host.type(onTop(host, 'filter-text'), '(a+)+$')
    await host.tick(300)
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('0 of 5 tickets')
    await host.type(onTop(host, 'filter-text'), 'index')
    await host.tick(300)
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('2 of 5 tickets')
    await host.tick(500)
    expect(host.storage.get('filters')).toMatchObject({ q: 'index' })
    await host.back()
    await host.settle()
    expect(keysOf(host, 'in_review')).toEqual(['KAN-4'])
    expect(keysOf(host, 'done')).toEqual(['KAN-3'])
    expect(keysOf(host, 'todo')).toEqual([])
    expect(host.findAll({ type: 'EmptyState', title: 'No ticket here matches the filters' }).length).toBeGreaterThan(0)
    // The next session opens filtered, and reads the whole board for it.
    const again = await start({ storage: { filters: { q: 'index' } } })
    expect(callsOf(again.host, 'board::get')).toEqual([{}])
    expect(keysOf(again.host, 'todo')).toEqual([])
  })

  it('Clear all and the board menu clear them', async () => {
    const { host, answers } = await start({ storage: { filters: { priorities: ['urgent'] } } })
    expect(keysOf(host, 'in_progress')).toEqual([])
    answers.menus.push('Clear filters')
    await host.press('board-menu')
    await host.settle()
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-1'])
    expect(host.prop('filters', 'active')).not.toBe(true)
  })

  it('a whole read that fails says so on the filters page, and Retry reads again', async () => {
    const fake = sample()
    for (let i = 0; i < 60; i++) fake.add({ title: `Done ${i}`, status: 'done' })
    let fail = true
    const real = (p: Record<string, unknown>) => fake.answer('kanban::board::get', p)
    const { host } = await start({ fake })
    fake.overrides['board::get'] = (p) => {
      if (fail && !p.limit) throw new Error('board.json is being rewritten')
      delete fake.overrides['board::get']
      return real(p)
    }
    await host.press('filters')
    await host.settle()
    expect(host.find(onTop(host, 'filters-error')).props).toMatchObject({
      headline: "Couldn't read the whole board",
      detail: 'board.json is being rewritten',
    })
    fail = false
    await host.press(onTop(host, { type: 'Button', text: 'Retry' }))
    await host.settle()
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('65 of 65 tickets')
  })

  it('labels: any or all of them', async () => {
    const { host } = await start()
    await host.press('filters')
    await host.settle()
    const root = host.roots.get(topCard(host)?.inst ?? -1)
    const chip = (text: string) => host.findAll({ type: 'Chip', text }, { within: root })[0].id
    await host.press(chip('frontend 2'))
    await host.press(chip('search 1'))
    await host.settle()
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('2 of 5 tickets')
    await host.select(onTop(host, 'label-mode'), 1)
    expect(host.prop(onTop(host, 'filters-page'), 'subtitle')).toBe('1 of 5 tickets')
  })
})

describe('releases and projects', () => {
  it('lists them to read; a tap shows its tickets on the board', async () => {
    const { host, answers } = await start()
    answers.menus.push('Releases')
    await host.press('board-menu')
    await host.settle()
    expect(topCard(host)?.name).toBe('releases')
    const rows = host.items(onTop(host, 'plan-list')) as Record<string, unknown>[]
    expect(rows.map((r) => [r.title, r.subtitle, r.badge])).toEqual([
      ['Phase 2', 'Oct 30 · 0 of 1 done', 'Planned'],
      ['Released', '', ''],
      ['Phase 1', 'Sep 1 · No tickets', 'Released'],
    ])
    await host.itemPress(onTop(host, 'plan-list'), 'r1')
    await host.settle()
    expect(topCard(host)?.name).toBe('root')
    expect(host.toasts.at(-1)).toEqual({ text: 'Showing Phase 2', kind: 'info' })
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-1'])
    expect(keysOf(host, 'in_review')).toEqual([])

    answers.menus.push('Projects')
    await host.press('board-menu')
    await host.settle()
    const projects = host.items(onTop(host, 'plan-list')) as Record<string, unknown>[]
    expect(projects.map((r) => [r.title, r.subtitle, r.hasBadge])).toEqual([
      ['iii cloud', 'Tech Lead · 0 of 1 done', false],
      ['Archived', '', false],
      ['Old site', 'No tickets', true],
    ])
  })
})
