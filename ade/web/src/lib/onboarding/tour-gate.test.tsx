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

import { tourIsDue, useTourGate } from './tour-gate'

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
  harness.prepare.mockClear()
  harness.open.mockClear()
  harness.mark.mockClear()
  harness.automated = false
  harness.state = { status: 'completed', setup_ended_at: 1_000_000 }
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
})

describe('tourIsDue', () => {
  it('is due once setup has ended', () => {
    expect(
      tourIsDue({ status: 'new', updated_at: 0, setup_ended_at: 10 }, false),
    ).toBe(true)
  })

  it('is never due once the tour has opened, before setup ends, or where nothing opens by itself', () => {
    const base = {
      status: 'completed' as const,
      updated_at: 0,
      setup_ended_at: 10,
    }
    expect(tourIsDue({ ...base, tour_opened_at: 20 }, false)).toBe(false)
    expect(tourIsDue({ status: 'completed', updated_at: 0 }, false)).toBe(false)
    expect(tourIsDue({ ...base, auto_open: false }, false)).toBe(false)
    expect(tourIsDue(base, true)).toBe(false)
    expect(tourIsDue(null, false)).toBe(false)
  })
})

describe('useTourGate', () => {
  it('opens the tour as soon as setup has ended, and records that it did', async () => {
    render()
    await flush()
    expect(harness.prepare).toHaveBeenCalledTimes(1)
    expect(harness.open).toHaveBeenCalledTimes(1)
    expect(harness.mark).toHaveBeenCalledTimes(1)
  })

  it('waits for the wizard to close', async () => {
    harness.state = { status: 'new', setup_ended_at: 0 }
    render(true)
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
    act(() => root?.render(<Host wizardOpen={false} />))
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
    expect(harness.open).not.toHaveBeenCalled()
    act(() => root?.unmount())
    harness.state = { status: 'completed', setup_ended_at: 0 }
    harness.automated = true
    render()
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })

  it('opens nothing when unmounted before the state arrives', async () => {
    render()
    act(() => root?.unmount())
    root = null
    await flush()
    expect(harness.open).not.toHaveBeenCalled()
  })
})
