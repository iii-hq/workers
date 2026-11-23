import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import type { Host } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { towerFixture } from '../../src/fixtures'
import type { TowerData, TowerQuestion } from '../../src/types'
import { mergeHolds, useTowerOverview, type TowerOverviewData } from './useTowerOverview'

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
  let recent: TowerQuestion[] = []
  const data: TowerData = {
    ...towerFixture,
    as_of: new Date().toISOString(),
    sourceHash: 'fixture',
    items: towerFixture.items
      .filter((item) => item.kind !== 'held')
      .map((item) => ({
        ...item,
        urgency:
          item.kind === 'stalled' || (item.kind === 'question' && item.questionClass === 'security')
            ? ('now' as const)
            : ('no-rush' as const),
      })),
    runs: towerFixture.runs.map((run) => ({ ...run, context_size: 1234 })),
    prs: towerFixture.prs.map((pr) => ({ ...pr, as_of: new Date().toISOString() })),
    direction: [],
    exceptions: { stalled: [], failing_prs: [], blocked_cards: [], old_questions: [] },
    median_decision_wait_ms: 9000,
  }
  const handlers = new Map<string, (payload: unknown) => void>()
  const disposed = vi.fn()
  const call = vi.fn(async (id: string, input: Record<string, unknown> = {}): Promise<unknown> => {
    if (id === 'tower::overview')
      return {
        ...data,
        summary: {
          result: null,
          valid_hash: null,
          checked_at: null,
          error: 'set the summary model in the tower settings',
          unavailable: true,
          fresh: false,
        },
        availability: { github: true, approval_gate: true },
        prs_interval_min: 7,
      }
    if (id === 'tower::needs-you::list') return { items: data.items }
    if (id === 'tower::runs::list') return { runs: data.runs, error: null }
    if (id === 'tower::prs::list') return { prs: data.prs, error: null }
    if (id === 'tower::decided::list') return { questions: [], median_decision_wait_ms: 9000 }
    if (id === 'tower::assumptions::list')
      return { questions: input.since ? recent : data.questions.filter((row) => row.state === 'recorded') }
    if (id === 'tower::watch') return { watched_until: new Date(Date.now() + 120_000).toISOString() }
    if (id === 'tower::summary::refresh')
      return {
        result: null,
        valid_hash: null,
        checked_at: null,
        error: null,
        unavailable: false,
        throttled_until: '2026-11-22T18:00:00Z',
      }
    if (id === 'approval::list-pending')
      return {
        pending: [
          {
            session_id: 'dispatch-r1',
            function_call_id: 'held-a',
            function_id: 'github::push',
            arguments_excerpt: { ref: '[redacted]' },
            pending_at: Date.now(),
            session_metadata: { kanban_ticket: data.tickets[0].key },
          },
        ],
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
  let result: TowerOverviewData
  function Probe({ scope = 'all', since }: { scope?: string; since?: string }) {
    result = useTowerOverview(host, scope, since)
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
    setRecent: (rows: TowerQuestion[]) => {
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

describe('useTowerOverview console boundary and live parts', () => {
  it('loads worker facts and only reads approval holds directly, with no git remote or metrics calls', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    expect(f.result().items.some((item) => item.kind === 'held')).toBe(true)
    expect(f.result().items.find((item) => item.kind === 'held')).toMatchObject({
      arguments: { ref: '[redacted]' },
      urgency: 'now',
    })
    expect(f.result().prs_interval_min).toBe(7)
    expect(f.call).toHaveBeenCalledWith('approval::list-pending', { limit: 200 })
    expect(f.call.mock.calls.every(([id]) => id.startsWith('tower::') || id === 'approval::list-pending')).toBe(true)
    expect(f.observe.mock.calls.map(([source]) => source)).toEqual([
      'tower::on-changed',
      'approval::on-pending-created',
      'approval::on-pending-resolved',
    ])
    expect(f.result().summary.error).toBe('set the summary model in the tower settings')
  })
  it('refetches just the changed part for the matching scope and keeps one subscription on Refresh', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    f.call.mockClear()
    const changed = [...f.handlers.entries()].find(([id]) => !id.includes('holds'))![1]
    await act(() => changed({ scope: 'other', parts: ['prs'] }))
    expect(f.call).not.toHaveBeenCalled()
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    expect(f.call.mock.calls.map(([id]) => id)).toEqual(['tower::prs::list'])
    await act(() => f.result().refresh())
    expect(f.observe).toHaveBeenCalledTimes(3)
  })
  it('refreshes held calls on both approval events, renews watch each minute, and disposes on unmount', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    f.call.mockClear()
    const changed = [...f.handlers.entries()].find(([id]) => id.includes('holds'))![1]
    await act(() => changed({}))
    expect(f.call.mock.calls.map(([id]) => id)).toEqual(['approval::list-pending'])
    f.call.mockClear()
    await act(() => vi.advanceTimersByTime(60_000))
    expect(f.call.mock.calls.map(([id]) => id)).toEqual(['tower::watch'])
    await act(() => root.unmount())
    expect(f.disposed).toHaveBeenCalledTimes(6)
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
    expect(f.call).toHaveBeenCalledWith('tower::assumptions::list', { scope: 'all', since })
    expect(f.result().assumptionsSinceVisit).toEqual([recent])
    await act(() => f.result().summary.refresh())
    expect(f.call).toHaveBeenCalledWith('tower::summary::refresh', { scope: 'all' })
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
    const changed = [...f.handlers.entries()].find(([id]) => !id.includes('holds'))![1]
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
    const earlier = await f.call('tower::overview', { scope: 'all' })
    let finish!: (value: unknown) => void
    f.call.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        }),
    )
    await act(() => f.result().refresh())
    f.data.prs = []
    const changed = [...f.handlers.entries()].find(([id]) => !id.includes('holds'))![1]
    await act(() => changed({ scope: 'all', parts: ['prs'] }))
    expect(f.result().prs).toEqual([])
    await act(() => finish(earlier))
    expect(f.result().prs).toEqual([])
  })
  it('retains an approval read error through unrelated successful worker reads', async () => {
    const f = fake()
    await act(() => root.render(<f.Probe />))
    const original = f.call.getMockImplementation()!
    f.call.mockImplementation(async (id, input) => {
      if (id === 'approval::list-pending') throw new Error('Approval source down')
      return original(id, input)
    })
    const holdsChanged = [...f.handlers.entries()].find(([id]) => id.includes('holds'))![1]
    const changed = [...f.handlers.entries()].find(([id]) => !id.includes('holds'))![1]
    await act(() => holdsChanged({}))
    expect(f.result().errors.needsYou).toBe('Approval source down')
    f.call.mockImplementation(original)
    await act(() => holdsChanged({}))
    expect(f.result().errors.needsYou).toBeNull()
    f.call.mockImplementation(async (id, input) => {
      if (id === 'approval::list-pending') throw new Error('Approval source down')
      return original(id, input)
    })
    await act(() => holdsChanged({}))
    await act(() => changed({ scope: 'all', parts: ['items'] }))
    expect(f.result().errors.needsYou).toBe('Approval source down')
    await act(() => changed({ scope: 'all', parts: ['direction'] }))
    expect(f.result().errors.needsYou).toBe('Approval source down')
    f.call.mockImplementation(original)
    await act(() => holdsChanged({}))
    expect(f.result().errors.needsYou).toBeNull()
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
    expect(f.call).toHaveBeenCalledWith('tower::overview', { scope: 'all' })
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
  it('keeps Needs you loading until the page approval read finishes', async () => {
    const f = fake()
    const original = f.call.getMockImplementation()!
    let finish!: (value: unknown) => void
    f.call.mockImplementation((id, input) =>
      id === 'approval::list-pending'
        ? new Promise((resolve) => {
            finish = resolve
          })
        : original(id, input),
    )
    await act(() => root.render(<f.Probe />))
    expect(f.result().loading.needsYou).toBe(true)
    await act(() => finish({ pending: [] }))
    expect(f.result().loading.needsYou).toBe(false)
  })
  it('marks parked or done roots with their own held call as waiting', () => {
    const f = fake()
    const run = f.data.runs[0]
    const pending = [
      {
        session_id: run.session.session_id,
        function_call_id: 'hold',
        function_id: 'f',
        arguments_excerpt: {},
        pending_at: 0,
      },
    ]
    const waiting = mergeHolds({ ...f.data, runs: [{ ...run, state: 'done' }] }, pending, 'all')
    expect(waiting.runs[0].state).toBe('waiting')
    expect(waiting.runs[0].needsYou.some((row) => row.kind === 'held')).toBe(true)
    const parked = {
      ...run,
      state: 'working' as const,
      status: { ...run.status, status: 'awaiting_functions' },
    }
    const parkedData = { ...f.data, runs: [parked] }
    expect(mergeHolds(parkedData, pending, 'all').runs[0].state).toBe('waiting')
    expect(mergeHolds(parkedData, [], 'all').runs[0].state).toBe('working')
    const active = { ...parked, status: { ...parked.status, status: 'running' } }
    expect(mergeHolds({ ...f.data, runs: [active] }, pending, 'all').runs[0].state).toBe('working')
    const stalled = mergeHolds({ ...f.data, runs: [{ ...run, state: 'stalled' }] }, pending, 'all')
    expect(stalled.runs[0].state).toBe('stalled')
  })
  it('keeps held calls scoped and preserves independently reported stalled items', () => {
    const f = fake()
    const stalled = f.data.runs[0]
    const data = {
      ...f.data,
      items: [
        {
          id: 'stalled',
          kind: 'stalled',
          urgency: 'now',
          sessionId: stalled.session.session_id,
          title: stalled.session.title,
          blockedCards: 0,
          createdAt: 0,
          lastProgress: 0,
          armedWakes: [],
        },
      ],
    } as TowerData
    const pending = [
      {
        session_id: stalled.session.session_id,
        function_call_id: 'held',
        function_id: 'f',
        arguments_excerpt: {},
        pending_at: 0,
        session_metadata: { kanban_ticket: f.data.tickets[0].key },
      },
    ]
    expect(mergeHolds(data, pending, 'all').items.map((row) => row.kind)).toEqual(['held', 'stalled'])
    expect(mergeHolds(data, pending, 'another-project').items.map((row) => row.kind)).toEqual(['stalled'])
  })
})
