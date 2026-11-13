// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Conversation, Message } from '@/types/chat'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ call: mocks.call }),
}))

import {
  resetWorkflowReads,
  useContextWorkflow,
  WORKFLOW_DETAILS_MS,
  WORKFLOW_REFRESH_MS,
  WORKFLOW_RUN_LIMIT,
} from './ContextWorkflow'

type Props = Parameters<typeof useContextWorkflow>[0]

const start = (runId: string, at = 1): Message =>
  ({
    id: `m-${runId}`,
    role: 'function-call',
    functionId: 'workflow::start',
    description: `Run ${runId}`,
    input: { nodes: {} },
    output: { content: [], details: { run_id: runId } },
    createdAt: at,
  }) as Message

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
let latest: ReturnType<typeof useContextWorkflow> = { runs: [], earlier: 0 }

function Probe(props: Props) {
  latest = useContextWorkflow(props)
  return null
}

const base: Props = {
  messages: [],
  sessions: [] as Conversation[],
  active: true,
  shown: true,
  open: true,
}

async function render(props: Partial<Props>) {
  root ??= createRoot(document.createElement('div'))
  await act(async () => {
    root?.render(<Probe {...base} {...props} />)
  })
  await advance(0)
}

const advance = (ms: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms)
  })

/** One refresh at a time: updates inside one `act` flush only at its end. */
const refreshes = async (count: number) => {
  for (let i = 0; i < count; i += 1) await advance(WORKFLOW_REFRESH_MS)
}

const statusCalls = () =>
  mocks.call.mock.calls
    .filter(([fn]) => fn === 'workflow::status')
    .map(([fn, payload]) => [fn, payload])

beforeEach(() => {
  vi.useFakeTimers()
  resetWorkflowReads()
  mocks.call.mockReset()
  latest = { runs: [], earlier: 0 }
})

afterEach(() => {
  act(() => root?.unmount())
  root = null
  vi.useRealTimers()
})

