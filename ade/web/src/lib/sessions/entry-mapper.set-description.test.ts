import { describe, expect, it } from 'vitest'
import {
  applyEntryUpsert,
  isHiddenFunctionId,
  transcriptToMessages,
} from './entry-mapper'
import type { TranscriptItem } from './types'

const assistant = (entryId: string, content: unknown[]): TranscriptItem =>
  ({
    entry_id: entryId,
    message: {
      role: 'assistant',
      content,
      timestamp: 1,
    },
  }) as unknown as TranscriptItem

const result = (
  entryId: string,
  callId: string,
  functionId: string,
): TranscriptItem =>
  ({
    entry_id: entryId,
    message: {
      role: 'function_result',
      function_call_id: callId,
      function_id: functionId,
      content: [{ type: 'text', text: '{"ok":true}' }],
      is_error: false,
      timestamp: 2,
    },
  }) as unknown as TranscriptItem

describe('harness::set-description stays out of the transcript (§1.2.3)', () => {
  it('names the description write in every tool mode', () => {
    expect(isHiddenFunctionId('harness::set-description')).toBe(true)
    expect(isHiddenFunctionId('harness__set-description')).toBe(true)
    expect(isHiddenFunctionId('shell::exec')).toBe(false)
  })

  it('drops the call and its result, and keeps the reply text', () => {
    const messages = transcriptToMessages(
      [
        assistant('e1', [
          { type: 'text', text: 'Done: the retries are in.' },
          {
            type: 'function_call',
            id: 'call-1',
            function_id: 'harness::set-description',
            arguments: { description: 'Retries with backoff' },
          },
        ]),
        result('e2', 'call-1', 'harness::set-description'),
      ],
      's1',
    )
    expect(messages.map((m) => m.role)).toEqual(['assistant'])
    expect(messages[0]).toMatchObject({ content: 'Done: the retries are in.' })
  })

  it('hides it when an agent_call wraps it, and keeps the other calls', () => {
    const messages = transcriptToMessages(
      [
        assistant('e1', [
          {
            type: 'function_call',
            id: 'call-1',
            function_id: 'agent_call',
            arguments: {
              function: 'harness::set-description',
              payload: { description: 'x' },
            },
          },
          {
            type: 'function_call',
            id: 'call-2',
            function_id: 'shell::exec',
            arguments: { command: 'ls' },
          },
        ]),
        result('e2', 'call-1', 'harness::set-description'),
        result('e3', 'call-2', 'shell::exec'),
      ],
      's1',
    )
    expect(messages).toHaveLength(1)
    expect(messages[0]).toMatchObject({
      role: 'function-call',
      functionId: 'shell::exec',
    })
  })

  it('adds no standalone row for a result whose call is not loaded', () => {
    expect(
      applyEntryUpsert([], result('e9', 'call-9', 'harness::set-description')),
    ).toEqual([])
  })

  it('hides an elided call too (a paged read)', () => {
    const item = {
      ...assistant('e1', [
        {
          type: 'function_call',
          id: 'call-1',
          function_id: 'harness::set-description',
        },
      ]),
      elided: true,
    } as TranscriptItem
    expect(transcriptToMessages([item], 's1')).toEqual([])
  })
})
