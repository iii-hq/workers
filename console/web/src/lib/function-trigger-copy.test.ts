import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import {
  assistantCopyText,
  functionTriggersByAssistant,
  functionTriggerToText,
} from './function-trigger-copy'

function fcall(
  overrides: Partial<FunctionTriggerMessage> = {},
): FunctionTriggerMessage {
  return {
    id: 'f1',
    createdAt: 0,
    role: 'function-trigger',
    functionId: 'shell::exec',
    input: { command: 'npm test' },
    ...overrides,
  }
}

function assistant(
  overrides: Partial<AssistantMessage> = {},
): AssistantMessage {
  return {
    id: 'a1',
    createdAt: 0,
    role: 'assistant',
    content: 'running the tests',
    ...overrides,
  }
}

function user(overrides: Partial<UserMessage> = {}): UserMessage {
  return { id: 'u1', createdAt: 0, role: 'user', content: 'go', ...overrides }
}

function thought(): ThoughtMessage {
  return {
    id: 't1',
    createdAt: 0,
    role: 'thought',
    content: 'scouting the workers tree',
    durationMs: 180,
  }
}

describe('functionTriggerToText', () => {
  it('renders the id and pretty-printed arguments', () => {
    expect(functionTriggerToText(fcall())).toBe(
      'ƒ shell::exec\n{\n  "command": "npm test"\n}',
    )
  })

  it('omits the argument block when input is empty', () => {
    expect(functionTriggerToText(fcall({ input: {} }))).toBe('ƒ shell::exec')
    expect(functionTriggerToText(fcall({ input: null }))).toBe('ƒ shell::exec')
    expect(functionTriggerToText(fcall({ input: undefined }))).toBe(
      'ƒ shell::exec',
    )
  })
})

describe('assistantCopyText', () => {
  it('returns the prose unchanged when there are no calls', () => {
    expect(assistantCopyText('hello', [])).toBe('hello')
  })

  it('appends each call under the prose, blank-line separated', () => {
    const text = assistantCopyText('running the tests', [
      fcall({ functionId: 'shell::exec', input: { command: 'npm test' } }),
      fcall({
        id: 'f2',
        functionId: 'coder::read-file',
        input: { path: '/a.ts' },
      }),
    ])
    expect(text).toBe(
      'running the tests\n\n' +
        'ƒ shell::exec\n{\n  "command": "npm test"\n}\n\n' +
        'ƒ coder::read-file\n{\n  "path": "/a.ts"\n}',
    )
  })

  it('drops the leading blank line when the message has no prose', () => {
    expect(assistantCopyText('', [fcall({ input: {} })])).toBe('ƒ shell::exec')
  })
})

describe('functionTriggersByAssistant', () => {
  it('maps an assistant to the run of calls that immediately follows it', () => {
    const a = assistant({ id: 'a1' })
    const c1 = fcall({ id: 'c1' })
    const c2 = fcall({ id: 'c2' })
    expect(functionTriggersByAssistant([a, c1, c2]).get('a1')).toEqual([c1, c2])
  })

  it('starts a fresh run at the next assistant message', () => {
    const a1 = assistant({ id: 'a1' })
    const c1 = fcall({ id: 'c1' })
    const a2 = assistant({ id: 'a2' })
    const c2 = fcall({ id: 'c2' })
    const map = functionTriggersByAssistant([a1, c1, a2, c2])
    expect(map.get('a1')).toEqual([c1])
    expect(map.get('a2')).toEqual([c2])
  })

  it('does not attach calls separated from the assistant by another role', () => {
    const a1 = assistant({ id: 'a1' })
    const c1 = fcall({ id: 'c1' })
    expect(functionTriggersByAssistant([a1, user(), c1]).has('a1')).toBe(false)
  })

  it('attributes leading calls forward to the turn-closing assistant', () => {
    // The canonical agent flow: thought → calls → summarizing prose.
    const c1 = fcall({ id: 'c1' })
    const a1 = assistant({ id: 'a1' })
    const map = functionTriggersByAssistant([user(), thought(), c1, a1])
    expect(map.get('a1')).toEqual([c1])
  })

  it('treats thought messages as transparent within a trailing run', () => {
    const a1 = assistant({ id: 'a1' })
    const c1 = fcall({ id: 'c1' })
    expect(functionTriggersByAssistant([a1, thought(), c1]).get('a1')).toEqual([
      c1,
    ])
  })

  it('drops leading calls when the turn ends without an assistant', () => {
    const map = functionTriggersByAssistant([
      fcall({ id: 'c1' }),
      user(),
      assistant({ id: 'a1' }),
    ])
    expect(map.size).toBe(0)
  })

  it('combines leading and trailing runs chronologically', () => {
    const c1 = fcall({ id: 'c1' })
    const a1 = assistant({ id: 'a1' })
    const c2 = fcall({ id: 'c2' })
    expect(functionTriggersByAssistant([user(), c1, a1, c2]).get('a1')).toEqual([
      c1,
      c2,
    ])
  })

  it('prefers trailing attribution between two assistants', () => {
    const a1 = assistant({ id: 'a1' })
    const c1 = fcall({ id: 'c1' })
    const a2 = assistant({ id: 'a2' })
    const map = functionTriggersByAssistant([a1, c1, a2])
    expect(map.get('a1')).toEqual([c1])
    expect(map.has('a2')).toBe(false)
  })
})
