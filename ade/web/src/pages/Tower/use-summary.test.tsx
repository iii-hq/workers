// @vitest-environment jsdom
import { act, StrictMode, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { IiiClient, ObserveSource } from '@/lib/iii-client'
import { TowerScreen } from '.'
import type { TowerData } from './data'
import { towerFixture, towerSummaryFixture } from './fixtures'
import { SUMMARY_INTERVAL, SUMMARY_SCHEMA, WRITER_READS } from './summary'
import { useTowerSummary } from './use-summary'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

let root: Root
let container: HTMLDivElement
let call: ReturnType<typeof vi.fn>
let iii: IiiClient
let state: ReturnType<typeof useTowerSummary>
let status: {
  status: string
  turn_id: string
  result: unknown
  result_error?: string
} | null
let meta: {
  kind: string
  metadata: { origin: string; agent_profile?: { id: string } }
}
let handlers: Map<string, (event: unknown) => void>
let observe: ReturnType<typeof vi.fn>
let disposers: ReturnType<typeof vi.fn>[]

async function settle() {
  await act(async () => {
    for (let index = 0; index < 40; index++) await Promise.resolve()
  })
}
async function render(scope = 'all', data = towerFixture, strict = false) {
  // Keep the probe's identity stable across renders and scope changes.
  await act(() =>
    root.render(
      strict ? (
        <StrictMode>
          <SummaryProbe scope={scope} data={data} />
        </StrictMode>
      ) : (
        <SummaryProbe scope={scope} data={data} />
      ),
    ),
  )
  await settle()
}
function SummaryProbe({ scope, data }: { scope: string; data: TowerData }) {
  state = useTowerSummary(iii, scope, data)
  return <div>{state.result?.brief.text}</div>
}
function emit(event: Record<string, unknown>) {
  act(() => {
    for (const handler of handlers.values()) handler(event)
  })
}
async function flushEvent() {
  await act(async () => vi.advanceTimersByTimeAsync(2_000))
  await settle()
}
const sends = () => call.mock.calls.filter(([id]) => id === 'harness::send')

async function respond(id: string) {
  if (id === 'harness::status') return status
  if (id === 'session::ensure') return { meta }
  if (id === 'harness::send') {
    status = { status: 'running', turn_id: 'writer-turn', result: null }
    return { session_id: 'tower-all', turn_id: 'writer-turn' }
  }
  return {}
}

beforeEach(() => {
  vi.useFakeTimers()
  container = document.createElement('div')
  root = createRoot(container)
  handlers = new Map()
  disposers = []
  status = null
  meta = {
    kind: 'automation',
    metadata: { origin: 'tower', agent_profile: { id: 'tower-writer' } },
  }
  call = vi.fn(respond)
  observe = vi.fn((_source: ObserveSource, _target: string) => {
    const off = vi.fn()
    disposers.push(off)
    return off
  })
  iii = {
    browserId: 'writer-test',
    call: call as IiiClient['call'],
    on(id: string, handler: (event: unknown) => void) {
      handlers.set(id, handler)
      const off = vi.fn(() => handlers.delete(id))
      disposers.push(off)
      return off
    },
    observe,
  } as unknown as IiiClient
})
afterEach(async () => {
  await act(() => root.unmount())
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('tower writer lifecycle', () => {
  it('ensures the scoped automation, permits exactly its reads before sending, and starts the kit with a structured output', async () => {
    await render('assistente')
    expect(call).toHaveBeenCalledWith('session::ensure', {
      session_id: 'tower-assistente',
      title: 'tower-assistente',
      kind: 'automation',
      metadata: { origin: 'tower' },
    })
    expect(call).toHaveBeenCalledWith('approval::set-mode', {
      session_id: 'tower-assistente',
      mode: 'auto',
    })
    expect(
      call.mock.calls
        .filter(([id]) => id === 'approval::add-always-allow')
        .map(([, payload]) => payload),
    ).toEqual(
      WRITER_READS.map((function_id) => ({
        session_id: 'tower-assistente',
        function_id,
      })),
    )
    const payload = sends()[0][1]
    expect(payload).toMatchObject({
      session_id: 'tower-assistente',
      idempotency_key: `tower:assistente:${Math.floor(Date.now() / SUMMARY_INTERVAL)}`,
      options: {
        agent: 'tower-writer',
        functions: { allow: WRITER_READS },
        output: { type: 'json', schema: SUMMARY_SCHEMA },
      },
    })
    expect(JSON.parse(payload.message)).toEqual({
      checked_at: null,
      refresh_at: new Date().toISOString(),
      project_id: 'assistente',
      repos: towerFixture.repositories,
      session_ids: towerFixture.runs.map((run) => run.session.session_id),
    })
    expect(
      call.mock.calls.findIndex(([id]) => id === 'harness::send'),
    ).toBeGreaterThan(
      call.mock.calls
        .map(([id]) => id)
        .lastIndexOf('approval::add-always-allow'),
    )
    expect(observe).toHaveBeenCalledWith(
      {
        function_id: 'harness::on-turn-completed',
        filters: { 'input.session_id': { $eq: 'tower-assistente' } },
      },
      expect.stringMatching(/::writer-test$/),
    )
  })

  it('restores a fresh verbose result, skips sending on open, and inherits the profile on Refresh', async () => {
    status = {
      status: 'completed',
      turn_id: 'existing',
      result: { ...towerSummaryFixture, unchanged: true },
    }
    await render()
    expect(state.result).toEqual(status.result)
    expect(sends()).toHaveLength(0)
    expect(call).toHaveBeenCalledWith('harness::status', {
      session_id: 'tower-all',
      verbose: true,
    })
    await act(() => state.refresh())
    await settle()
    expect(sends()).toHaveLength(1)
    expect(sends()[0][1].options).not.toHaveProperty('agent')
    expect(JSON.parse(sends()[0][1].message)).toMatchObject({
      checked_at: towerSummaryFixture.checked_at,
      project_id: null,
    })
  })

  it('serializes simultaneous tabs so only the first send selects a profile and both use the same slot', async () => {
    let pending = Promise.resolve()
    const request = vi.fn((_name: string, work: () => Promise<void>) => {
      const next = pending.then(work)
      pending = next
      return next
    })
    vi.stubGlobal('navigator', { locks: { request } })
    await act(() =>
      root.render(
        <>
          <SummaryProbe scope="all" data={towerFixture} />
          <SummaryProbe scope="all" data={towerFixture} />
        </>,
      ),
    )
    await settle()
    await settle()
    expect(sends()).toHaveLength(2)
    expect(sends()[0][1].options.agent).toBe('tower-writer')
    expect(sends()[1][1].options).not.toHaveProperty('agent')
    expect(sends()[1][1].idempotency_key).toBe(sends()[0][1].idempotency_key)
    expect(request).toHaveBeenCalledWith(
      'iii-tower-writer:tower-all',
      expect.any(Function),
    )
  })

  it('does not promote a partial status result or a completion with a result error', async () => {
    status = {
      status: 'failed',
      turn_id: 'existing',
      result: towerSummaryFixture,
    }
    await render('all', {
      ...towerFixture,
      loading: { ...towerFixture.loading, board: true },
    })
    expect(state.result).toBeNull()
    expect(state.error).toContain('did not complete')
    emit({
      session_id: 'tower-all',
      status: 'completed',
      result: towerSummaryFixture,
      result_error: 'Invalid output',
    })
    await flushEvent()
    expect(state.result).toBeNull()
    expect(state.error).toBe('Invalid output')
  })

  it('ignores an old scope’s slow status response', async () => {
    let resolveOld: (value: unknown) => void = () => {}
    const slow = new Promise((resolve) => {
      resolveOld = resolve
    })
    call.mockImplementation(
      async (id: string, payload: Record<string, unknown>) => {
        if (id === 'harness::status' && payload.session_id === 'tower-all')
          return slow
        return respond(id)
      },
    )
    await render('all')
    await render('assistente')
    expect(sends()).toHaveLength(1)
    await act(async () => {
      resolveOld({
        status: 'completed',
        turn_id: 'old',
        result: towerSummaryFixture,
      })
      await slow
    })
    await settle()
    expect(state.result).toBeNull()
    expect(sends()).toHaveLength(1)
    expect(sends()[0][1].session_id).toBe('tower-assistente')
  })

  it('gives manual Refresh a unique key while automatic sends keep the slot key', async () => {
    status = {
      status: 'completed',
      turn_id: 'existing',
      result: {
        ...towerSummaryFixture,
        checked_at: new Date(Date.now() - SUMMARY_INTERVAL - 1).toISOString(),
      },
    }
    await render()
    expect(sends()).toHaveLength(1)
    await act(() => state.refresh())
    await settle()
    expect(sends()).toHaveLength(2)
    expect(sends()[1][1].idempotency_key).toMatch(
      new RegExp(`^${sends()[0][1].idempotency_key}:refresh:`),
    )
    await act(async () => vi.advanceTimersByTimeAsync(SUMMARY_INTERVAL))
    await settle()
    expect(sends()).toHaveLength(3)
    expect(sends()[2][1].idempotency_key).not.toBe(
      sends()[0][1].idempotency_key,
    )
    expect(sends()[2][1].options).not.toHaveProperty('agent')
  })

  it('retains a manual Refresh while the header also reloads live sources', async () => {
    status = {
      status: 'completed',
      turn_id: 'existing',
      result: towerSummaryFixture,
    }
    await render()
    expect(sends()).toHaveLength(0)
    await render('all', {
      ...towerFixture,
      loading: { ...towerFixture.loading, sessions: true },
    })
    await act(() => state.refresh())
    await settle()
    expect(sends()).toHaveLength(0)
    await render()
    expect(sends()).toHaveLength(1)
  })

  it('queues a manual Refresh while an automatic send is busy', async () => {
    status = {
      status: 'completed',
      turn_id: 'existing',
      result: towerSummaryFixture,
    }
    await render()
    let resumeStatus = (_value: typeof status) => {}
    let delayStatus = true
    call.mockImplementation((id: string) => {
      if (id === 'harness::status' && delayStatus) {
        delayStatus = false
        return new Promise((resolve) => {
          resumeStatus = resolve
        })
      }
      return respond(id)
    })
    await act(async () => vi.advanceTimersByTimeAsync(SUMMARY_INTERVAL))
    await settle()
    await act(() => state.refresh())
    await act(() => resumeStatus(status))
    await settle()
    expect(sends()).toHaveLength(2)
    expect(sends()[0][1].idempotency_key).not.toContain(':refresh:')
    expect(sends()[1][1].idempotency_key).toMatch(/:refresh:[0-9a-f-]+$/)
  })

  it('does not leave a deduplicated Refresh waiting for a completion event that already fired', async () => {
    status = {
      status: 'completed',
      turn_id: 'existing',
      result: towerSummaryFixture,
    }
    call.mockImplementation(async (id: string) =>
      id === 'harness::send' ? { deduplicated: true } : respond(id),
    )
    await render()
    await act(() => state.refresh())
    await settle()
    expect(state.result).toEqual(towerSummaryFixture)
    expect(state.loading).toBe(false)
    expect(state.error).toBeNull()
  })

  it('sends the reloaded inputs after clicking the real header Refresh', async () => {
    status = {
      status: 'completed',
      turn_id: 'existing',
      result: towerSummaryFixture,
    }
    let finishReload = () => {}
    function HeaderProbe() {
      const [data, setData] = useState(towerFixture)
      const summary = useTowerSummary(iii, 'all', data)
      finishReload = () =>
        setData({ ...towerFixture, repositories: ['new/repo'], runs: [] })
      return (
        <TooltipProvider>
          <TowerScreen
            scope="all"
            onScopeChange={() => {}}
            summary={summary}
            data={{
              ...data,
              refresh: () =>
                setData({
                  ...data,
                  loading: { ...data.loading, sessions: true },
                }),
            }}
          />
        </TooltipProvider>
      )
    }
    await act(() => root.render(<HeaderProbe />))
    await settle()
    let finishStatus: (value: unknown) => void = () => {}
    const pendingStatus = new Promise((resolve) => {
      finishStatus = resolve
    })
    call.mockImplementation(async (id: string) =>
      id === 'harness::status' ? pendingStatus : respond(id),
    )
    await act(() =>
      container
        .querySelector<HTMLButtonElement>(
          '[aria-label="Refresh Control tower"]',
        )
        ?.click(),
    )
    await act(async () => {
      finishStatus(status)
      await pendingStatus
    })
    await settle()
    expect(sends()).toHaveLength(0)
    await act(() => finishReload())
    await settle()
    expect(sends()).toHaveLength(1)
    expect(JSON.parse(sends()[0][1].message)).toMatchObject({
      repos: ['new/repo'],
      session_ids: [],
    })
  })

  it('does not send a writer turn when its completion observer could not be registered', async () => {
    observe.mockImplementation(() => {
      throw new Error('Observer unavailable')
    })
    await render()
    expect(state.error).toBe('Observer unavailable')
    expect(state.loading).toBe(false)
    expect(sends()).toHaveLength(0)
    await act(() => state.refresh())
    await settle()
    expect(sends()).toHaveLength(0)
  })

  it('accepts only completed writer events, keeps valid prose before a malformed event in the debounce window, and advances unchanged freshness', async () => {
    await render()
    emit({
      session_id: 'another',
      status: 'completed',
      result: towerSummaryFixture,
    })
    await flushEvent()
    expect(state.result).toBeNull()
    const valid = {
      ...towerSummaryFixture,
      checked_at: new Date().toISOString(),
    }
    emit({ session_id: 'tower-all', status: 'completed', result: valid })
    emit({
      session_id: 'tower-all',
      status: 'completed',
      result: { bad: true },
    })
    await flushEvent()
    expect(state.result).toEqual(valid)
    expect(state.error).toContain('invalid')
    emit({
      session_id: 'tower-all',
      status: 'failed',
      result: { ...valid, brief: { ...valid.brief, text: 'Partial text' } },
    })
    await flushEvent()
    expect(state.result).toEqual(valid)
    const checked_at = new Date().toISOString()
    emit({
      session_id: 'tower-all',
      status: 'completed',
      result: { ...valid, unchanged: true, checked_at },
    })
    await flushEvent()
    expect(state.result).toEqual({ ...valid, unchanged: true, checked_at })
    expect(state.now).toBeGreaterThanOrEqual(Date.parse(checked_at))
  })

  it('waits for initial scope inputs, continues when GitHub fails, and ignores an old scope completion', async () => {
    await render('all', {
      ...towerFixture,
      loading: { ...towerFixture.loading, board: true },
    })
    expect(sends()).toHaveLength(0)
    await render('p1', {
      ...towerFixture,
      errors: { ...towerFixture.errors, prs: 'GitHub unavailable' },
    })
    expect(sends()).toHaveLength(1)
    emit({
      session_id: 'tower-all',
      status: 'completed',
      result: towerSummaryFixture,
    })
    await flushEvent()
    expect(state.result).toBeNull()
    expect(sends()[0][1].session_id).toBe('tower-p1')
  })

  it('does not send if granting a read fails or if the predictable session belongs to another agent', async () => {
    call.mockImplementation(async (id: string) => {
      if (id === 'approval::add-always-allow')
        throw new Error('Gate unavailable')
      return respond(id)
    })
    await render()
    expect(state.error).toContain('Gate unavailable')
    expect(sends()).toHaveLength(0)
    call.mockImplementation(respond)
    meta = { kind: 'user', metadata: { origin: 'someone-else' } }
    await act(() => state.refresh())
    await settle()
    expect(state.error).toContain('another agent or origin')
    expect(sends()).toHaveLength(0)
  })

  it('handles a missing kit/model as an absent state and can retry without resending an agent to an existing turn', async () => {
    call.mockImplementation(async (id: string) => {
      if (id === 'harness::send')
        throw new Error('Profile tower-writer has no model')
      return respond(id)
    })
    await render()
    expect(state.result).toBeNull()
    expect(state.error).toContain('no model')
    expect(state.loading).toBe(false)
    status = { status: 'completed', turn_id: 'existing', result: null }
    call.mockImplementation(respond)
    await act(() => state.refresh())
    await settle()
    expect(sends().at(-1)?.[1].options).not.toHaveProperty('agent')
  })

  it('cleans up observers, pending deliveries and timers, including a StrictMode remount', async () => {
    await render('all', towerFixture, true)
    emit({
      session_id: 'tower-all',
      status: 'completed',
      result: towerSummaryFixture,
    })
    await act(() => root.unmount())
    root = createRoot(container)
    expect(disposers.every((off) => off.mock.calls.length === 1)).toBe(true)
    const count = call.mock.calls.length
    await act(async () => vi.advanceTimersByTimeAsync(SUMMARY_INTERVAL * 2))
    expect(call).toHaveBeenCalledTimes(count)
  })
})
