import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import {
  allowScheduleCalls,
  createSchedule,
  listAllSchedules,
  scheduleSessionAllow,
  waitForSchedule,
} from './api'

type Handler = (payload: Record<string, unknown>) => unknown

/** A host whose `iii.call` answers from `handlers`; a missing one throws the
    engine's "Function not found", as a stack without that worker does. */
function fakeHost(handlers: Record<string, Handler>) {
  const call = vi.fn(async (functionId: string, payload: Record<string, unknown> = {}) => {
    const handler = handlers[functionId]
    if (!handler) throw new Error(`${functionId}: Function not found`)
    return handler(payload)
  })
  return { host: { iii: { call } } as unknown as Host, call }
}

const tick = (id: string, extra: Record<string, unknown> = {}) => ({
  observer_id: id,
  function_id: 'cron::on-tick',
  args: { expression: '0 0 * * * *' },
  label: id,
  fires: 0,
  created_at: 1,
  ...extra,
})

describe('createSchedule', () => {
  it("lets the schedule session observe cron::on-tick, and call a call delivery's target", async () => {
    // Without these the harness refuses the agent's harness::observe: "observing
    // `cron::on-tick` is not permitted by this session's dispatch policy".
    expect(scheduleSessionAllow()).toEqual(['harness::observe', 'harness::unobserve', 'cron::on-tick'])
    expect(scheduleSessionAllow('state::get')).toContain('state::get')

    const { host, call } = fakeHost({ 'harness::send': () => ({ session_id: 's1' }) })
    await createSchedule(host, { title: 't', instruction: 'i', model: 'm', target: 'state::get' })
    const payload = call.mock.calls[0][1] as { options: { functions: { allow: string[] } } }
    expect(payload.options.functions.allow).toEqual([
      'harness::observe',
      'harness::unobserve',
      'cron::on-tick',
      'state::get',
    ])
  })
})

describe('allowScheduleCalls', () => {
  it('calls nothing of the approval gate when no worker provides it', async () => {
    const { host, call } = fakeHost({
      'engine::functions::list': () => ({ functions: [{ function_id: 'state::get' }] }),
    })
    await allowScheduleCalls(host, 's1')
    expect(call.mock.calls.map(([id]) => id)).toEqual(['engine::functions::list'])
  })

  it('grants both calls when the gate is there', async () => {
    const { host, call } = fakeHost({
      'engine::functions::list': () => ({
        functions: ['approval::approve-always', 'approval::list-pending', 'approval::resolve'].map((function_id) => ({
          function_id,
        })),
      }),
      'approval::approve-always': () => ({}),
      'approval::list-pending': () => ({ pending: [] }),
    })
    await allowScheduleCalls(host, 's1')
    expect(call.mock.calls.filter(([id]) => id === 'approval::approve-always')).toHaveLength(2)
  })
})

describe('listAllSchedules', () => {
  it('finds a schedule an agent made in an ordinary chat, with that chat as its owner', async () => {
    const { host } = fakeHost({
      'session::list': (payload) =>
        payload.metadata
          ? { sessions: [{ session_id: 'page-1', title: 'Nightly' }] }
          : {
              sessions: [
                { session_id: 'page-1', title: 'Nightly' },
                { session_id: 'chat-1', title: 'Plain chat' },
                { session_id: 'chat-2', title: 'Agent chat' },
              ],
            },
      'harness::observers::list': (payload) =>
        payload.session_id === 'page-1'
          ? { observers: [tick('sub_page')] }
          : payload.session_id === 'chat-2'
            ? { observers: [tick('sub_agent', { target: 'state::get' })] }
            : { observers: [] },
      'engine::observers::list': () => ({
        observers: ['sub_page', 'sub_agent'].map((id) => ({
          id,
          function_id: 'cron::on-tick',
          target: { function_id: 'harness::observer::deliver' },
        })),
      }),
    })
    const tasks = await listAllSchedules(host)
    expect(tasks.map((task) => [task.observerId, task.sessionId, task.sessionTitle]).sort()).toEqual([
      ['sub_agent', 'chat-2', 'Agent chat'],
      ['sub_page', 'page-1', 'Nightly'],
    ])
    expect(tasks.find((task) => task.observerId === 'sub_agent')?.fromChat).toBe(true)
    expect(tasks.find((task) => task.observerId === 'sub_page')?.fromChat).toBeUndefined()
  })

  it('reads no other chat when every harness schedule is already accounted for', async () => {
    const { host, call } = fakeHost({
      'session::list': () => ({ sessions: [{ session_id: 'page-1' }] }),
      'harness::observers::list': () => ({ observers: [tick('sub_page')] }),
      'engine::observers::list': () => ({
        observers: [{ id: 'sub_page', function_id: 'cron::on-tick', target: { function_id: 'harness::observer::deliver' } }],
      }),
    })
    await listAllSchedules(host)
    expect(call.mock.calls.filter(([id]) => id === 'session::list')).toHaveLength(1)
  })
})

describe('waitForSchedule', () => {
  const sleep = async () => {}

  it('is created once the session owns a cron observer', async () => {
    let reads = 0
    const { host } = fakeHost({
      'harness::observers::list': () => ({ observers: reads++ > 1 ? [tick('sub_1')] : [] }),
      'session::get': () => ({ meta: { status: 'working' } }),
    })
    const outcome = await waitForSchedule(host, 's1', { sleep })
    expect(outcome).toMatchObject({ status: 'created', task: { observerId: 'sub_1' } })
  })

  it("fails with the agent's reply when the turn ends without one", async () => {
    const { host } = fakeHost({
      'harness::observers::list': () => ({ observers: [] }),
      'session::get': () => ({ meta: { status: 'done' } }),
      'session::messages': () => ({
        messages: [
          { message: { role: 'user', content: [{ type: 'text', text: 'Create it' }] } },
          { message: { role: 'assistant', content: [{ type: 'text', text: 'FAILED: cron::on-tick is denied by policy' }] } },
        ],
      }),
    })
    expect(await waitForSchedule(host, 's1', { sleep })).toEqual({
      status: 'failed',
      reply: 'FAILED: cron::on-tick is denied by policy',
    })
  })

  it('stays pending, never successful, while the turn has not finished', async () => {
    const { host } = fakeHost({
      'harness::observers::list': () => ({ observers: [] }),
      'session::get': () => ({ meta: { status: 'working' } }),
    })
    expect(await waitForSchedule(host, 's1', { sleep, timeoutMs: 3_000, intervalMs: 1_000 })).toEqual({
      status: 'pending',
    })
  })
})
