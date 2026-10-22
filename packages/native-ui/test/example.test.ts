/** examples/hello builds (the known-good bundle for a host) and works in the harness. */
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
