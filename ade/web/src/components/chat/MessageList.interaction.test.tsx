// @vitest-environment jsdom

import { act, StrictMode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FunctionTriggerMessage } from '@/types/chat'
import { MessageList } from './MessageList'

const placeholder: FunctionTriggerMessage = {
  id: 'e_assistant:2',
  role: 'function-trigger',
  functionTriggerId: 'call-list',
  functionId: 'engine::functions::list',
  input: undefined,
  createdAt: 0,
  durationMs: 10,
  unloaded: true,
  resultEntryId: 'e_result',
}
const hydrated: FunctionTriggerMessage = {
  ...placeholder,
  unloaded: false,
  input: { worker: 'code-runner' },
  output: {
    functions: [
      { function_id: 'code-runner::run', worker_name: 'code-runner' },
    ],
  },
}
const title = '[data-message-action="toggle"]'
const caret = '[data-message-action="toggle-caret"]'
const skeleton = '[data-function-trigger-skeleton]'
const failed = '[data-function-trigger-load-failed]'
const retry = '[data-message-action="retry-load"]'
const card = '[data-message-role="function-call"]'

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

async function click(selector: string) {
  const button = container.querySelector<HTMLButtonElement>(selector)
  expect(button).not.toBeNull()
  await act(() => button?.click())
}

/** Let a loader's promise settle and the card commit its outcome. */
const settle = () =>
  act(() => new Promise<void>((resolve) => setTimeout(resolve, 0)))

async function render(
  messages: FunctionTriggerMessage[],
  onLoadActivityEntries?: (ids: string[]) => Promise<boolean> | undefined,
  defaultOpenCalls = false,
) {
  await act(() => {
    root.render(
      <StrictMode>
        <MessageList
          messages={messages}
          onLoadActivityEntries={onLoadActivityEntries}
          defaultOpenCalls={defaultOpenCalls}
        />
      </StrictMode>,
    )
  })
}

describe('historical call details hydration', () => {
  it.each([title, caret])('loads a single call through %s', async (toggle) => {
    const load = vi.fn()
    await render([placeholder], load)
    expect(load).not.toHaveBeenCalled()
    expect(container.querySelector(skeleton)).toBeNull()

    await click(toggle)
    expect(load).toHaveBeenCalledExactlyOnceWith(['e_assistant', 'e_result'])
    expect(container.querySelector(skeleton)).not.toBeNull()

    // The entry upsert can change the block index; call identity stays stable.
    await render([{ ...hydrated, id: 'e_assistant:3' }], load)
    expect(container.querySelector(skeleton)).toBeNull()
    expect(container.textContent).toContain('code-runner::run')
    expect(load).toHaveBeenCalledTimes(1)
  })

  it('loads default-open cards once and retries only after closing and reopening', async () => {
    const load = vi.fn()
    await render([placeholder], load, true)
    expect(load).toHaveBeenCalledExactlyOnceWith(['e_assistant', 'e_result'])

    // New parent callbacks must not cause a failed load to loop on rerenders.
    await render([{ ...placeholder }], (ids) => load(ids), true)
    expect(load).toHaveBeenCalledTimes(1)
    await click(title)
    expect(load).toHaveBeenCalledTimes(1)
    await click(caret)
    expect(load).toHaveBeenCalledTimes(2)
  })

  it('does not ask again when the block index shifts before the result lands', async () => {
    const load = vi.fn()
    await render([placeholder], load, true)
    expect(load).toHaveBeenCalledTimes(1)
    // The call entry hydrated (re-deriving the id) but its result is still
    // on the way: the open card keeps waiting instead of asking again.
    await render([{ ...placeholder, id: 'e_assistant:3' }], load, true)
    expect(load).toHaveBeenCalledTimes(1)
    expect(container.querySelector(skeleton)).not.toBeNull()
  })

  it.each([{ pendingApproval: true }, { running: true }])(
    'never hydrates a live placeholder %o',
    async (flags) => {
      const load = vi.fn()
      await render([{ ...placeholder, ...flags }], load, true)
      expect(load).not.toHaveBeenCalled()
      expect(container.querySelector(skeleton)).toBeNull()
    },
  )

  it('only loads the opened call, leaving hidden history lazy', async () => {
    const load = vi.fn()
    const hidden = {
      ...placeholder,
      id: 'e_older:0',
      functionTriggerId: 'older-call',
      resultEntryId: 'e_older_result',
    }
    await render([hidden, placeholder], load)
    expect(load).not.toHaveBeenCalled()
    // Collapsed: only the latest call is mounted; the older one waits.
    expect(container.querySelectorAll(card)).toHaveLength(1)
    await click(title)
    expect(load).toHaveBeenCalledExactlyOnceWith(['e_assistant', 'e_result'])
  })

  it('does not request a loaded call, then asks once if it is elided again', async () => {
    const load = vi.fn()
    await render([hydrated], load, true)
    expect(load).not.toHaveBeenCalled()
    // A reconnect re-read elided the open row: hydrate it again, once.
    await render([placeholder], load, true)
    expect(load).toHaveBeenCalledExactlyOnceWith(['e_assistant', 'e_result'])
  })

  it('renders the skeleton without a loader on surfaces that have none', async () => {
    await render([placeholder], undefined, true)
    expect(container.querySelector(skeleton)).not.toBeNull()
  })

  it('swaps the skeleton for a retry row when the read fails', async () => {
    const load = vi
      .fn<(ids: string[]) => Promise<boolean>>()
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true)
    await render([placeholder], load, true)
    await settle()
    expect(load).toHaveBeenCalledTimes(1)
    expect(container.querySelector(failed)).not.toBeNull()
    expect(container.querySelector(skeleton)).toBeNull()

    await click(retry)
    await settle()
    expect(load).toHaveBeenCalledTimes(2)
    expect(container.querySelector(failed)).toBeNull()
    expect(container.querySelector(skeleton)).not.toBeNull()
  })

  it('drops a failure that lands after the card closed', async () => {
    let fail: (ok: boolean) => void = () => {}
    const load = vi.fn(
      () => new Promise<boolean>((resolve) => (fail = resolve)),
    )
    await render([placeholder], load, true)
    await click(title)
    fail(false)
    await settle()
    await click(title)
    await settle()
    expect(load).toHaveBeenCalledTimes(2)
    expect(container.querySelector(failed)).toBeNull()
    expect(container.querySelector(skeleton)).not.toBeNull()
  })
})