describe('useContextWorkflow', () => {
  it('asks nothing for a chat that started no run', async () => {
    await render({})
    expect(mocks.call).not.toHaveBeenCalled()
    expect(latest.runs).toEqual([])
  })

  it('reads a finished run once, with its node details', async () => {
    mocks.call.mockResolvedValue(snapshot('r_1', 'completed'))
    const messages = [start('r_1')]
    await render({ messages })
    await advance(WORKFLOW_REFRESH_MS * 10)
    expect(statusCalls()).toEqual([
      ['workflow::status', { run_id: 'r_1', details: true }],
    ])
    expect(latest.runs.map((run) => [run.runId, run.status])).toEqual([
      ['r_1', 'completed'],
    ])
  })

  it('reads a run again while it is going, then its details once it ends', async () => {
    mocks.call.mockResolvedValue(snapshot('r_1', 'running'))
    const messages = [start('r_1')]
    await render({ messages })
    expect(statusCalls()).toHaveLength(1)
    expect(latest.runs[0].live).toBe(true)
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
    mocks.call.mockResolvedValue(snapshot('r_1', 'completed'))
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
    mocks.call.mockImplementation((_fn, payload: { details: boolean }) =>
      Promise.resolve(
        payload.details
          ? snapshot('r_1', 'running')
          : { run_id: 'r_1', status: 'running', nodes: { a: 'done' } },
      ),
    )
    await render({ messages: [start('r_1')] })
    await advance(WORKFLOW_REFRESH_MS)
    expect(latest.runs[0].phases[0].nodes[0]).toMatchObject({
      label: 'A',
      state: 'done',
    })
    await refreshes(WORKFLOW_DETAILS_MS / WORKFLOW_REFRESH_MS - 1)
    expect(
      statusCalls().map(
        ([, payload]) => (payload as { details: boolean }).details,
      ),
    ).toEqual([true, false, false, false, false, false, true])
  })

  it('keeps reading a run going on after one read fails', async () => {
    mocks.call.mockResolvedValueOnce(snapshot('r_1', 'running'))
    mocks.call.mockRejectedValueOnce(new Error('reconnecting'))
    mocks.call.mockResolvedValue(snapshot('r_1', 'running'))
    await render({ messages: [start('r_1')] })
    await refreshes(2)
    expect(statusCalls()).toHaveLength(3)
    expect(latest.runs[0].live).toBe(true)
  })

  it('serves a turn change during a read with the read on its way', async () => {
    let answer: (value: unknown) => void = () => {}
    mocks.call.mockImplementation(
      () => new Promise((resolve) => (answer = resolve)),
    )
    const messages = [start('r_1')]
    await render({ messages, status: 'working' })
    await render({ messages, status: 'idle' })
    await render({ messages, status: 'working' })
    expect(statusCalls()).toHaveLength(1)
    answer(snapshot('r_1', 'running'))
    await advance(0)
    expect(latest.runs).toHaveLength(1)
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
  })

  it('reads a finished run once more while its usage settles', async () => {
    mocks.call.mockResolvedValue({
      run_id: 'r_1',
      status: 'completed',
      nodes: { a: 'done' },
      node_details: {
        a: { state: 'done', started_at: 1, usage: { complete: false } },
      },
    })
    await render({ messages: [start('r_1')] })
    await refreshes(10)
    expect(statusCalls()).toHaveLength(2)
  })

  it('keeps what it read when the panel mounts again', async () => {
    mocks.call.mockResolvedValue(snapshot('r_1', 'completed'))
    const messages = [start('r_1')]
    await render({ messages })
    act(() => root?.unmount())
    root = null
    await render({ messages })
    expect(statusCalls()).toHaveLength(1)
    expect(latest.runs).toHaveLength(1)
  })

  it('waits for a slow read before asking again', async () => {
    let answer: (value: unknown) => void = () => {}
    mocks.call.mockImplementation(
      () => new Promise((resolve) => (answer = resolve)),
    )
    const messages = [start('r_1')]
    await render({ messages })
    await advance(WORKFLOW_REFRESH_MS * 3)
    expect(statusCalls()).toHaveLength(1)
    answer(snapshot('r_1', 'running'))
    await advance(WORKFLOW_REFRESH_MS)
    expect(statusCalls()).toHaveLength(2)
  })

  it('folded, asks for the status alone, then the details once unfolded', async () => {
    mocks.call.mockImplementation((_fn, payload: { details: boolean }) =>
      Promise.resolve(snapshot('r_1', 'completed', payload.details)),
    )
    const messages = [start('r_1')]
    await render({ messages, open: false })
    expect(statusCalls()).toEqual([
      ['workflow::status', { run_id: 'r_1', details: false }],
    ])
    expect(latest.runs[0].phases).toEqual([])
    await render({ messages, open: true })
    expect(statusCalls()).toHaveLength(2)
    expect(statusCalls()[1][1]).toEqual({ run_id: 'r_1', details: true })
    expect(latest.runs[0].phases).toHaveLength(1)
    await render({ messages, open: false })
    await render({ messages, open: true })
    expect(statusCalls()).toHaveLength(2)
  })

  it('asks nothing while hidden or out of sight, and resumes at once', async () => {
    mocks.call.mockResolvedValue(snapshot('r_1', 'running'))
    const messages = [start('r_1')]
    await render({ messages, shown: false })
    await render({ messages, active: false })
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(statusCalls()).toHaveLength(0)
    await render({ messages })
    expect(statusCalls()).toHaveLength(1)
    await render({ messages, active: false })
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(statusCalls()).toHaveLength(1)
    await render({ messages })
    expect(statusCalls()).toHaveLength(2)
  })

  it('leaves out a run the worker no longer has and never asks again', async () => {
    mocks.call.mockResolvedValue(null)
    const messages = [start('r_gone')]
    await render({ messages, status: 'working' })
    await render({ messages, status: 'done' })
    expect(statusCalls()).toHaveLength(1)
    expect(latest.runs).toEqual([])
  })

  it('asks again after a failed call when a turn ends', async () => {
    mocks.call.mockRejectedValueOnce(new Error('offline'))
    mocks.call.mockResolvedValue(snapshot('r_1', 'completed'))
    const messages = [start('r_1')]
    await render({ messages, status: 'working' })
    expect(latest.runs).toEqual([])
    await advance(WORKFLOW_REFRESH_MS * 2)
    expect(statusCalls()).toHaveLength(1)
    await render({ messages, status: 'done' })
    expect(latest.runs).toHaveLength(1)
  })

  it('finds a run through its node sessions and lists the newest first', async () => {
    mocks.call.mockImplementation((_fn, payload: { run_id: string }) =>
      Promise.resolve(snapshot(payload.run_id, 'completed')),
    )
    const sessions = [
      {
        id: 'wf_r_old_a@r0',
        title: 'A',
        model: null,
        messages: [],
        createdAt: 5,
        updatedAt: 5,
        sessionMetadata: { workflow_run_id: 'r_old' },
      },
    ] as Conversation[]
    await render({ messages: [start('r_new', 9)], sessions })
    expect(latest.runs.map((run) => run.runId)).toEqual(['r_new', 'r_old'])
  })

  it('reads only the newest runs and counts the earlier ones', async () => {
    mocks.call.mockImplementation((_fn, payload: { run_id: string }) =>
      Promise.resolve(snapshot(payload.run_id, 'completed')),
    )
    const messages = Array.from({ length: WORKFLOW_RUN_LIMIT + 5 }, (_, i) =>
      start(`r_${i}`, i),
    )
    await render({ messages })
    expect(statusCalls()).toHaveLength(WORKFLOW_RUN_LIMIT)
    expect(latest.earlier).toBe(5)
    expect(latest.runs[0].runId).toBe(`r_${WORKFLOW_RUN_LIMIT + 4}`)
  })
})
