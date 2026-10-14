// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SelectionToolbar } from './ChatAside'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const quote = { messageId: 'm1', text: 'views' }
const rect = new DOMRect(100, 300, 40, 20)
const bounds = new DOMRect(0, 100, 800, 600)

let root: Root
let container: HTMLElement
let picked: typeof quote | null
const onAsk = vi.fn()

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    cb(0)
    return 0
  })
  picked = quote
  onAsk.mockReset()
  container = document.createElement('div')
  container.innerHTML =
    '<div data-message-list><p>text</p></div><nav><button>elsewhere</button></nav>'
  document.body.appendChild(container)
  root = createRoot(document.createElement('div'))
  act(() =>
    root.render(
      <SelectionToolbar
        read={() => (picked ? { quote: picked, rect, bounds } : null)}
        onAsk={onAsk}
      />,
    ),
  )
})

afterEach(() => {
  act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

const toolbar = () => document.querySelector('[data-selection-toolbar]')
const fire = (target: EventTarget, type: string) =>
  act(() => {
    target.dispatchEvent(new Event(type, { bubbles: true }))
  })

describe('SelectionToolbar', () => {
  it('shows over a selection and asks about it', () => {
    fire(document, 'selectionchange')
    expect(toolbar()).not.toBeNull()
    const ask = [...document.querySelectorAll('button')].find(
      (b) => b.textContent === 'Ask in side chat',
    )
    act(() => ask?.click())
    expect(onAsk).toHaveBeenCalledWith(quote)
    expect(toolbar()).toBeNull()
  })

  it('hides on Escape and when the selection leaves the transcript box', () => {
    fire(document, 'selectionchange')
    act(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    })
    expect(toolbar()).toBeNull()
    picked = null
    fire(document, 'selectionchange')
    expect(toolbar()).toBeNull()
  })

  it('waits for the press to end, and a press elsewhere does not bring it back', () => {
    const text = container.querySelector('p') as HTMLElement
    const elsewhere = container.querySelector('nav button') as HTMLElement
    fire(text, 'pointerdown')
    fire(document, 'selectionchange')
    expect(toolbar()).toBeNull()
    fire(text, 'pointerup')
    expect(toolbar()).not.toBeNull()
    fire(elsewhere, 'pointerdown')
    fire(elsewhere, 'pointerup')
    expect(toolbar()).toBeNull()
  })

  it('recovers from a press that ends in a drag', () => {
    const text = container.querySelector('p') as HTMLElement
    fire(text, 'pointerdown')
    fire(text, 'pointercancel')
    fire(document, 'selectionchange')
    expect(toolbar()).not.toBeNull()
  })
})
