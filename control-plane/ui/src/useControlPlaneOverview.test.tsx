import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import type { Host } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { controlPlaneFixture } from '../../src/fixtures'
import type { ControlPlaneData, ControlPlaneQuestion } from '../../src/types'
import { useControlPlaneOverview, type ControlPlaneOverviewData } from './useControlPlaneOverview'

let root: Root
let container: HTMLDivElement
beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})
afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

function fake() {
  let recent: ControlPlaneQuestion[] = []
  const data: ControlPlaneData = {
    ...controlPlaneFixture,
    as_of: new Date().toISOString(),
    sourceHash: 'fixture',
    items: controlPlaneFixture.items.map((item) => ({
        ...item,
        urgency:
          item.kind === 'stalled' || (item.kind === 'question' && item.questionClass === 'security')
            ? ('now' as const)
            : ('no-rush' as const),
    })),
    runs: controlPlaneFixture.runs.map((run) => ({ ...run, context_size: 1234 })),
    prs: controlPlaneFixture.prs.map((pr) => ({ ...pr, as_of: new Date().toISOString() })),
    direction: [],
    exceptions: { stalled: [], failing_prs: [], blocked_cards: [], old_questions: [] },
    median_decision_wait_ms: 9000,
  }
  const handlers = new Map<string, (payload: unknown) => void>()
  const disposed = vi.fn()
  const call = vi.fn(async (id: string, input: Record<string, unknown> = {}): Promise<unknown> => {
    if (id === 'control-plane::overview')
      return {
        ...data,
        summary: {
          result: null,
          valid_hash: null,
          checked_at: null,
          error: 'set the summary model in the Control plane settings',
          unavailable: true,
          fresh: false,
        },
        availability: { github: true },
        prs_interval_min: 7,
      }
    if (id === 'control-plane::needs-you::list') return { items: data.items }
    if (id === 'control-plane::runs::list') return { runs: data.runs, error: null }
    if (id === 'control-plane::prs::list') return { prs: data.prs, error: null }
    if (id === 'control-plane::decided::list') return { questions: [], median_decision_wait_ms: 9000 }
    if (id === 'control-plane::assumptions::list')
      return { questions: input.since ? recent : data.questions.filter((row) => row.state === 'recorded') }
    if (id === 'control-plane::watch') return { watched_until: new Date(Date.now() + 120_000).toISOString() }
    if (id === 'control-plane::summary::refresh')
      return {
        result: null,
        valid_hash: null,
        checked_at: null,
        error: null,
        unavailable: false,
        throttled_until: '2026-11-22T18:00:00Z',
      }
    throw new Error(`Unexpected direct source read ${id}`)
  })
  const observe = vi.fn((_source: string, _target: string) => disposed)
  const connections: Array<(state: unknown) => void> = []
  const iii = {
    browserId: 'test-browser',
    call,
    observe,
    on: vi.fn((id, handler) => {
      handlers.set(id, handler)
      return () => {
        handlers.delete(id)
        disposed()
      }
    }),
    addConnectionStateListener: vi.fn((handler: (state: unknown) => void) => {
      connections.push(handler)
      return disposed
    }),
  } as unknown as Host['iii']
  const host = { iii } as Host
  let result: ControlPlaneOverviewData
  function Probe({ scope = 'all', since }: { scope?: string; since?: string }) {
    result = useControlPlaneOverview(host, scope, since)
    return null
  }
  return {
    data,
    call,
    observe,
    handlers,
    disposed,
    connections,
    Probe,
    result: () => result!,
    setRecent: (rows: ControlPlaneQuestion[]) => {
      recent = rows
    },
  }
}
async function flush() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

