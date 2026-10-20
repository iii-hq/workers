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
  mark: vi.fn(async () => undefined),
  state: null as Record<string, unknown> | null,
  automated: false,
}))

vi.mock('./api', () => ({
  fetchOnboardingState: async () => harness.state,
  markTourOpened: harness.mark,
}))
vi.mock('./open', () => ({ browserIsAutomated: () => harness.automated }))
vi.mock('./tour', () => ({
  prepareTour: harness.prepare,
  openTour: harness.open,
}))

import { TOUR_AFTER_MS, tourDueAt, useTourGate } from './tour-gate'

let root: Root | null = null

function Host({ wizardOpen = false }: { wizardOpen?: boolean }) {
  useTourGate(wizardOpen, true)
  return null
}

function render(wizardOpen = false) {
  root = createRoot(document.createElement('div'))
  act(() => root?.render(<Host wizardOpen={wizardOpen} />))
}

/** Let the state read and the prepare/open chain settle. */
async function flush() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(1_000_000)
  harness.prepare.mockClear()
  harness.open.mockClear()
  harness.mark.mockClear()
  harness.automated = false
  harness.state = { status: 'completed', setup_ended_at: 1_000_000 }
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  vi.useRealTimers()
})

describe('tourDueAt', () => {
  it('is five minutes after the first open', () => {
    expect(
      tourDueAt({ status: 'new', updated_at: 0, setup_ended_at: 10 }, false),
    ).toBe(10 + TOUR_AFTER_MS)
  })

  it('is never due once the tour has opened, without a first open, or where nothing opens by itself', () => {
    const base = {
      status: 'completed' as const,
      updated_at: 0,
      setup_ended_at: 10,
    }
    expect(tourDueAt({ ...base, tour_opened_at: 20 }, false)).toBeNull()
    expect(tourDueAt({ status: 'completed', updated_at: 0 }, false)).toBeNull()
    expect(tourDueAt({ ...base, auto_open: false }, false)).toBeNull()
    expect(tourDueAt(base, true)).toBeNull()
    expect(tourDueAt(null, false)).toBeNull()
  })
})

describe('useTourGate', () => {
  it('opens the tour five minutes after the first open, and records that it did', async () => {
    render()
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
    await act(async () => {
      vi.advanceTimersByTime(TOUR_AFTER_MS - 1)
    })
    expect(harness.open).not.toHaveBeenCalled()
    await act(async () => {
      vi.advanceTimersByTime(1)
    })
    await flush()
    expect(harness.prepare).toHaveBeenCalledTimes(1)
    expect(harness.open).toHaveBeenCalledTimes(1)
    expect(harness.mark).toHaveBeenCalledTimes(1)
  })

  it('opens at once on a load past the mark (a restart, a reload)', async () => {
    harness.state = {
      status: 'completed',
      setup_ended_at: 1_000_000 - TOUR_AFTER_MS - 5,
    }
    render()
    await flush()
    await act(async () => {
      vi.advanceTimersByTime(0)
    })
    await flush()
    expect(harness.open).toHaveBeenCalledTimes(1)
  })

  it('waits for the wizard to close', async () => {
    harness.state = { status: 'new', setup_ended_at: 0 }
    render(true)
    await flush()
    await act(async () => {
      vi.advanceTimersByTime(TOUR_AFTER_MS)
    })
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
    act(() => root?.render(<Host wizardOpen={false} />))
    await flush()
    await act(async () => {
      vi.advanceTimersByTime(0)
    })
    await flush()
    expect(harness.open).toHaveBeenCalledTimes(1)
  })

  it('stays quiet once the tour has opened, and under automation', async () => {
    harness.state = {
      status: 'completed',
      setup_ended_at: 0,
      tour_opened_at: 5,
    }
    render()
    await flush()
    await act(async () => {
      vi.advanceTimersByTime(TOUR_AFTER_MS)
    })
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
    act(() => root?.unmount())
    harness.state = { status: 'completed', setup_ended_at: 0 }
    harness.automated = true
    render()
    await flush()
    await act(async () => {
      vi.advanceTimersByTime(TOUR_AFTER_MS)
    })
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })

  it('forgets a pending open when unmounted before it is due', async () => {
    render()
    await flush()
    act(() => root?.unmount())
    root = null
    await act(async () => {
      vi.advanceTimersByTime(TOUR_AFTER_MS)
    })
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })
})
