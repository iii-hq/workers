// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FunctionCallMessage, Message } from '@/types/chat'
import { MessageList } from './MessageList'

const observe = (
  id: string,
  functionId: string,
  extra: Partial<FunctionCallMessage> = {},
): FunctionCallMessage => ({
  id,
  role: 'function-call',
  functionId: 'harness::observe',
  input: { function_id: functionId },
  output: { observer_id: `sub_${id}` },
  durationMs: 2,
  createdAt: 0,
  ...extra,
})

const call = (id: string, functionId: string): FunctionCallMessage => ({
  id,
  role: 'function-call',
  functionId,
  input: {},
  output: { ok: true },
  durationMs: 2,
  createdAt: 0,
})

// The screenshot's batch: six functions, one of them observed five times.
const batch: Message[] = [
  call('c0', 'engine::functions::list'),
  observe('o1', 'session::on-deleted'),
  observe('o2', 'directory::skills::on-change'),
  observe('o3', 'directory::agents::on-change'),
  observe('o4', 'directory::kits::on-change'),
  observe('o5', 'secrets::on-changed'),
  ...[6, 7, 8, 9, 10].map((n) => observe(`o${n}`, 'configuration::on-updated')),
]

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
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

const runs = () =>
  container.querySelectorAll<HTMLElement>('[data-message-role="observe-run"]')
const observeRows = () =>
  container.querySelectorAll(
    '[data-message-role="function-call"][data-function-id="harness::observe"]',
  )

describe('consecutive observes', () => {
  it('fold into one row that lists every observed function', async () => {
    await act(() => root.render(<MessageList messages={batch} />))
    expect(runs()).toHaveLength(1)
    const run = runs()[0]
    expect(run.querySelector('.chat-tool-row__title')?.textContent).toBe(
      'Observing 6 functions',
    )
    expect(run.querySelector('[data-observe-run-list]')?.textContent).toBe(
      'session::on-deleted, directory::skills::on-change, directory::agents::on-change, directory::kits::on-change, secrets::on-changed, configuration::on-updated ×5',
    )
    expect(observeRows()).toHaveLength(0)
  })

  it('open to the individual observe rows', async () => {
    await act(() => root.render(<MessageList messages={batch} />))
    const toggle = runs()[0].querySelector<HTMLButtonElement>(
      '[data-message-action="toggle-observe-run"]',
    )
    expect(toggle?.getAttribute('aria-expanded')).toBe('false')
    await act(() => toggle?.click())
    expect(toggle?.getAttribute('aria-expanded')).toBe('true')
    expect(observeRows()).toHaveLength(10)
    expect(container.querySelector('[data-message-row~="o6"]')).not.toBeNull()
  })

  it('leave a single observe as its own row', async () => {
    await act(() =>
      root.render(
        <MessageList
          messages={[
            observe('o1', 'session::on-deleted'),
            call('c1', 'engine::functions::list'),
            observe('o2', 'secrets::on-changed'),
          ]}
          defaultOpenCalls
        />,
      ),
    )
    expect(runs()).toHaveLength(0)
    expect(observeRows()).toHaveLength(2)
  })

  it('open when a landing targets one of them', async () => {
    await act(() =>
      root.render(<MessageList messages={batch} focusMessageId="o7" />),
    )
    expect(runs()[0].dataset.expanded).toBe('true')
    expect(container.querySelector('[data-message-row~="o7"]')).not.toBeNull()
  })

  it('show the run running while any observe still runs', async () => {
    await act(() =>
      root.render(
        <MessageList
          messages={[
            observe('o1', 'session::on-deleted'),
            observe('o2', 'secrets::on-changed', {
              running: true,
              output: undefined,
            }),
          ]}
        />,
      ),
    )
    expect(runs()[0].dataset.functionStatus).toBe('running')
  })
})
