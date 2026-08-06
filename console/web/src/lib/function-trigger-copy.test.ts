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

  it('runs the input through `redact` before serializing, when given', () => {
    const SECRET = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
    const call = fcall({
      functionId: 'sandbox-code-runner::run',
      input: { runtime_id: SECRET, code: '1+1' },
    })
    const redact = (v: unknown) =>
      JSON.parse(JSON.stringify(v).replaceAll(SECRET, 'rt-3f9a…'))
    const text = functionTriggerToText(call, redact)
    expect(text).not.toContain(SECRET)
    expect(text).toContain('rt-3f9a…')
  })

  it('is untouched by an absent redact function (the common case)', () => {
    expect(functionTriggerToText(fcall())).toBe(
      functionTriggerToText(fcall(), undefined),
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

  describe('redactFor', () => {
    const SECRET = 'rt-3f9a2c1e-7b64-4d0a-9c11-5e8ab2d4f077'
    const MASKED = 'rt-3f9a…'
    const mask = (v: unknown) =>
      JSON.parse(JSON.stringify(v).replaceAll(SECRET, MASKED))
    /** Only claims sandbox-code-runner::run — proves dispatch is per-call, not global. */
    const redactFor = (functionId: string) =>
      functionId === 'sandbox-code-runner::run' ? mask : undefined

    it('redacts a claimed call and leaves an unclaimed one untouched', () => {
      const text = assistantCopyText(
        'ran it',
        [
          fcall({
            id: 'c1',
            functionId: 'sandbox-code-runner::run',
            input: { runtime_id: SECRET, code: '1+1' },
          }),
          fcall({
            id: 'c2',
            functionId: 'shell::exec',
            input: { command: `echo ${SECRET}` },
          }),
        ],
        redactFor,
      )
      // The claimed call's secret never appears in the eval block…
      expect(text).toContain(MASKED)
      // …but the unclaimed shell call is untouched (no redactor claims it) —
      // proves dispatch is per-functionId, not a blanket scrub.
      expect(text).toContain(SECRET)
    })

    it('never redacts the assistant prose itself', () => {
      const proseWithLookalike = `see ${SECRET}`
      const text = assistantCopyText(
        proseWithLookalike,
        [fcall({ functionId: 'shell::exec', input: {} })],
        () => mask,
      )
      expect(text.startsWith(proseWithLookalike)).toBe(true)
    })

    it('is the same as calling with no redactFor at all when every call is unclaimed', () => {
      const calls = [fcall({ functionId: 'shell::exec' })]
      expect(assistantCopyText('hi', calls, () => undefined)).toBe(
        assistantCopyText('hi', calls),
      )
    })
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
    expect(functionTriggersByAssistant([user(), c1, a1, c2]).get('a1')).toEqual(
      [c1, c2],
    )
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
