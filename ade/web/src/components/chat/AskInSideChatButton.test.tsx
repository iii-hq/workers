// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AskInSideChatContext, type SideChatQuote } from '@/lib/chat-aside'
import { AskInSideChatButton } from './CopyMessageButton'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

let root: Root | null = null
let container: HTMLElement | null = null

function render(ask: ((quote: SideChatQuote) => void) | null) {
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  act(() =>
    root?.render(
      <AskInSideChatContext.Provider
        value={ask ? { ask, quotedIds: new Set() } : null}
      >
        <article>
          <p data-prose>first sentence. second sentence.</p>
          <AskInSideChatButton
            text="the whole message"
            messageId="m1"
            agent="Frontend"
          />
        </article>
        <p data-outside>elsewhere</p>
      </AskInSideChatContext.Provider>,
    ),
  )
  return container
}

const askButton = () =>
  container?.querySelector<HTMLButtonElement>('[aria-label="Ask in side chat"]')

function select(node: Element) {
  const range = document.createRange()
  range.selectNodeContents(node)
  const selection = window.getSelection()
  selection?.removeAllRanges()
  selection?.addRange(range)
}

afterEach(() => {
  window.getSelection()?.removeAllRanges()
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

describe('AskInSideChatButton', () => {
  it('renders nothing where the chat has no side panel', () => {
    render(null)
    expect(askButton()).toBeNull()
  })

  it('asks about the whole message when nothing is selected', () => {
    const ask = vi.fn()
    render(ask)
    act(() => askButton()?.click())
    expect(ask).toHaveBeenCalledWith({
      messageId: 'm1',
      text: 'the whole message',
      agent: 'Frontend',
    })
  })

  it('asks about the selection inside the message', () => {
    const ask = vi.fn()
    const view = render(ask)
    select(view.querySelector('[data-prose]') as Element)
    act(() => askButton()?.click())
    expect(ask).toHaveBeenCalledWith({
      messageId: 'm1',
      text: 'first sentence. second sentence.',
      agent: 'Frontend',
    })
  })

  it('ignores a selection outside the message', () => {
    const ask = vi.fn()
    const view = render(ask)
    select(view.querySelector('[data-outside]') as Element)
    act(() => askButton()?.click())
    expect(ask).toHaveBeenCalledWith({
      messageId: 'm1',
      text: 'the whole message',
      agent: 'Frontend',
    })
  })
})
