/** The SDK runtime in the harness: tree ops, capabilities, hooks, errors, limits. */
import { describe, expect, it } from 'vitest'
import { mounted, opsSince } from './helpers'

describe('tree ops', () => {
  it('destroys what a branch leaves behind, last in the batch', async () => {
    const h = await mounted('tree')
    const details = h.find('details')
    const from = h.messages.length
    await h.press('toggle')
    const ops = opsSince(h, from)
    expect(ops.at(-1)).toEqual([6, details.id])
    expect(ops.filter((o) => o[0] === 6)).toHaveLength(1)
    expect(h.findAll('details')).toHaveLength(0)
    expect(h.find('closed').props?.text).toBe('Closed')
    await h.press('toggle')
    expect(h.find('details').id).not.toBe(details.id)
    expect(h.violations).toEqual([])
  })

  it('moves children within a slot instead of recreating them', async () => {
    const h = await mounted('tree')
    const ids = ['a', 'b', 'c'].map((n) => h.find(`item-${n}`).id)
    const from = h.messages.length
    await h.press('rotate')
    const ops = opsSince(h, from)
    expect(ops.every((o) => o[0] === 4)).toBe(true)
    expect(['b', 'c', 'a'].map((n) => h.find(`item-${n}`).id)).toEqual([ids[1], ids[2], ids[0]])
    const col = h.find('col')
    const order = col.slots?.children.map((c) => c.props?.testTag).filter((t) => String(t).startsWith('item-'))
    expect(order).toEqual(['item-b', 'item-c', 'item-a'])
    await h.press('reverse')
    const order2 = h
      .find('col')
      .slots?.children.map((c) => c.props?.testTag)
      .filter((t) => String(t).startsWith('item-'))
    expect(order2).toEqual(['item-a', 'item-c', 'item-b'])
  })

  it('sends a node held outside its JSX again under a new id after a destroy', async () => {
    const h = await mounted('tree')
    const kept = h.find('kept')
    await h.press('kept-toggle')
    expect(h.findAll('kept')).toHaveLength(0)
    expect(opsSince(h, h.messages.length - 1)).toContainEqual([6, kept.id])
    await h.press('kept-toggle')
    const again = h.find('kept')
    expect(again.id).toBeGreaterThan(kept.id)
    expect(again.props?.text).toBe('Kept')
  })

  it('puts slot props in their slots and warns about loose text', async () => {
    const h = await mounted('tree')
    const page = h.tree()!
    expect(page.slots?.actions?.[0].props?.testTag).toBe('sheet')
    expect(page.slots?.bottom?.[0].props?.testTag).toBe('bottom')
    expect(h.logs.some((l) => l.level === 'warn' && l.message.includes('loose text'))).toBe(true)
    expect(h.find('the-sheet').props?.open).toBeUndefined()
    await h.press('sheet')
    expect(h.find('the-sheet').props?.open).toBe(true)
    await h.emit('the-sheet', 'dismiss')
    expect(h.find('the-sheet').props?.open).toBe(false)
  })

  it('keeps a Switch and a Segmented on the value the app chose', async () => {
    const h = await mounted('tree')
    await h.toggle('switch')
    expect(h.prop('switch', 'checked')).toBe(true)
    await h.select('seg', 1)
    expect(h.prop('seg', 'selected')).toBe(1)
    await h.press('strict')
    await h.toggle('switch')
    expect(h.prop('switch', 'checked')).toBe(true)
    await h.select('seg', 0)
    expect(h.prop('seg', 'selected')).toBe(1)
  })
})

