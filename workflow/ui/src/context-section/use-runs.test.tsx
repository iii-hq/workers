// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  resetWorkflowReads,
  useWorkflowRuns,
  WORKFLOW_DETAILS_MS,
  WORKFLOW_IDLE_LIST_MS,
  WORKFLOW_REFRESH_MS,
  WORKFLOW_RUN_LIMIT,
} from './use-runs'
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const mocks = { status: vi.fn(), list: vi.fn() }
// One client for every render, as the host hands it.
const iii = {
  call: (fn: string, payload?: Record<string, unknown>) =>
    fn === 'workflow::list' ? mocks.list(payload) : mocks.status(fn, payload),
} as Props['iii']

type Props = Parameters<typeof useWorkflowRuns>[0]

/** `workflow::list` replies by chat, newest first. */
let LISTS: Record<string, string[]> = {}
const listed = (...runIds: string[]) => {
  LISTS.main = runIds
}

const snapshot = (runId: string, status: string, details = true) => ({
  run_id: runId,
  status,
  nodes: { a: status === 'running' ? 'running' : 'done' },
  ...(details
    ? {
        node_details: {
          a: { state: status === 'running' ? 'running' : 'done', label: 'A' },
        },
      }
    : {}),
})

let root: Root | null = null
let latest: ReturnType<typeof useWorkflowRuns> = { runs: [], earlier: 0 }

function Probe(props: Props) {
  latest = useWorkflowRuns(props)
  return null
}

const base: Props = { iii, sessionId: 'main', active: true, open: true, isStreaming: false }

async function render(props: Partial<Props>) {
  root ??= createRoot(document.createElement('div'))
  await act(async () => {
    root?.render(<Probe {...base} {...props} />)
  })
  await settle()
}

const advance = (ms: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms)
  })

/** The list, then the reads it names, then their replies. */
const settle = async () => {
  for (let i = 0; i < 3; i += 1) await advance(0)
}

/** One refresh at a time: updates inside one `act` flush only at its end. */
const refreshes = async (count: number) => {
  for (let i = 0; i < count; i += 1) await advance(WORKFLOW_REFRESH_MS)
}

const statusCalls = () =>
  mocks.status.mock.calls.filter(([fn]) => fn === 'workflow::status').map(([fn, payload]) => [fn, payload])
const listCalls = () => mocks.list.mock.calls.map(([payload]) => payload)

beforeEach(() => {
  vi.useFakeTimers()
  resetWorkflowReads()
  mocks.status.mockReset()
  mocks.list.mockReset()
  LISTS = {}
  mocks.list.mockImplementation(({ caller_session_id }: { caller_session_id: string }) =>
    Promise.resolve({
      runs: (LISTS[caller_session_id] ?? []).map((run_id, i) => ({
        run_id,
        status: 'running',
        created_at: 1000 - i,
        updated_at: 1000 - i,
      })),
    }),
  )
  latest = { runs: [], earlier: 0 }
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  vi.useRealTimers()
})

