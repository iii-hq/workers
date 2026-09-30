// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FolderBrowser, trailOf } from './FolderBrowser'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const tree: Record<string, string[]> = {
  '/w': ['api', 'web'],
  '/w/web': ['src'],
  '/w/web/src': [],
}

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({
    trigger: async (id: string, payload: { path?: string }) =>
      id === 'shell::workspace::roots'
        ? { roots: ['/w'] }
        : {
            entries: (tree[payload.path ?? ''] ?? []).map((name) => ({
              name,
              kind: 'dir',
              path: `${payload.path}/${name}`,
            })),
          },
  }),
}))

let root: Root
let host: HTMLDivElement
const onUse = vi.fn()

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn()
  Element.prototype.scrollTo = vi.fn()
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
})

afterEach(() => {
  act(() => root.unmount())
  host.remove()
  onUse.mockReset()
})

const settle = () => act(async () => {})
const input = () =>
  host.querySelector('input[name="folder-search"]') as HTMLInputElement
async function press(key: string, init: KeyboardEventInit = {}) {
  await act(async () => {
    input().dispatchEvent(
      new KeyboardEvent('keydown', { key, bubbles: true, ...init }),
    )
  })
}
const columns = () =>
  [...host.querySelectorAll('.overflow-y-auto')].map((col) =>
    [...col.querySelectorAll('button')].map((b) => b.textContent),
  )

describe('FolderBrowser keyboard', () => {
  it('trailOf stops at the root and the depth', () => {
    expect(trailOf('/w/web/src', '/w', 3)).toEqual([
      '/w',
      '/w/web',
      '/w/web/src',
    ])
    expect(trailOf('/w/web/src', '/w/web', 3)).toEqual(['/w/web', '/w/web/src'])
    expect(trailOf('/a/b/c/d', '/', 3)).toEqual(['/a/b', '/a/b/c', '/a/b/c/d'])
    expect(trailOf('/', '/', 3)).toEqual(['/'])
  })

  it('arrows move, Enter opens, ← goes up, Cmd/Ctrl+Enter uses', async () => {
    act(() =>
      root.render(
        <FolderBrowser wide keyboard busy={false} error={null} onUse={onUse} />,
      ),
    )
    await settle()
    expect(columns()).toEqual([['api', 'web']])

    await press('ArrowDown')
    await press('ArrowDown')
    await press('Enter')
    await settle()
    // parent column keeps web open next to its children
    expect(columns()).toEqual([['api', 'web'], ['src']])

    // nothing highlighted: Cmd+Enter uses the folder you are in
    await press('Enter', { metaKey: true })
    expect(onUse).toHaveBeenLastCalledWith('/w/web')

    await press('ArrowRight') // nothing highlighted: stays
    await press('ArrowDown')
    await press('ArrowRight')
    await settle()
    expect(columns()).toEqual([['api', 'web'], ['src'], []])

    // ← returns with the folder you came from highlighted
    await press('ArrowLeft')
    await press('Enter', { ctrlKey: true })
    expect(onUse).toHaveBeenLastCalledWith('/w/web/src')

    // typing highlights the first match
    await press('ArrowLeft')
    await act(async () => {
      const set = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        'value',
      )?.set
      set?.call(input(), 'ap')
      input().dispatchEvent(new Event('input', { bubbles: true }))
    })
    await press('Enter', { metaKey: true })
    expect(onUse).toHaveBeenLastCalledWith('/w/api')
  })
})