describe('the capability object', () => {
  const calls = { 'demo::echo': (payload: unknown) => ({ echo: payload }) }

  it('calls its own namespace, with the timeout clamped', async () => {
    const h = await mounted('capabilities', { calls })
    await h.press('call-own')
    expect(h.prop('out', 'text')).toBe('ok {"echo":{"a":1}}')
    expect(h.requests.at(-1)).toEqual({
      reqId: 1,
      method: 'call',
      args: { fn: 'demo::echo', payload: { a: 1 }, timeoutMs: 120000 },
    })
    await h.press('call-missing')
    expect(h.prop('out', 'text')).toBe('error function_not_found: Function demo::missing not found')
  })

  it('refuses other namespaces before anything reaches the host', async () => {
    const h = await mounted('capabilities', { calls })
    const sent = h.requests.length
    await h.press('call-other')
    expect(h.prop('out', 'text')).toBe('error forbidden: demo may only call demo::… functions (not state::get)')
    await h.press('call-engine')
    expect(h.prop('out', 'text')).toBe(
      'error forbidden: demo may only call demo::… functions (not engine::functions::list)',
    )
    await h.press('bind-other')
    expect(h.prop('out', 'text')).toBe('threw forbidden: demo may only bind demo::… triggers (not state::changed)')
    expect(h.requests.length).toBe(sent)
  })

  it('reaches the host UI, the clipboard, URLs, files and storage', async () => {
    const h = await mounted('capabilities', {
      calls,
      ui: { confirm: () => true, prompt: () => 'typed' },
    })
    await h.press('toast')
    expect(h.toasts).toEqual([{ text: 'Saved', kind: 'success' }])
    await h.press('confirm')
    expect(h.prop('out', 'text')).toBe('ok true')
    expect(h.requests.at(-1)?.args).toEqual({ title: 'Clear?', confirm: 'Clear', destructive: true })
    await h.press('prompt')
    expect(h.prop('out', 'text')).toBe('ok "typed"')
    expect(h.requests.at(-1)?.args).toEqual({ title: 'Name', confirm: 'OK', keyboard: 1, mono: true })
    await h.press('clipboard')
    expect(h.clipboard).toEqual(['copied'])
    await h.press('open-http')
    expect(h.prop('out', 'text')).toBe('ok true')
    await h.press('open-file')
    expect(h.prop('out', 'text')).toMatch(/^error forbidden: openUrl only opens http, https and mailto/)
    await h.press('share')
    expect(h.shared).toEqual([{ text: 'hello', title: 'Hi' }])
    await h.press('save')
    expect(h.prop('out', 'text')).toBe('ok {"path":"Downloads/iii/hi.txt"}')
    await h.press('storage')
    expect(h.prop('out', 'text')).toBe('ok {"v":{"v":1},"keys":["k"]}')
    await h.press('storage-long-key')
    expect(h.prop('out', 'text')).toBe('error invalid: storage keys have at most 128 characters')
    await h.press('push-unknown')
    expect(h.prop('out', 'text')).toBe('error invalid: nav.push: "nowhere" is not one of the pages of defineApp')
    await h.press('app')
    expect(h.prop('out', 'text')).toBe('worker:demo demo')
    await h.press('report')
    expect(h.errors.at(-1)).toMatchObject({ message: 'soft failure' })
    expect(h.errors.at(-1)?.fatal).toBeUndefined()
  })
})

describe('hooks', () => {
  it('useCall follows its payload and stops when it is undefined', async () => {
    const h = await mounted('hooks', { calls: { 'demo::echo': (p: unknown) => ({ echo: (p as { q: string }).q }) } })
    expect(h.prop('result', 'text')).toBe('one')
    expect(h.prop('echo', 'text')).toBe('echo one')
    await h.press('two')
    expect(h.prop('result', 'text')).toBe('two')
    const calls = h.requests.filter((r) => r.method === 'call').length
    await h.press('none')
    expect(h.requests.filter((r) => r.method === 'call').length).toBe(calls)
  })

  it('useTrigger binds while enabled and unbinds when it is not', async () => {
    const h = await mounted('hooks', { calls: { 'demo::echo': { echo: 'x' } } })
    expect([...h.subscriptions.values()]).toEqual([{ trigger: 'demo::tick', config: { every: 1 } }])
    expect(await h.emitTrigger('demo::tick', {})).toBe(1)
    expect(h.prop('events', 'text')).toBe('1')
    await h.toggle('live')
    expect(h.subscriptions.size).toBe(0)
    expect(h.requests.at(-1)?.method).toBe('off')
    expect(await h.emitTrigger('demo::tick', {})).toBe(0)
    await h.toggle('live')
    expect(h.subscriptions.size).toBe(1)
  })

  it('onEnv and useEnv follow the host', async () => {
    const h = await mounted('hooks', { calls: { 'demo::echo': { echo: 'x' } } })
    await h.env({ visible: false })
    expect(h.prop('visible', 'text')).toBe('hidden')
    await h.env({ windowClass: 'wide', posture: 'book' })
    expect(h.prop('env-log', 'text')).toBe('visible | windowClass,posture')
  })

  it('usePage replaces its card and popToRoot answers the push', async () => {
    const h = await mounted('hooks', { calls: { 'demo::echo': { echo: 'x' } } })
    await h.press('detail')
    expect(h.deck.map((c) => c.name)).toEqual(['root', 'detail'])
    expect(h.tree()?.props?.title).toBe('Detail {"n":1}')
    await h.press('swap')
    expect(h.deck.map((c) => c.name)).toEqual(['root', 'other'])
    expect(h.prop('from', 'text')).toBe('2')
    await h.press('home')
    expect(h.deck.map((c) => c.name)).toEqual(['root'])
    expect(h.pending.size).toBe(0)
    expect(h.ready.pages).toEqual([
      { name: 'root', modal: false, fullScreen: false },
      { name: 'detail', modal: false, fullScreen: false },
      { name: 'other', modal: true, fullScreen: false },
    ])
  })
})

