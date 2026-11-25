// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '@/types/chat'
import { MessageList } from './MessageList'

const messages: Message[] = [
  { id: 'm1', role: 'user', content: 'See acme/web#482', createdAt: 1 },
  { id: 'm2', role: 'assistant', content: 'Looking at it.', createdAt: 2 },
  { id: 'm3', role: 'user', content: 'Thanks', createdAt: 3 },
]

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.useFakeTimers()
  // jsdom has no scrolling; a landing scrolls to its row.
  Element.prototype.scrollTo ??= () => {}
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

const row = (id: string) =>
  container.querySelector<HTMLElement>(`[data-message-row~="${id}"]`)

describe('Show in conversation lands with a 1.2 s flash (§1.3.6)', () => {
  it('marks the landed row data-flash="true" for 1.2 s, then drops it', async () => {
    const handled = vi.fn()
    await act(() =>
      root.render(
        <MessageList
          messages={messages}
          focusMessageId="m1"
          onFocusMessageHandled={handled}
        />,
      ),
    )
    expect(handled).toHaveBeenCalledOnce()
    expect(row('m1')?.dataset.flash).toBe('true')
    expect(row('m2')?.dataset.flash).toBeUndefined()
    await act(async () => {
      vi.advanceTimersByTime(1199)
    })
    expect(row('m1')?.dataset.flash).toBe('true')
    await act(async () => {
      vi.advanceTimersByTime(1)
    })
    expect(row('m1')?.dataset.flash).toBeUndefined()
  })

  it('moves the mark to the next landing', async () => {
    await act(() =>
      root.render(<MessageList messages={messages} focusMessageId="m1" />),
    )
    await act(() =>
      root.render(<MessageList messages={messages} focusMessageId="m3" />),
    )
    expect(row('m1')?.dataset.flash).toBeUndefined()
    expect(row('m3')?.dataset.flash).toBe('true')
  })
})
