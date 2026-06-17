import { describe, expect, it, vi } from 'vitest'
import {
  getTurnStatus,
  isTurnActive,
  predictedUserEntryId,
  sendTurn,
  stopTurn,
} from './harness-send'

function fakeClient(impl?: (fn: string, payload: unknown) => unknown) {
  const trigger = vi.fn(async (fn: string, payload: unknown) =>
    impl ? impl(fn, payload) : undefined,
  )
  return {
    trigger: trigger as unknown as <T>(fn: string, p?: object) => Promise<T>,
  }
}

describe('sendTurn', () => {
  it('triggers harness::send with the request verbatim and returns the response', async () => {
    const client = fakeClient(() => ({
      session_id: 's-1',
      turn_id: 't-1',
      accepted: true,
    }))
    const res = await sendTurn(client, {
      session_id: 's-1',
      message: 'hi',
      model: 'claude-sonnet-4',
      provider: 'anthropic',
      idempotency_key: 'msg-1',
      options: { functions: { allow: ['*'], expose: 'agent_trigger' } },
    })
    expect(res).toEqual({ session_id: 's-1', turn_id: 't-1', accepted: true })
    expect(client.trigger).toHaveBeenCalledWith('harness::send', {
      session_id: 's-1',
      message: 'hi',
      model: 'claude-sonnet-4',
      provider: 'anthropic',
      idempotency_key: 'msg-1',
      options: { functions: { allow: ['*'], expose: 'agent_trigger' } },
    })
  })
})

describe('stopTurn', () => {
  it('triggers harness::stop and reports the stopping flag', async () => {
    const client = fakeClient(() => ({ stopping: true }))
    expect(await stopTurn(client, 's-1')).toBe(true)
    expect(client.trigger).toHaveBeenCalledWith('harness::stop', {
      session_id: 's-1',
    })
  })

  it('passes turn_id when supplied', async () => {
    const client = fakeClient(() => ({ stopping: false }))
    expect(await stopTurn(client, 's-1', 't-9')).toBe(false)
    expect(client.trigger).toHaveBeenCalledWith('harness::stop', {
      session_id: 's-1',
      turn_id: 't-9',
    })
  })
})

describe('getTurnStatus', () => {
  it('triggers harness::status and returns the report (or null)', async () => {
    const report = {
      session_id: 's-1',
      status: 'running' as const,
      step: 2,
      turn_count: 1,
      depth: 0,
      max_turns: 16,
      pending_function_calls: [],
      children: [],
    }
    const client = fakeClient(() => report)
    expect(await getTurnStatus(client, 's-1')).toEqual(report)

    const empty = fakeClient(() => null)
    expect(await getTurnStatus(empty, 's-1')).toBeNull()
  })
})

describe('isTurnActive', () => {
  it('is true only for in-flight statuses', () => {
    expect(isTurnActive('running')).toBe(true)
    expect(isTurnActive('awaiting_functions')).toBe(true)
    expect(isTurnActive('completed')).toBe(false)
    expect(isTurnActive('cancelled')).toBe(false)
    expect(isTurnActive('failed')).toBe(false)
  })
})

describe('predictedUserEntryId', () => {
  it('mirrors the harness e_idem_<key> scheme', () => {
    expect(predictedUserEntryId('msg-abc')).toBe('e_idem_msg-abc')
  })

  it('replaces characters outside [A-Za-z0-9_-]', () => {
    expect(predictedUserEntryId('a/b c')).toBe('e_idem_a_b_c')
  })
})
