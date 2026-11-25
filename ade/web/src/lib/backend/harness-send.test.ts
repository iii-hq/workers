import { describe, expect, it, vi } from 'vitest'
import {
  getTurnStatus,
  isTurnActive,
  predictedUserEntryId,
  sendTurn,
  stopTurn,
  toSkillOptions,
  toSystemPromptOptions,
  turnRest,
} from './harness-send'

function fakeClient(impl?: (fn: string, payload: unknown) => unknown) {
  const call = vi.fn(async (fn: string, payload: unknown) =>
    impl ? impl(fn, payload) : undefined,
  )
  return {
    call: call as unknown as <T>(fn: string, p?: object) => Promise<T>,
  }
}

describe('sendTurn', () => {
  it('calls harness::send with the request verbatim and returns the response', async () => {
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
      options: { functions: { allow: ['*'], expose: 'agent_call' } },
    })
    expect(res).toEqual({ session_id: 's-1', turn_id: 't-1', accepted: true })
    expect(client.call).toHaveBeenCalledWith('harness::send', {
      session_id: 's-1',
      message: 'hi',
      model: 'claude-sonnet-4',
      provider: 'anthropic',
      idempotency_key: 'msg-1',
      options: { functions: { allow: ['*'], expose: 'agent_call' } },
    })
  })
})

describe('stopTurn', () => {
  it('calls harness::stop and reports the stopping flag', async () => {
    const client = fakeClient(() => ({ stopping: true }))
    expect(await stopTurn(client, 's-1')).toBe(true)
    expect(client.call).toHaveBeenCalledWith('harness::stop', {
      session_id: 's-1',
      quiet_deadlines: true,
    })
  })

  it('passes turn_id when supplied', async () => {
    const client = fakeClient(() => ({ stopping: false }))
    expect(await stopTurn(client, 's-1', 't-9')).toBe(false)
    expect(client.call).toHaveBeenCalledWith('harness::stop', {
      session_id: 's-1',
      quiet_deadlines: true,
      turn_id: 't-9',
    })
  })
})

describe('getTurnStatus', () => {
  it('calls harness::status and returns the report (or null)', async () => {
    const report = {
      session_id: 's-1',
      turn_id: 't-1',
      status: 'running' as const,
      step: 2,
      turn_count: 1,
      depth: 0,
      max_turns: 16,
      validation_retries: 0,
      max_validation_retries: 2,
      transient_resumes: 1,
      max_transient_resumes: 1,
      partial_result_available: true,
      pending_function_calls: [],
      children: [],
      expects_wake: false,
      armed_wakes: [],
    }
    const client = fakeClient(() => report)
    expect(await getTurnStatus(client, 's-1')).toEqual(report)
    expect(client.call).toHaveBeenCalledWith('harness::status', {
      session_id: 's-1',
      verbose: true,
    })

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

describe('turnRest', () => {
  const ended = {
    status: 'completed' as const,
    expects_wake: false,
    open_asks: 0,
  }

  it('tells a completed turn that waits for a wake from one that ended', () => {
    expect(turnRest(ended)).toBe('ended')
    expect(turnRest({ ...ended, expects_wake: true })).toBe('waiting')
  })

  it('is null for a turn that did not complete', () => {
    for (const status of [
      'running',
      'awaiting_functions',
      'cancelled',
      'failed',
    ] as const) {
      expect(turnRest({ ...ended, status })).toBeNull()
      expect(turnRest({ ...ended, status, expects_wake: true })).toBeNull()
    }
  })

  it('is null while a question card is up, whether or not a wake is armed', () => {
    expect(turnRest({ ...ended, open_asks: 1 })).toBeNull()
    expect(turnRest({ ...ended, open_asks: 2 })).toBeNull()
    expect(turnRest({ ...ended, open_asks: 1, expects_wake: true })).toBeNull()
  })

  it('is null for an older harness without open_asks, and with no turn', () => {
    expect(turnRest({ ...ended, open_asks: undefined })).toBeNull()
    expect(
      turnRest({ ...ended, open_asks: undefined, expects_wake: true }),
    ).toBeNull()
    expect(turnRest(null)).toBeNull()
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

describe('toSystemPromptOptions', () => {
  it('returns no fields for default/empty selections', () => {
    expect(toSystemPromptOptions(null)).toEqual({})
    expect(toSystemPromptOptions(undefined)).toEqual({})
    expect(toSystemPromptOptions({ body: '   ', strategy: 'enrich' })).toEqual(
      {},
    )
  })

  it('maps body + strategy onto the snake_case wire fields', () => {
    expect(
      toSystemPromptOptions({
        body: 'Talk like a pirate.',
        strategy: 'override',
      }),
    ).toEqual({
      system_prompt: 'Talk like a pirate.',
      system_prompt_strategy: 'override',
    })
    expect(
      toSystemPromptOptions({ body: 'Be terse.', strategy: 'enrich' }),
    ).toEqual({
      system_prompt: 'Be terse.',
      system_prompt_strategy: 'enrich',
    })
  })
})

describe('toSkillOptions', () => {
  it('omits all-skills selections and maps an exact subset to the wire', () => {
    expect(toSkillOptions(undefined)).toEqual({})
    expect(toSkillOptions([])).toEqual({})
    expect(toSkillOptions(['review', 'release'])).toEqual({
      skills: ['review', 'release'],
    })
  })
})
