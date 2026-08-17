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
      thought('settled'),
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

  it('keeps a streaming thought in place as a live boundary', () => {
    const liveThought = thought('live', true)
    const rows = functionTriggerGroups([call('c1'), liveThought, call('c2')])

    expect(rows.map((row) => row.kind)).toEqual([
      'function-trigger-group',
      'message',
      'function-trigger-group',
    ])
    expect(rows[1]).toEqual({ kind: 'message', message: liveThought })
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
