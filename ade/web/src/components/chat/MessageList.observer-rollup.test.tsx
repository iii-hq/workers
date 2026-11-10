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
    expect(button?.textContent).toBe('3events')
    expect(
      container.querySelectorAll('[data-message-role="observer-activity"]'),
    ).toHaveLength(0)
    await act(() => button?.click())
    expect(onShowObservers).toHaveBeenCalledOnce()
  })

  it('stays an inline group where there is no side panel', async () => {
    await act(() => root.render(<MessageList messages={run} />))
    expect(container.querySelector('[data-observer-rollup]')).toBeNull()
    expect(
      container.querySelector('.chat-tool-group__label')?.textContent,
    ).toContain('Observed')
  })
})
