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
  input: {},
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
const trailing =
  '[data-message-role="function-call"] > div > button[aria-hidden="true"]'
const skeleton = '[data-function-trigger-skeleton]'

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

async function render(
  messages: FunctionTriggerMessage[],
  onLoadActivityEntries?: (ids: string[]) => void,
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
  it.each([title, trailing])(
    'loads a single call through %s',
    async (toggle) => {
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
    },
  )

  it('loads default-open cards once and retries only after closing and reopening', async () => {
    const load = vi.fn()
    await render([placeholder], load, true)
    expect(load).toHaveBeenCalledExactlyOnceWith(['e_assistant', 'e_result'])

    // New parent callbacks must not cause a failed load to loop on rerenders.
    await render([{ ...placeholder }], (ids) => load(ids), true)
    expect(load).toHaveBeenCalledTimes(1)
    await click(title)
    expect(load).toHaveBeenCalledTimes(1)
    await click(trailing)
    expect(load).toHaveBeenCalledTimes(2)
  })

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
    await click(title)
    expect(load).toHaveBeenCalledExactlyOnceWith(['e_assistant', 'e_result'])
  })

  it('does not request loaded calls or require a loader on other surfaces', async () => {
    const load = vi.fn()
    await render([hydrated], load, true)
    expect(load).not.toHaveBeenCalled()
    await render([placeholder], undefined, true)
    expect(container.querySelector(skeleton)).not.toBeNull()
  })
})