describe('useControlPlaneOverview console boundary and live parts', () => {
  it('loads worker facts without direct source reads', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    expect(f.result().prs_interval_min).toBe(7)
    expect(f.result().items).toEqual(f.data.items)
    expect(f.result().runs).toEqual(f.data.runs)
    expect(f.call.mock.calls.every(([id]) => id.startsWith('control-plane::'))).toBe(true)
    expect(f.observe.mock.calls.map(([source]) => source)).toEqual(['control-plane::on-changed'])
    expect(f.result().summary.error).toBe('set the summary model in the Control plane settings')
  })
  it('refetches just the changed part for the matching scope and keeps one subscription on Refresh', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    f.call.mockClear()
    const changed = [...f.handlers.values()][0]
    await act(() => changed({ scope: 'other', parts: ['prs'] }))
    expect(f.call).not.toHaveBeenCalled()
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    expect(f.call.mock.calls.map(([id]) => id)).toEqual(['control-plane::prs::list'])
    await act(() => f.result().refresh())
    expect(f.observe).toHaveBeenCalledOnce()
  })
  it('renews watch each minute and disposes on unmount', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    f.call.mockClear()
    await act(() => vi.advanceTimersByTime(60_000))
    expect(f.call.mock.calls.map(([id]) => id)).toEqual(['control-plane::watch'])
    await act(() => root.unmount())
    expect(f.disposed).toHaveBeenCalledTimes(3)
    root = createRoot(container)
    f.call.mockClear()
    await act(() => vi.advanceTimersByTime(120_000))
    expect(f.call).not.toHaveBeenCalled()
  })
  it('passes the last visit to the worker and exposes throttled_until from summary refresh', async () => {
    const f = fake()
    const recent = f.data.questions.find((row) => row.state === 'recorded')!
    f.setRecent([recent])
    const since = new Date(Date.now() - 3600_000).toISOString()
    await act(() => root.render(<f.Probe since={since} />))
    expect(f.call).toHaveBeenCalledWith('control-plane::assumptions::list', { scope: 'all', since })
    expect(f.result().assumptionsSinceVisit).toEqual([recent])
    await act(() => f.result().summary.refresh())
    expect(f.call).toHaveBeenCalledWith('control-plane::summary::refresh', { scope: 'all' })
    expect(f.result().summary.throttled_until).toBe('2026-11-22T18:00:00Z')
  })
  it('does not accept an older response after a newer part fetch or a scope change', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    let finish!: (value: unknown) => void
    f.call.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        }),
    )
    const changed = [...f.handlers.values()][0]
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    await act(() => finish({ prs: [], error: null }))
    expect(f.result().prs).toEqual(f.data.prs)
    f.call.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        }),
    )
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    await act(() => root.render(<f.Probe scope="assistente" />))
    await act(() => finish({ prs: [], error: null }))
    expect(f.result().scope).toBe('assistente')
    expect(f.result().prs).toEqual(f.data.prs)
    await flush()
  })
  it('preserves a newer PR part when an earlier overview arrives late', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    const earlier = await f.call('control-plane::overview', { scope: 'all' })
    let finish!: (value: unknown) => void
    f.call.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        }),
    )
    await act(() => f.result().refresh())
    f.data.prs = []
    const changed = [...f.handlers.values()][0]
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    expect(f.result().prs).toEqual([])
    await act(() => finish(earlier))
    expect(f.result().prs).toEqual([])
  })
  it('refetches on connected only and rejects future summary freshness', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    f.call.mockClear()
    await act(() => {
      f.connections[0]('disconnected')
      f.connections[0]('connecting')
    })
    expect(f.call).not.toHaveBeenCalled()
    await act(() => f.connections[0]('connected'))
    expect(f.call).toHaveBeenCalledWith('control-plane::overview', { scope: 'all' })
    f.call.mockResolvedValueOnce({
      result: null,
      checked_at: new Date(Date.now() + 60_000).toISOString(),
      error: null,
      unavailable: false,
      valid_hash: null,
    })
    await act(() => f.result().summary.refresh())
    expect(f.result().summary.fresh).toBe(false)
  })
})
