/** examples/hello, examples/catalog2 and examples/catalog3 build (the known-good bundles for a host) and work in the harness. */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'
import { buildNativeUi } from '../src/build.mjs'
import { createTestHost } from '../src/testing/index.mjs'
import { pkg } from './helpers'

const outfile = join(pkg, 'examples/hello/dist/native.js')
let bundle = ''

beforeAll(async () => {
  const r = await buildNativeUi({ entry: join(pkg, 'examples/hello/src/app.tsx'), outfile, ns: 'hello' })
  expect(r.warnings).toEqual([])
  bundle = readFileSync(outfile, 'utf8')
})

describe('examples/hello', () => {
  it('greets, lists, toasts, offers a menu and remembers', async () => {
    const h = await createTestHost({ bundle, ns: 'hello', ui: { actionMenu: () => 1 } })
    await h.mount('root')
    expect(h.tree()?.props).toMatchObject({ title: 'Hello', subtitle: 'Compact window' })
    expect(h.find('greetings').items).toEqual([])
    await h.press('greet')
    expect(h.toasts.at(-1)).toEqual({ text: 'Type a name first', kind: 'error' })
    await h.type('name', ['A', 'Ada'])
    await h.press('greet')
    expect(h.prop('name', 'value')).toBe('')
    await h.press('add-world')
    expect(h.find('greetings').items).toEqual([
      { key: 'g2', name: 'Hello, world!', note: '#2' },
      { key: 'g1', name: 'Hello, Ada!', note: '#1' },
    ])
    expect(h.storage.get('names')).toEqual(['Hello, world!', 'Hello, Ada!'])
    await h.itemPress('greetings', 'g1')
    expect(h.toasts.at(-1)).toEqual({ text: 'Hello, Ada!', kind: 'info' })
    await h.itemLongPress('greetings', 'g2')
    expect(h.find('greetings').items).toEqual([{ key: 'g1', name: 'Hello, Ada!', note: '#1' }])
    await h.env({ windowClass: 'wide' })
    expect(h.tree()?.props?.subtitle).toBe('Wide window')

    const again = await createTestHost({ bundle, ns: 'hello', storage: { names: ['Hello, Ada!'] } })
    await again.mount('root')
    expect(again.find('greetings').items).toEqual([{ key: 'g1', name: 'Hello, Ada!', note: 'saved' }])
  })
})

