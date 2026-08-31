import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import {
  collapsedFunctionTriggerCalls,
  functionTriggerGroups,
} from './function-trigger-groups'

function call(
  id: string,
  overrides: Partial<FunctionTriggerMessage> = {},
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId: 'shell::run',
    description: `call ${id}`,
    input: {},
    createdAt: 0,
    ...overrides,
  }
}

function assistant(
  id: string,
  stopReason: AssistantMessage['stopReason'],
): AssistantMessage {
  return {
    id,
    role: 'assistant',
    content: `message ${id}`,
    stopReason,
    createdAt: 0,
  }
}

function thought(id: string, streaming = false): ThoughtMessage {
  return {
    id,
    role: 'thought',
    content: `thought ${id}`,
    durationMs: 0,
    streaming,
    createdAt: 0,
  }
}

describe('functionTriggerGroups', () => {
  it('uses an intermediate update as the preceding batch summary and preserves final prose', () => {
    const intro = assistant('intro', 'function_call')
    const resultAndNext = assistant('progress', 'function_call')
    const final = assistant('final', 'end')
    const rows = functionTriggerGroups([
      intro,
      call('c1'),
      call('c2'),
      resultAndNext,
      call('c3'),
      final,
    ])

    expect(rows).toHaveLength(4)
    expect(rows[0]).toEqual({ kind: 'message', message: intro })
    expect(rows[1]).toMatchObject({
      kind: 'function-trigger-group',
      calls: [{ id: 'c1' }, { id: 'c2' }],
      summary: { id: 'progress' },
    })
    expect(rows[2]).toMatchObject({
      kind: 'function-trigger-group',
      calls: [{ id: 'c3' }],
      summary: undefined,
    })
    expect(rows[3]).toEqual({ kind: 'message', message: final })
  })

  it('keeps a streaming thought visible without splitting the call group', () => {
    const liveThought = thought('live', true)
    const rows = functionTriggerGroups([call('c1'), liveThought, call('c2')])

    expect(rows.map((row) => row.kind)).toEqual([
      'function-trigger-group',
      'message',
    ])
    expect(rows[0]).toMatchObject({
      id: 'function-trigger-group:c1',
      calls: [{ id: 'c1' }, { id: 'c2' }],
    })
    expect(rows[1]).toEqual({ kind: 'message', message: liveThought })
  })

  it('groups calls across completed thoughts once their surface is hidden', () => {
    const settledThought = thought('settled')
    const rows = functionTriggerGroups([call('c1'), settledThought, call('c2')])

    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({
      kind: 'function-trigger-group',
      calls: [{ id: 'c1' }, { id: 'c2' }],
    })
  })

  it('keeps the group identity stable while a completed thought exits', () => {
    const liveThought = thought('settled', true)
    const settledThought = thought('settled')
    const liveRows = functionTriggerGroups([call('c1'), liveThought])
    const rows = functionTriggerGroups(
      [call('c1'), settledThought, call('c2')],
      new Set(['settled']),
    )

    expect(rows.map((row) => row.kind)).toEqual([
      'function-trigger-group',
      'message',
    ])
    expect(rows[0]).toMatchObject({
      id: 'function-trigger-group:c1',
      calls: [{ id: 'c1' }, { id: 'c2' }],
    })
    expect(liveRows[0]).toMatchObject({
      id: 'function-trigger-group:c1',
      calls: [{ id: 'c1' }],
    })
    expect(liveRows[1]).toEqual({ kind: 'message', message: liveThought })
    expect(rows[1]).toEqual({ kind: 'message', message: settledThought })
  })

  it('keeps a leading thought before the first call during handoff', () => {
    const liveThought = thought('leading', true)
    const settledThought = thought('leading')
    const liveRows = functionTriggerGroups([liveThought])
    const exitRows = functionTriggerGroups(
      [settledThought, call('c1')],
      new Set(['leading']),
    )

    expect(liveRows).toEqual([{ kind: 'message', message: liveThought }])
    expect(exitRows.map((row) => row.kind)).toEqual([
      'message',
      'function-trigger-group',
    ])
    expect(exitRows[0]).toEqual({ kind: 'message', message: settledThought })
    expect(exitRows[1]).toMatchObject({
      id: 'function-trigger-group:c1',
      calls: [{ id: 'c1' }],
    })
  })

  it('recognizes legacy progress messages by the following call', () => {
    const legacyProgress = assistant('legacy', undefined)
    const rows = functionTriggerGroups([call('c1'), legacyProgress, call('c2')])

    expect(rows[0]).toMatchObject({
      kind: 'function-trigger-group',
      calls: [{ id: 'c1' }],
      summary: { id: 'legacy' },
    })
  })

  it('does not group calls across a user boundary', () => {
    const user: UserMessage = {
      id: 'u1',
      role: 'user',
      content: 'continue',
      createdAt: 0,
    }
    const rows = functionTriggerGroups([call('c1'), user, call('c2')])

    expect(rows.map((row) => row.kind)).toEqual([
      'function-trigger-group',
      'message',
      'function-trigger-group',
    ])
  })
})

describe('collapsedFunctionTriggerCalls', () => {
  it('keeps only the latest ordinary call', () => {
    expect(
      collapsedFunctionTriggerCalls(
        [call('c1'), call('c2'), call('c3')],
        () => false,
      ).map((message) => message.id),
    ).toEqual(['c3'])
  })

  it('also keeps rich displays, approvals, and running calls visible', () => {
    expect(
      collapsedFunctionTriggerCalls(
        [
          call('rich', { functionId: 'shell::file_changes' }),
          call('pending', { pendingApproval: true }),
          call('running', { running: true }),
          call('latest'),
        ],
        (message) => message.functionId === 'shell::file_changes',
      ).map((message) => message.id),
    ).toEqual(['rich', 'pending', 'running', 'latest'])
  })
})
