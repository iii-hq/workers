// @vitest-environment jsdom

import { Blocks } from 'lucide-react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createKeyDispatcher } from '@/hooks/use-keybindings'
import { EmptyPane } from './EmptyPane'
import type { ScreenOption } from './use-screen-options'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const pages: ScreenOption[] = ['chat', 'traces'].map((value) => ({
  value,
  label: value,
  icon: Blocks,
}))

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  // jsdom lays nothing out; the launcher scrolls its highlight into view.
  Element.prototype.scrollIntoView = () => {}
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(() => {
  act(() => root.unmount())
  host.remove()
})

const search = () =>
  host.querySelector<HTMLInputElement>(
    'input[type="search"]',
  ) as HTMLInputElement
const pressEscape = () =>
  act(() => {
    search().dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
    )
  })
const type = (text: string) =>
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    setter?.call(search(), text)
    search().dispatchEvent(new Event('input', { bubbles: true }))
  })

// QA 2026-11-16 F3: Escape on the empty "What should this panel show?"
// launcher did nothing; the empty panel stayed open.
describe('EmptyPane Escape', () => {
  it('clears a query first, then closes the empty panel', () => {
    const onRemove = vi.fn()
    act(() =>
      root.render(
        <EmptyPane
          screenOptions={pages}
          onAttach={vi.fn()}
          onRemove={onRemove}
        />,
      ),
    )
    type('tra')
    expect(search().value).toBe('tra')
    pressEscape()
    expect(search().value).toBe('')
    expect(onRemove).not.toHaveBeenCalled()
    pressEscape()
    expect(onRemove).toHaveBeenCalledTimes(1)
  })

  it('lets go of the caret when the panel cannot close', () => {
    act(() =>
      root.render(<EmptyPane screenOptions={pages} onAttach={vi.fn()} />),
    )
    act(() => search().focus())
    expect(document.activeElement).toBe(search())
    pressEscape()
    expect(document.activeElement).not.toBe(search())
  })
})

// QA 2026-11-16 F1: ⌃T opens a workspace whose launcher search takes the
// caret, and then ⌃T, ⌃W and ⌃⇧→ did nothing: the search swallowed them.
describe('EmptyPane workspace keys', () => {
  it('lets the workspace chords through its focused search', () => {
    act(() =>
      root.render(<EmptyPane screenOptions={pages} onAttach={vi.fn()} />),
    )
    const ran: string[] = []
    const dispatcher = createKeyDispatcher(
      () => ({
        'workspace.create': () => ran.push('create'),
        'workspace.close': () => ran.push('close'),
        'workspace.next': () => ran.push('next'),
        'workspace.previous': () => ran.push('previous'),
      }),
      'mac',
      () => [],
    )
    const chord = (key: string, shiftKey = false) =>
      dispatcher.onKeyDown({
        key,
        code: '',
        ctrlKey: true,
        metaKey: false,
        altKey: false,
        shiftKey,
        isComposing: false,
        repeat: false,
        target: search(),
        preventDefault: () => {},
      })
    chord('t')
    chord('w')
    chord('ArrowRight', true)
    chord('ArrowLeft', true)
    expect(ran).toEqual(['create', 'close', 'next', 'previous'])
  })

  it('leaves modified arrows to the workspace, not its own list', () => {
    act(() =>
      root.render(<EmptyPane screenOptions={pages} onAttach={vi.fn()} />),
    )
    const event = new KeyboardEvent('keydown', {
      key: 'ArrowLeft',
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    })
    act(() => {
      search().dispatchEvent(event)
    })
    // Unclaimed: the console's key listener still sees it.
    expect(event.defaultPrevented).toBe(false)
  })
})
