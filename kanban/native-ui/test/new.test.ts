/** A new ticket (design §2, §4): where it starts, what it carries, a subtask's defaults, failures. */
import { describe, expect, it } from 'vitest'
import { callsOf, itemOf, keysOf, onTop, start, topCard } from './fake'

describe('a new ticket', () => {
  it('on a phone starts in the lane on screen, and opens once created', async () => {
    const { host, answers } = await start({ live: false })
    await host.select('lanes', 3)
    await host.press('new-ticket')
    expect(host.ready.pages.find((p) => p.name === 'new')).toMatchObject({ modal: true })
    expect(topCard(host)).toMatchObject({ name: 'new', params: { status: 'in_review' } })
    expect(host.prop(onTop(host, 'create'), 'enabled')).toBe(false)
    await host.type(onTop(host, 'new-title'), '  Fix the flaky test  ')
    await host.type(onTop(host, 'new-description'), 'It fails on **CI**.')
    answers.menus.push('High')
    await host.press(onTop(host, 'new-priority'))
    await host.press(onTop(host, 'new-assignee'))
    await host.itemPress(onTop(host, 'pick-list'), 'tech-lead')
    await host.settle()
    expect(host.prop(onTop(host, 'new-assignee'), 'subtitle')).toBe('Tech Lead')
    await host.press(onTop(host, 'create'))
    await host.settle()
    expect(callsOf(host, 'ticket::create')).toEqual([
      {
        title: 'Fix the flaky test',
        status: 'in_review',
        priority: 'high',
        description: 'It fails on **CI**.',
        assignee: 'tech-lead',
        actor: 'user',
      },
    ])
    expect(topCard(host)?.name).toBe('root')
    expect(keysOf(host, 'in_review')).toEqual(['KAN-4', 'KAN-6'])
    expect(host.prop('board-split', 'showDetail')).toBe(true)
    expect(host.find('ticket').props).toMatchObject({ title: 'KAN-6' })
  })

  it('on a wide screen starts in the default column', async () => {
    const { host } = await start({ env: { windowClass: 'wide' }, live: false })
    await host.select('lanes', 3)
    await host.press('new-ticket')
    expect(topCard(host)).toMatchObject({ name: 'new', params: { status: 'backlog' } })
    // Back drops it: nothing is created.
    await host.back()
    await host.settle()
    expect(callsOf(host, 'ticket::create')).toEqual([])
  })

  it('a failure keeps the form and says why', async () => {
    const { host, fake } = await start({ live: false })
    fake.overrides['ticket::create'] = () => {
      throw new Error('project Old site is archived; it takes no new tickets')
    }
    await host.press('new-ticket')
    await host.type(onTop(host, 'new-title'), 'Anything')
    await host.press(onTop(host, 'create'))
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({
      text: "Couldn't create the ticket: project Old site is archived; it takes no new tickets",
      kind: 'error',
    })
    expect(topCard(host)?.name).toBe('new')
    expect(host.prop(onTop(host, 'create'), 'loading')).not.toBe(true)
  })

  it('a subtask from the ticket menu starts in the first column with the parent’s project and release', async () => {
    const { host, answers } = await start({ live: false })
    await host.itemPress('lane:in_progress', 't1')
    await host.settle()
    answers.menus.push('Add subtask')
    await host.press('ticket-menu')
    await host.settle()
    expect(topCard(host)).toMatchObject({
      name: 'new',
      params: { parent: 'KAN-1', status: 'backlog', project: 'p1', release: 'r1' },
    })
    expect(host.prop(onTop(host, 'new'), 'title')).toBe('New subtask')
    expect(host.prop(onTop(host, 'new-project'), 'subtitle')).toBe('iii cloud')
    await host.type(onTop(host, 'new-title'), 'Third step')
    await host.press(onTop(host, 'create'))
    await host.settle()
    expect(callsOf(host, 'ticket::create')).toEqual([
      {
        title: 'Third step',
        status: 'backlog',
        priority: 'low',
        project: 'p1',
        release: 'r1',
        parent: 'KAN-1',
        actor: 'user',
      },
    ])
    expect(host.toasts.at(-1)).toEqual({ text: 'Created KAN-6', kind: 'success' })
    await host.settle()
    expect(itemOf(host, '@sub')).toMatchObject({ count: '1 of 3' })
  })
})
