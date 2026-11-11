// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message, SystemMessage } from '@/types/chat'
import { MessageList } from './MessageList'

const wait = (n: number): SystemMessage => ({
  id: `e_obsskip_sub_1_condition_${String(n).padStart(32, 'a')}`,
  role: 'system',
  kind: 'observer-fired',
  content: 'resend batch watch',
  observer: {
    observer_id: 'sub_1',
    target: 'harness::send',
    action: 'A send-over-time batch changed in Resend',
    once: false,
    retired: false,
    fired_at: 1,
  },
  createdAt: 1,
})
const run: Message[] = [wait(1), wait(2), wait(3)]

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  // jsdom has no scrolling; a landing scrolls to its row.
  Element.prototype.scrollTo ??= () => {}
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

describe('a run of observer events', () => {
  it('shows an eye and a count that opens the Observers section', async () => {
    const onShowObservers = vi.fn()
    await act(() =>
      root.render(
        <MessageList messages={run} onShowObservers={onShowObservers} />,
      ),
    )
    const button = container.querySelector<HTMLButtonElement>(
      '[data-observer-rollup] button',
    )
    expect(button?.textContent).toContain('3events')
    expect(
      container.querySelector('[data-observer-sources]')?.textContent,
    ).toContain('A send-over-time batch changed in Resend')
    expect(
      container.querySelectorAll('[data-message-role="observer-activity"]'),
    ).toHaveLength(0)
    await act(() => button?.click())
    expect(onShowObservers).toHaveBeenCalledOnce()
  })

  it('shows a single event as itself', async () => {
    await act(() =>
      root.render(
        <MessageList messages={[wait(1)]} onShowObservers={() => {}} />,
      ),
    )
    expect(container.querySelector('[data-observer-rollup]')).toBeNull()
    expect(
      container.querySelector('[data-message-role="observer-activity"]'),
    ).not.toBeNull()
  })

  it('opens the run when a landing targets one of its events', async () => {
    await act(() =>
      root.render(
        <MessageList
          messages={run}
          onShowObservers={() => {}}
          focusMessageId={run[1].id}
        />,
      ),
    )
    expect(container.querySelector('[data-observer-rollup]')).toBeNull()
    expect(
      container.querySelector(`[data-message-row~="${run[1].id}"]`),
    ).not.toBeNull()
  })

  it('stays an inline group where there is no side panel', async () => {
    await act(() => root.render(<MessageList messages={run} />))
    expect(container.querySelector('[data-observer-rollup]')).toBeNull()
    expect(
      container.querySelector('.chat-tool-group__label')?.textContent,
    ).toContain('Observed')
  })
})
