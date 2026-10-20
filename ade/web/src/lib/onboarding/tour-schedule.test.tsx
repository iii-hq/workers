// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const harness = vi.hoisted(() => ({
  prepare: vi.fn(async () => undefined),
  open: vi.fn(),
  onTurn: null as ((payload: unknown) => void) | null,
  off: vi.fn(),
  config: null as Record<string, unknown> | null,
  /** What `harness::status` reads for the example chat. */
  status: null as null | {
    status: string
    turn_id: string | null
    expects_wake: boolean
  },
  calls: [] as string[],
  trigger: vi.fn(async (_fn: string, _payload: unknown) => ({ closed: [] })),
}))

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger: harness.trigger }),
}))
vi.mock('@/lib/backend/harness-send', () => ({
  getTurnStatus: async () => harness.status,
  isTurnActive: (status: string) =>
    status === 'running' || status === 'awaiting_functions',
}))

vi.mock('./tour', () => ({
  prepareTour: harness.prepare,
  openTour: harness.open,
}))
vi.mock('@/lib/engine-trigger', () => ({
  subscribeEngineTrigger: async (
    _type: string,
    config: Record<string, unknown>,
    onEvent: (payload: unknown) => void,
  ) => {
    harness.config = config
    harness.onTurn = onEvent
    return harness.off
  },
}))

import {
  clearTourSchedule,
  readTourSchedule,
  scheduleTour,
  TOUR_DELAY_MS,
  useScheduledTour,
} from './tour-schedule'

function Host() {
  useScheduledTour()
  return null
}

let root: Root | null = null

function render() {
  root = createRoot(document.createElement('div'))
  act(() => root?.render(<Host />))
}

/** Let the subscription and the prepare/open chain settle. */
async function flush() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

beforeEach(() => {
  harness.status = null
  harness.calls = []
  harness.trigger.mockClear()
  harness.trigger.mockImplementation(async (fn: string) => {
    harness.calls.push(fn)
    return { closed: [] }
  })
  harness.open.mockImplementation(() => {
    harness.calls.push('open')
  })
  vi.useFakeTimers()
  harness.prepare.mockClear()
  harness.open.mockClear()
  harness.off.mockClear()
  harness.onTurn = null
  harness.config = null
  clearTourSchedule()
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  vi.useRealTimers()
})

describe('useScheduledTour', () => {
  it('opens the tour when the example chat’s turn ends', async () => {
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    expect(harness.config).toEqual({ session_id: 'console-1' })

    act(() =>
      harness.onTurn?.({ session_id: 'console-1', status: 'completed' }),
    )
    await flush()
    expect(harness.open).toHaveBeenCalledOnce()
    expect(readTourSchedule()).toBeNull()
    expect(harness.off).toHaveBeenCalled()
    // The answer is reported before the tour opens, so its first step is
    // already closed when the card loads.
    expect(harness.trigger).toHaveBeenCalledWith('onboarding::steps::observe', {
      trigger_type: 'harness::turn-completed',
      payload: { session_id: 'console-1', status: 'completed' },
    })
    expect(harness.calls).toEqual(['onboarding::steps::observe', 'open'])
  })

  it('opens without closing the first step when the turn failed', async () => {
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    act(() => harness.onTurn?.({ session_id: 'console-1', status: 'failed' }))
    await flush()
    expect(harness.open).toHaveBeenCalledOnce()
    expect(harness.trigger).not.toHaveBeenCalled()
  })

  it('catches a turn that ended before the subscription landed', async () => {
    harness.status = {
      status: 'completed',
      turn_id: 't-1',
      expects_wake: false,
    }
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    await flush()
    expect(harness.open).toHaveBeenCalledOnce()
    expect(harness.trigger).toHaveBeenCalledWith(
      'onboarding::steps::observe',
      expect.objectContaining({
        payload: expect.objectContaining({
          session_id: 'console-1',
          turn_id: 't-1',
          status: 'completed',
        }),
      }),
    )
  })

  it('keeps waiting while the example is still answering', async () => {
    harness.status = { status: 'running', turn_id: 't-1', expects_wake: false }
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })

  it('waits past a turn that hands over to a later one', async () => {
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    act(() => harness.onTurn?.({ session_id: 'console-1', terminal: false }))
    act(() => harness.onTurn?.({ session_id: 'console-2' }))
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })

  it('opens the tour after the delay when no answer comes', async () => {
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    act(() => vi.advanceTimersByTime(TOUR_DELAY_MS - 1))
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
    act(() => vi.advanceTimersByTime(1))
    await flush()
    expect(harness.open).toHaveBeenCalledOnce()
    // Nothing answered, so nothing is reported: step one still asks.
    expect(harness.trigger).not.toHaveBeenCalled()
  })

  it('opens at once for a schedule that came due while the page was away', async () => {
    scheduleTour('console-1', Date.now() - TOUR_DELAY_MS - 1)
    render()
    act(() => vi.advanceTimersByTime(0))
    await flush()
    expect(harness.open).toHaveBeenCalledOnce()
  })

  it('does nothing once the schedule is cleared', async () => {
    render()
    act(() => scheduleTour('console-1'))
    await flush()
    act(() => clearTourSchedule())
    act(() => vi.advanceTimersByTime(TOUR_DELAY_MS))
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })
})
