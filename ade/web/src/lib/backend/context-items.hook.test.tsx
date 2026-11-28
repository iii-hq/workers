// @vitest-environment jsdom

import { act, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  on: vi.fn(() => () => {}),
  observe: vi.fn(() => () => {}),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () =>
    Promise.resolve({
      call: mocks.call,
      on: mocks.on,
      observe: mocks.observe,
      browserId: 'tab',
    }),
}))

import {
  MESSAGE_DEBOUNCE_MS,
  PR_REFRESH_MS,
  useContextItems,
} from './context-items'

type Controls = {
  setVisible: (visible: boolean) => void
  setTick: (tick: number) => void
}

function mount(enabled = true) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  const controls = {} as Controls
  let latest: ReturnType<typeof useContextItems> | undefined
  function Probe() {
    const [visible, setVisible] = useState(false)
    const [tick, setTick] = useState(0)
    controls.setVisible = setVisible
    controls.setTick = setTick
    latest = useContextItems('s1', { enabled, visible, messageTick: tick })
    return null
  }
  act(() => root.render(<Probe />))
  return {
    controls,
    get current() {
      return latest as ReturnType<typeof useContextItems>
    },
    unmount: () => {
      act(() => root.unmount())
      container.remove()
    },
  }
}

const reads = () =>
  mocks.call.mock.calls.filter(([id]) => id === 'harness::context-items').length

async function flush() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

beforeEach(() => {
  vi.useFakeTimers()
  mocks.call.mockReset()
  mocks.call.mockResolvedValue({ session_id: 's1', links: [] })
  mocks.observe.mockClear()
})
afterEach(() => {
  vi.useRealTimers()
})

describe('useContextItems refresh rules (§1.2.4)', () => {
  it('reads when the conversation opens, for the badge', async () => {
    const hook = mount()
    await flush()
    expect(reads()).toBe(1)
    expect(mocks.call).toHaveBeenCalledWith('harness::context-items', {
      session_id: 's1',
    })
    expect(hook.current.items).not.toBeNull()
    hook.unmount()
  })

  it('watches harness::on-turn-completed for this session', async () => {
    const hook = mount()
    await flush()
    expect(mocks.observe).toHaveBeenCalledWith(
      {
        function_id: 'harness::on-turn-completed',
        filters: { 'input.session_id': { $eq: 's1' } },
      },
      expect.stringContaining('ade::context-items-turn'),
    )
    hook.unmount()
  })

  it('reads again when the pane opens, and on new messages only while visible (2 s)', async () => {
    const hook = mount()
    await flush()
    act(() => hook.controls.setTick(1))
    await act(async () => {
      vi.advanceTimersByTime(MESSAGE_DEBOUNCE_MS)
    })
    expect(reads()).toBe(1)

    act(() => hook.controls.setVisible(true))
    await flush()
    expect(reads()).toBe(2)

    act(() => hook.controls.setTick(2))
    act(() => hook.controls.setTick(3))
    await act(async () => {
      vi.advanceTimersByTime(MESSAGE_DEBOUNCE_MS - 1)
    })
    expect(reads()).toBe(2)
    await act(async () => {
      vi.advanceTimersByTime(1)
    })
    expect(reads()).toBe(3)

    // Pull request states every 2 minutes while the pane is visible.
    await act(async () => {
      vi.advanceTimersByTime(PR_REFRESH_MS)
    })
    expect(reads()).toBe(4)

    // Closing the pane reads nothing.
    act(() => hook.controls.setVisible(false))
    await act(async () => {
      vi.advanceTimersByTime(PR_REFRESH_MS)
    })
    expect(reads()).toBe(4)
    hook.unmount()
  })

  it('calls nothing on an engine without harness::context-items', async () => {
    const hook = mount(false)
    await flush()
    expect(reads()).toBe(0)
    expect(hook.current.items).toBeNull()
    hook.unmount()
  })
})