describe('errors', () => {
  it('reports a throwing handler or trigger without stopping the app', async () => {
    const h = await mounted('errors')
    await h.press('throw')
    expect(h.prop('n', 'text')).toBe('1')
    expect(h.errors.at(-1)).toMatchObject({ message: 'TypeError: handler exploded' })
    await h.emitTrigger('demo::boom', {})
    expect(h.errors.at(-1)).toMatchObject({ message: 'trigger exploded' })
    await h.press('ok')
    expect(h.prop('n', 'text')).toBe('2')
    expect(h.failed).toBeNull()
  })

  it('makes a page that cannot render fatal', async () => {
    const h = await mounted('errors')
    await expect(h.mount('broken')).rejects.toThrow(/reported: cannot render/)
    expect(h.errors.at(-1)).toMatchObject({ message: 'cannot render', fatal: true })
  })

  it('ignores events of nodes that are gone and unknown codes', async () => {
    const h = await mounted('errors')
    const from = h.messages.length
    await h.deliver([
      [2, 9999, 1, []],
      [42, 'future'],
      [3, 9998, 'k', 9997, 1, []],
    ])
    expect(h.messages.length).toBe(from)
  })
})

describe('limits', () => {
  it('splits a batch the host refuses as too large, and stops on one entry that cannot fit', async () => {
    const h = await mounted('big', { maxMessageBytes: 4096 })
    const from = h.messages.length
    await h.emitTrigger('demo::fill', { count: 6, size: 1000 })
    const sent = h.messages.slice(from)
    expect(sent.length).toBeGreaterThan(1)
    for (const m of sent) expect(Buffer.byteLength(m)).toBeLessThanOrEqual(4096)
    expect(h.find({ type: 'List' }).items).toHaveLength(6)
    await expect(h.emitTrigger('demo::fill', { count: 1, size: 5000 })).rejects.toThrow(/reported/)
    expect(h.errors.at(-1)?.fatal).toBe(true)
  })
})

describe('components', () => {
  it('sends RemoteScreen its wire and enums, and passes its event arguments', async () => {
    const h = await mounted('widgets')
    const screen = h.find('screen')
    expect(screen.props).toMatchObject({
      session: 's1',
      mode: 'view',
      placeholder: 'Loading…',
      events: ['longPress', 'frame'],
      wire: {
        frames: { trigger: 'demo::frame-event', config: { session_id: '$session' } },
        start: { fn: 'demo::screencast::start', args: { session_id: '$session', viewer_id: '$viewer' }, renewMs: 6000 },
      },
    })
    await h.longPress('screen', [12.5, 40])
    expect(h.prop('point', 'text')).toBe('12.5,40')
    await h.emit('screen', 'frame', [390, 844])
    expect(h.prop('point', 'text')).toBe('390x844')
  })

  it('puts ListDetail slots in place and follows its dismiss', async () => {
    const h = await mounted('widgets')
    const ld = h.find('ld')
    expect(Object.keys(ld.slots ?? {})).toEqual(['list', 'detail'])
    expect(ld.props).toMatchObject({ listTitle: 'Tabs', events: ['dismiss'] })
    await h.itemPress({ type: 'List' }, 'a')
    expect(h.prop('ld', 'showDetail')).toBe(true)
    await h.emit('ld', 'dismiss')
    expect(h.prop('ld', 'showDetail')).toBe(false)
  })

  it('binds nested item fields, also as Text children', async () => {
    const h = await mounted('widgets')
    const list = h.find({ type: 'List' })
    const row = list.slots?.template0?.[0]
    expect(row?.props).toMatchObject({ title: { $item: 'title' }, subtitle: { $item: 'meta.host' } })
    expect(row?.slots?.trailing?.[0].props).toMatchObject({ text: { $item: 'meta.host' } })
  })

  it('does not echo the host its own text, and writes with the last count', async () => {
    const h = await mounted('widgets')
    const from = h.messages.length
    await h.type('q', 'abc')
    expect(h.messages.length).toBe(from)
    await h.press('clear')
    expect(JSON.parse(h.messages.at(-1)!)).toEqual([[2, h.find('q').id, 16, ['', 1]]])
    expect(h.prop('q', 'value')).toBe('')
  })

  it('asks the host to focus a node by its ref', async () => {
    const h = await mounted('widgets')
    await h.press('focus')
    expect(h.focusRequests).toEqual([{ node: h.find('q').id, focused: true }])
  })
})