describe('useWorkflowRuns', () => {
  it('asks only for the list of a chat that started no run', async () => {
    await render({})
    await advance(WORKFLOW_REFRESH_MS * 3)
    expect(listCalls()).toEqual([{ caller_session_id: 'main' }])
    expect(mocks.status).not.toHaveBeenCalled()
    expect(latest.runs).toEqual([])
  })

  it('reads a finished run once, with its node details', async () => {
    mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
    listed('r_1')
    await render({})
    await advance(WORKFLOW_REFRESH_MS * 10)
    expect(statusCalls()).toEqual([['workflow::status', { run_id: 'r_1', details: true }]])
    expect(latest.runs.map((run) => [run.runId, run.status])).toEqual([['r_1', 'completed']])
    // Nothing going: the list is asked again only at the idle pace.
    expect(listCalls()).toHaveLength(2)
  })

  it('reads a run again while it is going, then its details once it ends', async () => {
    mocks.status.mockResolvedValue(snapshot('r_1', 'running'))
    listed('r_1')
    await render({})
    expect(statusCalls()).toHaveLength(1)
    expect(latest.runs[0].live).toBe(true)
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
    mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(3)
    expect(latest.runs[0].live).toBe(false)
    await advance(WORKFLOW_REFRESH_MS * 10)
    expect(statusCalls().map(([, payload]) => payload)).toEqual([
      { run_id: 'r_1', details: true },
      { run_id: 'r_1', details: false },
      { run_id: 'r_1', details: false },
      { run_id: 'r_1', details: true },
    ])
  })

  it('reads the node details of a run going on at a slower pace, keeping node states fresh', async () => {
    mocks.status.mockImplementation((_fn, payload: { details: boolean }) =>
      Promise.resolve(
        payload.details ? snapshot('r_1', 'running') : { run_id: 'r_1', status: 'running', nodes: { a: 'done' } },
      ),
    )
    listed('r_1')
    await render({})
    await advance(WORKFLOW_REFRESH_MS)
    expect(latest.runs[0].phases[0].nodes[0]).toMatchObject({
      label: 'A',
      state: 'done',
    })
    await refreshes(WORKFLOW_DETAILS_MS / WORKFLOW_REFRESH_MS - 1)
    expect(statusCalls().map(([, payload]) => (payload as { details: boolean }).details)).toEqual([
      true,
      false,
      false,
      false,
      false,
      false,
      true,
    ])
  })

  it('keeps reading a run going on after one read fails', async () => {
    mocks.status.mockResolvedValueOnce(snapshot('r_1', 'running'))
    mocks.status.mockRejectedValueOnce(new Error('reconnecting'))
    mocks.status.mockResolvedValue(snapshot('r_1', 'running'))
    listed('r_1')
    await render({})
    await refreshes(2)
    expect(statusCalls()).toHaveLength(3)
    expect(latest.runs[0].live).toBe(true)
  })

  it('serves a turn change during a read with the read on its way', async () => {
    let answer: (value: unknown) => void = () => {}
    mocks.status.mockImplementation(() => new Promise((resolve) => (answer = resolve)))
    listed('r_1')
    await render({ isStreaming: true })
    await render({ isStreaming: false })
    await render({ isStreaming: true })
    expect(statusCalls()).toHaveLength(1)
    answer(snapshot('r_1', 'running'))
    await advance(0)
    expect(latest.runs).toHaveLength(1)
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
  })

  it('reads a finished run once more while its usage settles', async () => {
    mocks.status.mockResolvedValue({
      run_id: 'r_1',
      status: 'completed',
      nodes: { a: 'done' },
      node_details: {
        a: { state: 'done', started_at: 1, usage: { complete: false } },
      },
    })
    listed('r_1')
    await render({})
    await refreshes(10)
    expect(statusCalls()).toHaveLength(2)
  })

  it('keeps what it read when the panel mounts again', async () => {
    mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
    listed('r_1')
    await render({})
    act(() => root?.unmount())
    root = null
    mocks.list.mockImplementation(() => new Promise(() => {}))
    await render({})
    expect(statusCalls()).toHaveLength(1)
    // Shown before the new list is back.
    expect(latest.runs).toHaveLength(1)
  })

  it('waits for a slow read before asking again', async () => {
    let answer: (value: unknown) => void = () => {}
    mocks.status.mockImplementation(() => new Promise((resolve) => (answer = resolve)))
    listed('r_1')
    await render({})
    await advance(WORKFLOW_REFRESH_MS * 3)
    expect(statusCalls()).toHaveLength(1)
    answer(snapshot('r_1', 'running'))
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
  })

  it('folded, asks for the status alone, then the details once unfolded', async () => {
    mocks.status.mockImplementation((_fn, payload: { details: boolean }) =>
      Promise.resolve(snapshot('r_1', 'completed', payload.details)),
    )
    listed('r_1')
    await render({ open: false })
    expect(statusCalls()).toEqual([['workflow::status', { run_id: 'r_1', details: false }]])
    expect(latest.runs[0].phases).toEqual([])
    await render({ open: true })
    expect(statusCalls()).toHaveLength(2)
    expect(statusCalls()[1][1]).toEqual({ run_id: 'r_1', details: true })
    expect(latest.runs[0].phases).toHaveLength(1)
    await render({ open: false })
    await render({ open: true })
    expect(statusCalls()).toHaveLength(2)
  })

  it('asks nothing while out of sight, and resumes at once', async () => {
    mocks.status.mockResolvedValue(snapshot('r_1', 'running'))
    listed('r_1')
    await render({ active: false })
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(listCalls()).toHaveLength(0)
    expect(statusCalls()).toHaveLength(0)
    await render({})
    expect(listCalls()).toHaveLength(1)
    expect(statusCalls()).toHaveLength(1)
    await render({ active: false })
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(statusCalls()).toHaveLength(1)
    await render({})
    expect(listCalls()).toHaveLength(2)
    expect(statusCalls()).toHaveLength(2)
  })

  it('leaves out a run the worker no longer has and never asks again', async () => {
    mocks.status.mockResolvedValue(null)
    listed('r_gone')
    await render({ isStreaming: true })
    await render({ isStreaming: false })
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(statusCalls()).toHaveLength(1)
    expect(latest.runs).toEqual([])
  })

  it('asks again after a failed call when a turn ends', async () => {
    mocks.status.mockRejectedValueOnce(new Error('offline'))
    mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
    listed('r_1')
    await render({ isStreaming: true })
    expect(latest.runs).toEqual([])
    await render({ isStreaming: false })
    expect(statusCalls()).toHaveLength(2)
    expect(latest.runs).toHaveLength(1)
  })

  it('retries a failed first read at the next refresh', async () => {
    mocks.status.mockRejectedValueOnce(new Error('offline'))
    mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
    listed('r_1')
    await render({})
    expect(latest.runs).toEqual([])
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
    expect(latest.runs).toHaveLength(1)
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(statusCalls()).toHaveLength(2)
  })

  it('shows a read that landed while the panel was out of sight', async () => {
    let answer: (value: unknown) => void = () => {}
    mocks.status.mockImplementation(() => new Promise((resolve) => (answer = resolve)))
    listed('r_1')
    await render({})
    await render({ active: false })
    answer(snapshot('r_1', 'completed'))
    await advance(0)
    await render({})
    expect(statusCalls()).toHaveLength(1)
    expect(latest.runs).toHaveLength(1)
  })

  it('unfolded during a status read, asks for the details once it is back', async () => {
    const answers: ((value: unknown) => void)[] = []
    mocks.status.mockImplementation(() => new Promise((resolve) => answers.push(resolve)))
    listed('r_1')
    await render({ open: false })
    await render({ open: true })
    expect(statusCalls()).toEqual([['workflow::status', { run_id: 'r_1', details: false }]])
    answers[0](snapshot('r_1', 'running', false))
    await advance(0)
    expect(statusCalls()[1]).toEqual(['workflow::status', { run_id: 'r_1', details: true }])
    answers[1](snapshot('r_1', 'completed'))
    await advance(0)
    expect(latest.runs[0]).toMatchObject({ status: 'completed', live: false })
    expect(latest.runs[0].phases).toHaveLength(1)
    await advance(WORKFLOW_REFRESH_MS * 3)
    expect(statusCalls()).toHaveLength(2)
  })

  it('lists the runs newest first, as the worker lists them', async () => {
    mocks.status.mockImplementation((_fn, payload: { run_id: string }) =>
      Promise.resolve(snapshot(payload.run_id, 'completed')),
    )
    listed('r_new', 'r_old')
    await render({})
    expect(latest.runs.map((run) => run.runId)).toEqual(['r_new', 'r_old'])
  })

  it('reads only the newest runs and counts the earlier ones', async () => {
    mocks.status.mockImplementation((_fn, payload: { run_id: string }) =>
      Promise.resolve(snapshot(payload.run_id, 'completed')),
    )
    listed(...Array.from({ length: WORKFLOW_RUN_LIMIT + 5 }, (_, i) => `r_${WORKFLOW_RUN_LIMIT + 4 - i}`))
    await render({})
    expect(statusCalls()).toHaveLength(WORKFLOW_RUN_LIMIT)
    expect(latest.earlier).toBe(5)
    expect(latest.runs[0].runId).toBe(`r_${WORKFLOW_RUN_LIMIT + 4}`)
  })

  describe('the list', () => {
    it("lists again at a turn's start and end, and when the window gets focus", async () => {
      await render({})
      await render({ isStreaming: true })
      await render({ isStreaming: false })
      expect(listCalls()).toHaveLength(3)
      await act(async () => {
        window.dispatchEvent(new Event('focus'))
      })
      await advance(0)
      expect(listCalls()).toHaveLength(4)
      // Out of sight, focus asks nothing.
      await render({ active: false })
      await act(async () => {
        window.dispatchEvent(new Event('focus'))
      })
      await advance(0)
      expect(listCalls()).toHaveLength(4)
    })

    it('finds a run a turn starts while the turn is going', async () => {
      mocks.status.mockResolvedValue(snapshot('r_1', 'running'))
      await render({ isStreaming: true })
      expect(latest.runs).toEqual([])
      listed('r_1')
      await advance(WORKFLOW_REFRESH_MS)
      await settle()
      expect(listCalls()).toHaveLength(2)
      expect(latest.runs.map((run) => run.runId)).toEqual(['r_1'])
    })

    it('lists again while a run is going, and stops once nothing is', async () => {
      mocks.status.mockResolvedValue(snapshot('r_1', 'running'))
      listed('r_1')
      // Folded, so the idle re-list stays out of the count.
      await render({ open: false })
      await refreshes(2)
      expect(listCalls()).toHaveLength(3)
      mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
      await refreshes(4)
      const settled = listCalls().length
      await refreshes(4)
      expect(listCalls()).toHaveLength(settled)
    })

    it('lists again every half minute while unfolded and idle, to find a run a sub-agent starts', async () => {
      mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
      await render({})
      expect(listCalls()).toHaveLength(1)
      await advance(WORKFLOW_IDLE_LIST_MS - 1)
      expect(listCalls()).toHaveLength(1)
      listed('r_1')
      await advance(1)
      await settle()
      expect(listCalls()).toHaveLength(2)
      expect(latest.runs.map((run) => run.runId)).toEqual(['r_1'])
      await advance(WORKFLOW_IDLE_LIST_MS)
      expect(listCalls()).toHaveLength(3)
    })

    it('does not list again on its own while folded or out of sight', async () => {
      await render({ open: false })
      await advance(WORKFLOW_IDLE_LIST_MS * 3)
      expect(listCalls()).toHaveLength(1)
      await render({ open: true, active: false })
      await advance(WORKFLOW_IDLE_LIST_MS * 3)
      expect(listCalls()).toHaveLength(1)
    })

    it('lists again as soon as the section unfolds', async () => {
      await render({ open: false })
      expect(listCalls()).toHaveLength(1)
      await render({ open: true })
      expect(listCalls()).toHaveLength(2)
    })

    it('never shows a slow list under another chat', async () => {
      mocks.status.mockImplementation((_fn, payload: { run_id: string }) =>
        Promise.resolve(snapshot(payload.run_id, 'completed')),
      )
      let answer: (value: unknown) => void = () => {}
      mocks.list.mockImplementationOnce(() => new Promise((resolve) => (answer = resolve)))
      LISTS.a = ['r_a']
      LISTS.b = ['r_b']
      await render({ sessionId: 'a' })
      await render({ sessionId: 'b' })
      answer({ runs: [{ run_id: 'r_a', status: 'completed', created_at: 1, updated_at: 2 }] })
      await advance(0)
      expect(latest.runs.map((run) => run.runId)).toEqual(['r_b'])
      await render({ sessionId: 'a' })
      expect(latest.runs.map((run) => run.runId)).toEqual(['r_a'])
    })

    it('keeps the runs listed when a list fails or cannot be read', async () => {
      mocks.status.mockResolvedValue(snapshot('r_1', 'completed'))
      listed('r_1')
      await render({})
      mocks.list.mockRejectedValueOnce(new Error('offline'))
      await render({ isStreaming: true })
      expect(latest.runs).toHaveLength(1)
      mocks.list.mockResolvedValueOnce({ error: 'nope' })
      await render({ isStreaming: false })
      expect(latest.runs).toHaveLength(1)
    })

    it('sends a list asked during one on its way after it, once', async () => {
      const answers: ((value: unknown) => void)[] = []
      mocks.list.mockImplementation(() => new Promise((resolve) => answers.push(resolve)))
      await render({})
      await render({ isStreaming: true })
      await render({ isStreaming: false })
      // The one on its way may predate a run the turn started: one more follows it.
      expect(listCalls()).toHaveLength(1)
      answers[0]({ runs: [] })
      await advance(0)
      expect(listCalls()).toHaveLength(2)
      answers[1]({ runs: [] })
      await advance(0)
      expect(listCalls()).toHaveLength(2)
    })
  })
})