describe('examples/catalog2', () => {
  const out = join(pkg, 'examples/catalog2/dist/catalog2.js')
  let source = ''
  beforeAll(async () => {
    const r = await buildNativeUi({ entry: join(pkg, 'examples/catalog2/src/app.tsx'), outfile: out, ns: 'catalog2' })
    expect(r.warnings).toEqual([])
    expect(r.ready?.catalog).toBe(2)
    source = readFileSync(out, 'utf8')
  })
  const open = async (page: string) => {
    const h = await createTestHost({ bundle: source, ns: 'catalog2' })
    await h.mount('root')
    await h.press(`open-${page}`)
    expect(h.deck.at(-1)?.name).toBe(page)
    return h
  }

  it('shows Markdown and reports each link, opening nothing', async () => {
    const h = await open('markdown')
    expect(h.prop('md-last-link', 'text')).toBe('none')
    await h.link('md-doc', 'kan:KAN-12')
    expect(h.prop('md-last-link', 'text')).toBe('kan:KAN-12')
    await h.link('md-doc', 'https://example.com')
    expect(h.prop('md-last-link', 'text')).toBe('https://example.com')
    await h.link('md-dense', 'https://example.com/dense')
    expect(h.prop('md-last-link', 'text')).toBe('https://example.com/dense')
    expect(h.find('md-nolink').props?.events).toBeUndefined()
    expect(h.opened).toEqual([])
  })

  it('sends a Markdown longer than maxMarkdownChars whole (the host cuts it)', async () => {
    const h = await open('markdownLong')
    const text = String(h.prop('md-long', 'text'))
    expect(text.length).toBeGreaterThan(200_000)
    expect(h.prop('md-long-length', 'text')).toBe(String(text.length))
    await h.link('md-long', 'https://example.com/early')
    expect(h.prop('md-long-last-link', 'text')).toBe('https://example.com/early')
    await expect(h.link('md-long', 'https://example.com/past')).rejects.toThrow(/past limits.maxMarkdownChars/)
  })

  it('has five lanes of thirty, and follows the lane in view', async () => {
    const h = await open('lanes')
    const lanes = h.lanes('lanes')
    expect(lanes.titles).toEqual(['Backlog', 'Todo', 'Doing', 'Review', 'Done'])
    expect(lanes.counts).toEqual(['30', '30', '30', '30', '30'])
    expect(lanes.lanes.map((l) => l.props?.testTag)).toEqual([0, 1, 2, 3, 4].map((i) => `lanes-list-${i}`))
    expect(h.keys('lanes-list-4')).toHaveLength(30)
    expect(h.prop('lanes-selected', 'text')).toBe('0')
    await h.select('lanes', 3)
    expect(h.prop('lanes-selected', 'text')).toBe('3')
  })

  it('moves cards across lanes and reorders a plain list', async () => {
    const h = await open('dnd')
    expect(h.lanes('dnd-lanes').counts).toEqual(['5', '3', '2'])
    await h.drop('dnd-list-1', 'c2', 0, 'dnd-list-0')
    expect(h.prop('dnd-last', 'text')).toBe('c2:0:0')
    expect(h.keys('dnd-list-0')).toEqual(['c1', 'c3', 'c4', 'c5'])
    expect(h.keys('dnd-list-1')).toEqual(['c2', 'c6', 'c7', 'c8'])
    expect(h.lanes('dnd-lanes').counts).toEqual(['4', '4', '2'])
    await h.drop('dnd-list-0', 'c1', 3)
    expect(h.prop('dnd-last', 'text')).toBe('c1:3:0')
    expect(h.keys('dnd-list-0')).toEqual(['c3', 'c4', 'c5', 'c1'])
    await h.itemPress('dnd-list-2', 'c9')
    expect(h.prop('dnd-pressed', 'text')).toBe('c9')
    await h.itemLongPress('dnd-list-2', 'c10')
    expect(h.prop('dnd-longpress', 'text')).toBe('c10')
    // The cards and the steps are in different drag groups.
    await expect(h.drop('dnd-reorder', 'c3', 0, 'dnd-list-0')).rejects.toThrow(/not in the drag group "order"/)
    await h.drop('dnd-reorder', 's1', 5)
    expect(h.prop('dnd-order', 'text')).toBe('s2,s3,s4,s5,s6,s1')
    expect(h.prop('dnd-order-last', 'text')).toBe('s1:5')
    // Hues by name in the script, by index on the host.
    expect((h.items('dnd-list-0')[0] as { hue: number }).hue).toBe(3)
  })

  it('fills the width with the board until a card opens, then shows its lane', async () => {
    const h = await open('fill')
    expect(h.prop('fill', 'listFill')).toBe(true)
    expect(h.prop('fill', 'showDetail')).toBeUndefined()
    await h.itemPress('fill-list-2', 'c10')
    expect(h.prop('fill', 'showDetail')).toBe(true)
    expect(h.lanes('fill-lanes').selected).toBe(2)
    expect(h.prop('fill-selected', 'text')).toBe('2')
    expect(h.prop('fill-detail-id', 'text')).toBe('c10')
    await h.link('fill-detail-body', 'kan:KAN-10')
    expect(h.toasts.at(-1)).toEqual({ text: 'kan:KAN-10', kind: 'info' })
    await h.press('fill-close')
    expect(h.prop('fill', 'showDetail')).toBe(false)
  })

  it('draws every hue, by prop and bound in a template', async () => {
    const h = await open('hue')
    for (const hue of ['blue', 'purple', 'teal', 'green', 'amber', 'rose']) {
      for (const part of ['icon', 'badge', 'dot', 'chip']) expect(h.prop(`hue-${part}-${hue}`, 'hue')).toBe(hue)
    }
    expect(h.prop('hue-icon-none', 'hue')).toBeUndefined()
    expect(h.items('hue-list').map((it) => (it as { hue: number }).hue)).toEqual([0, 1, 2, 3, 4, 5, 6])
  })
})

