/** Catalogue 2 in the SDK: Markdown, Lanes, drag and drop, hues, listFill, and the version an app declares. */
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { buildNativeUi } from '../src/build.mjs'
import { createTestHost, VmFailure } from '../src/testing/index.mjs'
import { fixture, mounted, opsSince } from './helpers'

const HUES = ['none', 'blue', 'purple', 'teal', 'green', 'amber', 'rose']

describe('the version an app declares', () => {
  it('goes in ready: 2 when defineApp says so, 1 by default', async () => {
    expect((await mounted('board')).ready.catalog).toBe(2)
    expect((await mounted('widgets')).ready.catalog).toBe(1)
  })

  it('keeps an older host from mounting the bundle, as the app does', async () => {
    const err = await createTestHost({ bundle: await fixture('board'), catalog: 1 }).catch((e) => e)
    expect(err).toBeInstanceOf(VmFailure)
    expect(err.kind).toBe('catalog')
    expect(err.message).toMatch(/needs catalog 2; this host renders 1/)
    const old = await createTestHost({ bundle: await fixture('widgets'), catalog: 1 })
    expect(old.initInfo()).toMatchObject({ catalog: 1 })
  })

  it('warns once per piece newer than the declared catalogue', async () => {
    const h = await mounted('undeclared')
    const warnings = h.logs.filter((l) => l.level === 'warn').map((l) => l.message)
    expect(warnings).toEqual([
      'native-ui: Markdown is catalogue 2, and this app declares 1: defineApp({ catalog: 2, … }), and NativeUi::catalog(2) in the worker',
      'native-ui: Chip.hue is catalogue 2, and this app declares 1: defineApp({ catalog: 2, … }), and NativeUi::catalog(2) in the worker',
    ])
    expect((await mounted('board')).logs.filter((l) => l.level === 'warn')).toEqual([])
  })

  it('must be one the SDK speaks', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'native-ui-catalog-'))
    try {
      const entry = join(dir, 'app.tsx')
      writeFileSync(
        entry,
        "import { defineApp, Text } from '@iii-dev/native-ui'\ndefineApp({ catalog: 3, pages: { root: () => <Text>hi</Text> } })\n",
      )
      await expect(buildNativeUi({ entry, outfile: join(dir, 'app.js'), ns: 'demo' })).rejects.toThrow(
        /catalog is a version from 1 to 2/,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})

describe('Lanes', () => {
  it('sends titles, counts, the selected lane and one child per lane', async () => {
    const h = await mounted('board')
    const lanes = h.lanes('lanes')
    expect(lanes.titles).toEqual(['Todo', 'Doing', 'Done'])
    expect(lanes.counts).toEqual(['3', '1', '0'])
    expect(lanes.selected).toBe(0)
    expect(lanes.lanes.map((l) => [l.type, l.props?.testTag, l.props?.dragGroup])).toEqual([
      ['List', 'lane-0', 'cards'],
      ['List', 'lane-1', 'cards'],
      ['List', 'lane-2', 'cards'],
    ])
    expect(h.find('lanes').props?.events).toEqual(['select'])
  })

  it('takes the lane the person settles on', async () => {
    const h = await mounted('board')
    const from = h.messages.length
    await h.select('lanes', 2)
    expect(h.lanes('lanes').selected).toBe(2)
    // The app took it: its own value, the one the host shows.
    expect(opsSince(h, from)).toEqual([[2, h.find('lanes').id, 18, 2]])
    await expect(h.select('lanes', 3)).rejects.toThrow(/not one of the 3 lanes/)
  })

  it('sends its own lane back when the app does not take the change', async () => {
    const h = await createTestHost({ bundle: await fixture('board') })
    const inst = await h.mount('stubborn')
    const { id } = h.find('stubborn')
    const from = h.messages.length
    await h.select('stubborn', 2)
    expect(opsSince(h, from)).toEqual([[2, id, 18, 1]])
    expect(h.tree(inst)?.props?.selected).toBe(1)
  })
})

describe('drag and drop', () => {
  it('moves a card across lists: the handler gets the source List as its ref', async () => {
    const h = await mounted('board')
    await h.drop('lane-1', 'b', 1, 'lane-0')
    expect(h.prop('last-drop', 'text')).toBe('b:1:0')
    expect(h.keys('lane-0')).toEqual(['a', 'c'])
    expect(h.keys('lane-1')).toEqual(['d', 'b'])
    expect(h.lanes('lanes').counts).toEqual(['2', '2', '0'])
    await h.drop('lane-2', 'd', 0, 'lane-1')
    expect(h.keys('lane-2')).toEqual(['d'])
  })

  it('reorders within a list (the index counts the items without the dragged one)', async () => {
    const h = await mounted('board')
    const from = h.messages.length
    await h.drop('lane-0', 'a', 2)
    expect(h.prop('last-drop', 'text')).toBe('a:2:0')
    expect(h.keys('lane-0')).toEqual(['b', 'c', 'a'])
    const lane0 = h.find('lane-0').id
    expect(h.toVm.at(-1)).toBe(`[[2,${lane0},15,["a",2,${lane0}]]]`)
    expect(opsSince(h, from).every((o) => o[0] === 8 || o[0] === 2)).toBe(true)
    await expect(h.drop('lane-0', 'a', 3)).rejects.toThrow(/out of 0\.\.2/)
  })

  it('refuses a drop the host would never send', async () => {
    const h = await mounted('board')
    await expect(h.drop('lane-1', 'zz', 0, 'lane-0')).rejects.toThrow(/no item "zz"/)
    await expect(h.drop('lanes', 'a', 0, 'lane-0')).rejects.toThrow(/not a List/)
  })

  it("fires a lifted card's longPress when released without moving", async () => {
    const h = await mounted('board')
    await h.itemLongPress('lane-0', 'b')
    expect(h.prop('pressed', 'text')).toBe('b teal')
  })
})

describe('hues', () => {
  it('go by name in a prop and in an item field bound to one; the host gets the index', async () => {
    const h = await mounted('board')
    expect(h.items('lane-0').map((it) => (it as { hue: number }).hue)).toEqual([
      HUES.indexOf('blue'),
      HUES.indexOf('teal'),
      HUES.indexOf('rose'),
    ])
    const badge = h.findAll({ type: 'Badge' }, { templates: true })[0]
    expect(badge.props?.hue).toEqual({ $item: 'hue' })
    await h.itemPress('lane-0', 'a')
    expect(h.prop('icon', 'hue')).toBe('teal')
    expect(h.violations).toEqual([])
  })
})

describe('ListDetail.listFill and Markdown', () => {
  it('opens a card: showDetail and the lane in one batch, its Markdown with links', async () => {
    const h = await mounted('board')
    expect(h.prop('ld', 'listFill')).toBe(true)
    await h.select('lanes', 0)
    const from = h.messages.length
    await h.itemPress('lane-1', 'd')
    expect(h.messages.length - from).toBe(1)
    expect(h.prop('ld', 'showDetail')).toBe(true)
    expect(h.lanes('lanes').selected).toBe(1)
    expect(h.find('body')).toMatchObject({
      type: 'Markdown',
      props: { text: '# d\n\nSee [the docs](https://example.com/d) and [KAN-2](kan:KAN-2).', dense: true },
    })
    await h.link('body', 'kan:KAN-2')
    expect(h.prop('last-link', 'text')).toBe('kan:KAN-2')
    await h.link('body', 'https://example.com/d')
    expect(h.prop('last-link', 'text')).toBe('https://example.com/d')
    expect(h.opened).toEqual([])
    await expect(h.link('body', 'https://elsewhere.dev')).rejects.toThrow(/has no "https:\/\/elsewhere.dev"/)
  })
})

describe('Markdown in a List template', () => {
  it('gets the item first, then the href (itemHandler types it)', async () => {
    const h = await createTestHost({ bundle: await fixture('board') })
    await h.mount('comments')
    await h.itemEmit('comments', 'c2', 'comment', 'link', ['https://example.com/pr/1'])
    expect(h.prop('comment-link', 'text')).toBe('c2 https://example.com/pr/1')
  })
})

describe('the test host', () => {
  it('fails a List whose items hold a bound field of the wrong type', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'native-ui-bound-'))
    afterAll(() => rmSync(dir, { recursive: true, force: true }))
    const entry = join(dir, 'app.tsx')
    writeFileSync(
      entry,
      `import { createList, defineApp, List, Text } from '@iii-dev/native-ui'
const rows = createList<{ key: string; n: number }>({ key: 'key', items: [{ key: 'a', n: 1 }] })
defineApp({ pages: { root: () => <List source={rows} templates={[(row) => <Text text={row.n} />]} /> } })
`,
    )
    const outfile = join(dir, 'app.js')
    await buildNativeUi({ entry, outfile, ns: 'demo', smoke: false })
    const h = await createTestHost({ bundle: readFileSync(outfile, 'utf8') })
    await expect(h.mount('root')).rejects.toThrow(/item "a" of List \d+: n \(bound to text\) is not a string: 1/)
  })
})
