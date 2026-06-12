import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import {
  applyEntryUpsert,
  applyFcallPatch,
  clearTransientFlags,
  entrySegments,
  transcriptToMessages,
} from './entry-mapper'
import type { AgentMessage, TranscriptItem } from './types'

function userItem(entryId: string, text: string): TranscriptItem {
  return {
    entry_id: entryId,
    message: { role: 'user', content: [{ type: 'text', text }], timestamp: 1 },
  }
}

function assistantItem(
  entryId: string,
  content: Extract<AgentMessage, { role: 'assistant' }>['content'],
): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'assistant',
      content,
      stop_reason: 'end',
      model: 'm',
      provider: 'p',
      timestamp: 2,
    },
  }
}

function resultItem(
  entryId: string,
  functionCallId: string,
  text: string,
  isError = false,
): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'function_result',
      function_call_id: functionCallId,
      function_id: 'shell::run',
      content: [{ type: 'text', text }],
      details: {},
      is_error: isError,
      timestamp: 3,
    },
  }
}

describe('entrySegments', () => {
  it('maps a user entry to one user message whose id IS the entry id', () => {
    const [msg] = entrySegments(userItem('msg-1-user-0', 'hello'))
    expect(msg).toMatchObject({
      id: 'msg-1-user-0',
      role: 'user',
      content: 'hello',
    })
  })

  it('splits an assistant entry into thought/text/function-call segments by block', () => {
    const segments = entrySegments(
      assistantItem('e-a', [
        { type: 'thinking', text: 'pondering' },
        { type: 'text', text: 'the answer' },
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'agent_trigger',
          arguments: { function: 'shell::run', payload: { command: 'ls' } },
        },
      ]),
      'sess-1',
    )
    expect(segments.map((s) => [s.id, s.role])).toEqual([
      ['e-a:0', 'thought'],
      ['e-a:1', 'assistant'],
      ['e-a:2', 'function-call'],
    ])
    // agent_trigger is unwrapped to the real target for display.
    expect(segments[2]).toMatchObject({
      functionId: 'shell::run',
      input: { command: 'ls' },
      functionCallId: 'fc-1',
      sessionId: 'sess-1',
    })
  })

  it('renders nothing for an empty assistant placeholder', () => {
    expect(entrySegments(assistantItem('e-a', []))).toEqual([])
  })

  it('maps a compaction custom entry to the compaction marker', () => {
    const [marker] = entrySegments({
      entry_id: 'e-c',
      custom: {
        custom_type: 'compaction',
        data: { summary: 'older turns', tokens_before: 1200, timestamp: 9 },
      },
    })
    expect(marker).toMatchObject({
      id: 'e-c',
      role: 'system',
      kind: 'compaction',
      summaryText: 'older turns',
      tokensBefore: 1200,
    })
  })
})

describe('applyEntryUpsert', () => {
  it('replaces the optimistic user message in place (predicted entry id)', () => {
    const optimistic: Message = {
      id: 'msg-1-user-0',
      role: 'user',
      content: 'hello',
      attachments: [{ id: 'a1', name: 'f.txt', size: 1, type: 'text/plain' }],
      createdAt: 0,
    }
    const next = applyEntryUpsert(
      [optimistic],
      userItem('msg-1-user-0', 'hello'),
    )
    expect(next).toHaveLength(1)
    expect(next[0]).toMatchObject({ id: 'msg-1-user-0', content: 'hello' })
    // Attachments are client-only; preserved across the replacement.
    expect((next[0] as { attachments?: unknown[] }).attachments).toHaveLength(1)
  })

  it("re-derives an entry's segments wholesale on update (streamed snapshots)", () => {
    let messages = applyEntryUpsert(
      [],
      assistantItem('e-a', [{ type: 'text', text: 'par' }]),
    )
    messages = applyEntryUpsert(
      messages,
      assistantItem('e-a', [{ type: 'text', text: 'partial reply' }]),
    )
    expect(messages).toHaveLength(1)
    expect(messages[0]).toMatchObject({ id: 'e-a:0', content: 'partial reply' })
  })

  it('keeps entry order when replacing a mid-path entry', () => {
    let messages = transcriptToMessages([
      userItem('e-u', 'hi'),
      assistantItem('e-a', [{ type: 'text', text: 'one' }]),
      userItem('e-u2', 'more'),
    ])
    messages = applyEntryUpsert(
      messages,
      assistantItem('e-a', [{ type: 'text', text: 'two' }]),
    )
    expect(messages.map((m) => m.id)).toEqual(['e-u', 'e-a:0', 'e-u2'])
    expect(messages[1]).toMatchObject({ content: 'two' })
  })

  it('pairs a function_result entry into the matching function-call row', () => {
    let messages = transcriptToMessages([
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'agent_trigger',
          arguments: { function: 'shell::run', payload: {} },
        },
      ]),
    ])
    messages = applyEntryUpsert(messages, resultItem('fr-fc-1', 'fc-1', 'ok'))
    expect(messages).toHaveLength(1)
    expect(messages[0]).toMatchObject({
      role: 'function-call',
      running: false,
      output: { content: [{ type: 'text', text: 'ok' }], details: {} },
    })
  })

  it('wraps errored function results in the error envelope', () => {
    let messages = transcriptToMessages([
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
    ])
    messages = applyEntryUpsert(
      messages,
      resultItem('fr-fc-1', 'fc-1', 'boom', true),
    )
    const row = messages[0] as { output?: { error?: { message?: string } } }
    expect(row.output?.error?.message).toBe('boom')
  })

  it('absorbs a locally-created fcall row (agent::events) into the entry segment', () => {
    // The live approval flow appended a local row before the assistant
    // snapshot arrived.
    const local: Message = {
      id: 'local-1',
      role: 'function-call',
      functionId: 'shell::run',
      input: {},
      running: true,
      pendingApproval: true,
      functionCallId: 'fc-1',
      createdAt: 0,
    }
    const next = applyEntryUpsert(
      [local],
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
    )
    expect(next).toHaveLength(1)
    expect(next[0]).toMatchObject({
      id: 'e-a:0',
      running: true,
      pendingApproval: true,
      functionCallId: 'fc-1',
    })
  })
})

describe('applyFcallPatch / clearTransientFlags', () => {
  it('patches the row matching functionCallId and reports found', () => {
    const messages = transcriptToMessages([
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
    ])
    const { messages: next, found } = applyFcallPatch(messages, 'fc-1', {
      running: true,
    })
    expect(found).toBe(true)
    expect(next[0]).toMatchObject({ running: true })
    expect(applyFcallPatch(next, 'fc-unknown', { running: true }).found).toBe(
      false,
    )
  })

  it('clears dangling streaming/running flags when the turn ends', () => {
    const messages: Message[] = [
      {
        id: 't',
        role: 'thought',
        content: 'x',
        durationMs: 0,
        streaming: true,
        createdAt: 0,
      },
      {
        id: 'a',
        role: 'assistant',
        content: 'y',
        streaming: true,
        createdAt: 0,
      },
      {
        id: 'f',
        role: 'function-call',
        functionId: 'shell::run',
        input: {},
        running: true,
        createdAt: 0,
      },
    ]
    const next = clearTransientFlags(messages)
    expect(
      next.map((m) => ('streaming' in m ? m.streaming : undefined)),
    ).toEqual([false, false, undefined])
    expect((next[2] as { running?: boolean }).running).toBe(false)
  })
})