describe('examples/catalog3', () => {
  const out = join(pkg, 'examples/catalog3/dist/catalog3.js')
  let source = ''
  beforeAll(async () => {
    const r = await buildNativeUi({ entry: join(pkg, 'examples/catalog3/src/app.tsx'), outfile: out, ns: 'catalog3' })
    expect(r.warnings).toEqual([])
    expect(r.ready?.catalog).toBe(3)
    source = readFileSync(out, 'utf8')
  })
  const open = async (page: string, options: { ui?: { toast?: () => boolean } } = {}) => {
    const h = await createTestHost({ bundle: source, ns: 'catalog3', ...options })
    await h.mount('root')
    await h.press(`open-${page}`)
    expect(h.deck.at(-1)?.name).toBe(page)
    return h
  }

  it('opens every page without a warning or a violation', async () => {
    for (const page of [
      'overview',
      'checkbox',
      'people',
      'cards',
      'charts',
      'gauges',
      'steps',
      'code',
      'wave',
      'chat',
    ]) {
      const h = await open(page)
      expect(h.violations, page).toEqual([])
      expect(
        h.logs.filter((l) => l.level === 'warn' || l.level === 'error'),
        page,
      ).toEqual([])
    }
  })

  it('lays the overview out like a dashboard: stats, a question card, a chart, gauges, a meter, milestones', async () => {
    const h = await open('overview')
    expect(h.find('ov-agents').props).toMatchObject({ label: 'Agents', value: '6', icon: 'bot', hue: 'purple' })
    expect(h.prop('ov-cost', 'visible')).toBe(false)
    expect(h.find('ov-question').props).toMatchObject({ tone: 'attention' })
    expect(h.find('ov-latency').props).toMatchObject({ kind: 'area', axes: true, legend: ['api', 'db'] })
    expect(h.find('ov-steps').props?.items).toHaveLength(4)
    await h.env({ windowClass: 'wide' })
    expect(h.prop('ov-cost', 'visible')).toBe(true)
  })

  it('keeps the All checkbox in step with the list', async () => {
    const h = await open('checkbox')
    // checked false is the default, so it does not travel.
    expect(h.find('cb-all').props).toMatchObject({ indeterminate: true })
    expect(h.prop('cb-all', 'checked')).toBeUndefined()
    await h.toggle('cb-all')
    expect(h.prop('cb-count', 'text')).toBe('4 of 4 done')
    expect(h.find('cb-all').props).toMatchObject({ checked: true, indeterminate: false })
    await h.toggle('cb-1')
    expect(h.prop('cb-count', 'text')).toBe('3 of 4 done')
    expect(h.find('cb-all').props).toMatchObject({ checked: false, indeterminate: true })
    await h.toggle('cb-stubborn')
    expect(h.prop('cb-stubborn', 'checked')).toBe(false)
  })

  it('reports the point, the step and the line picked, and seeks the waveform', async () => {
    let h = await open('charts')
    await h.select('chart-bar', 4)
    expect(h.prop('chart-picked', 'text')).toBe('bar 4 Fri 30')
    await h.select('chart-line', 0)
    expect(h.prop('chart-picked', 'text')).toBe('line 0 04:00')
    await h.select('stat-p95', 19)
    expect(h.prop('chart-picked', 'text')).toBe('stat 19 228')
    h = await open('steps')
    await h.select('steps-vertical', 3)
    expect(h.prop('steps-picked', 'text')).toBe('3 Apple host')
    h = await open('code')
    await h.select('code-ts', 44)
    expect(h.prop('code-picked', 'text')).toBe('line 44')
    await expect(h.select('code-ts', 39)).rejects.toThrow(/not a line/)
    expect(String(h.prop('code-long', 'text')).length).toBeGreaterThan(200_000)
    h = await open('wave')
    await h.select('wave-voice', 0.75)
    expect(h.prop('wave-progress', 'text')).toBe('75%')
    expect(h.prop('wave-voice', 'progress')).toBe(0.75)
  })

  it('loads older messages at the top and undoes a delete from the toast', async () => {
    const h = await open('chat', { ui: { toast: () => true } })
    expect(h.find('chat-log').props).toMatchObject({ anchor: 'bottom', events: ['startReached'] })
    expect(h.keys('chat-log')).toHaveLength(30)
    await h.emit('chat-log', 'startReached')
    expect(h.keys('chat-log')).toHaveLength(50)
    expect(h.keys('chat-log')[0]).toBe(51)
    await h.press('chat-send')
    expect(h.keys('chat-log').at(-1)).toBe(101)
    await h.press('chat-delete')
    expect(h.toasts.at(-1)).toEqual({ text: 'Deleted message 101', kind: 'info', action: 'Undo' })
    expect(h.prop('chat-state', 'text')).toBe('restored 101')
    expect(h.keys('chat-log').at(-1)).toBe(101)
  })
})
