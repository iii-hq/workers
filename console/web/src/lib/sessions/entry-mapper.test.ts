import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import {
  applyEntryUpsert,
  applyFcallPatch,
  clearTransientFlags,
  entrySegments,
  splitReactionTask,
  transcriptToMessages,
} from './entry-mapper'
import type { AgentMessage, TranscriptItem } from './types'

function userItem(
  entryId: string,
  text: string,
  origin?: TranscriptItem['origin'],
): TranscriptItem {
  return {
    entry_id: entryId,
    ...(origin ? { origin } : {}),
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

  it('collapses attached-file blocks into chips instead of dumping content', () => {
    const item: TranscriptItem = {
      entry_id: 'msg-2-user-0',
      message: {
        role: 'user',
        content: [
          { type: 'text', text: 'review #file(src/a.rs) please' },
          {
            type: 'text',
            text: '<attached-file path="src/a.rs" size="12" total-lines="1">\nfn main() {}\n</attached-file>',
          },
          {
            type: 'text',
            text: '<attached-file path="gone.txt" error="not found" />',
          },
        ],
        timestamp: 1,
      },
    }
    const [msg] = entrySegments(item)
    expect(msg).toMatchObject({
      role: 'user',
      content: 'review #file(src/a.rs) please',
      attachments: [
        {
          id: 'mention-src/a.rs',
          name: 'src/a.rs',
          size: 12,
          type: 'text/x-file-mention',
        },
        {
          id: 'mention-gone.txt',
          name: 'gone.txt (not found)',
          size: 0,
          type: 'text/x-file-mention',
        },
      ],
    })
    expect((msg as { content: string }).content).not.toContain('fn main')
  })

  it('marks only trusted notification user entries', () => {
    expect(
      entrySegments(userItem('e-1', 'normal', { notification: false }))[0],
    ).not.toHaveProperty('notification')
    expect(
      entrySegments(userItem('e-2', 'wake', { notification: true }))[0],
    ).toMatchObject({ notification: true })
    expect(entrySegments(userItem('e_notify_sub_1', 'wake'))[0]).toMatchObject({
      notification: true,
    })
  })

  it('splits a reaction task from its appended event block', () => {
    // The exact format react.rs produces (single_event_task).
    const content =
      'Present the results.\n\n<event>\n```json\n{"session_id":"reviewer-1","status":"completed"}\n```\n</event>'
    const [msg] = entrySegments(userItem('e_react_1', content))
    expect(msg).toMatchObject({
      reaction: true,
      content: 'Present the results.',
      reactionEvent: {
        label: 'event',
        json: JSON.stringify(
          { session_id: 'reviewer-1', status: 'completed' },
          null,
          2,
        ),
      },
    })
  })

  it('splitReactionTask handles inputs, collapsed whitespace, and bad JSON', () => {
    // Join variant (gather_inputs_task).
    expect(
      splitReactionTask(
        'Combine.\n\n<inputs>\n```json\n{"a":1}\n```\n</inputs>',
      ),
    ).toEqual({
      task: 'Combine.',
      appendix: { label: 'inputs', json: '{\n  "a": 1\n}' },
    })
    // Whitespace collapsed onto one line (as rendered markdown re-serializes).
    expect(
      splitReactionTask('Do it. <event> ```json {"x":1} ``` </event>').appendix
        ?.label,
    ).toBe('event')
    // Invalid JSON stays raw instead of disappearing.
    expect(
      splitReactionTask('T\n\n<event>\n```json\nnot-json{\n```\n</event>')
        .appendix?.json,
    ).toBe('not-json{')
    // No appendix → untouched.
    expect(splitReactionTask('plain task')).toEqual({ task: 'plain task' })
  })

  it('marks react-fired task entries as reactions', () => {
    expect(
      entrySegments(userItem('e-1', 'do the thing', { reaction: true }))[0],
    ).toMatchObject({ reaction: true })
    expect(entrySegments(userItem('e_react_ab12', 'do it'))[0]).toMatchObject({
      reaction: true,
    })
    expect(
      entrySegments(userItem('e-2', 'typed by hand'))[0],
    ).not.toHaveProperty('reaction')
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

  it('flags an agent_trigger whose target is not resolvable yet', () => {
    // Providers degrade partial/streaming JSON arguments to `{}`, so the
    // wrapped target function is unknown until the stream finishes.
    const [seg] = entrySegments(
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'agent_trigger',
          arguments: {},
        },
      ]),
    )
    expect(seg).toMatchObject({
      role: 'function-call',
      functionId: 'agent_trigger',
      unresolvedTarget: true,
    })
    // A resolvable wrapper carries no flag.
    const [resolved] = entrySegments(
      assistantItem('e-b', [
        {
          type: 'function_call',
          id: 'fc-2',
          function_id: 'agent_trigger',
          arguments: { function: 'shell::run', payload: {} },
        },
      ]),
    )
    expect(resolved).toMatchObject({
      functionId: 'shell::run',
      unresolvedTarget: undefined,
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

  it('absorbs a locally-created fcall row (pending approval) into the entry segment', () => {
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

  it('carries filesystemAccess through a snapshot re-derivation while pending', () => {
    const local: Message = {
      id: 'local-1',
      role: 'function-call',
      functionId: 'shell::fs::read',
      input: {},
      running: false,
      pendingApproval: true,
      functionCallId: 'fc-1',
      sessionId: 'sess-a',
      filesystemAccess: {
        requestedRoot: '/abs/existing/dir',
        errorCode: 'S215',
      },
      createdAt: 0,
    }
    const next = applyEntryUpsert(
      [local],
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::fs::read',
          arguments: {},
        },
      ]),
    )
    expect(next).toHaveLength(1)
    expect(next[0]).toMatchObject({
      id: 'e-a:0',
      pendingApproval: true,
      functionCallId: 'fc-1',
      filesystemAccess: {
        requestedRoot: '/abs/existing/dir',
        errorCode: 'S215',
      },
    })
  })

  it('infers running for unpaired calls while the session is working', () => {
    const messages = applyEntryUpsert(
      [],
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
      { working: true },
    )
    expect(messages[0]).toMatchObject({
      role: 'function-call',
      running: true,
    })
  })

  it('does not infer running when the session is not working', () => {
    const messages = applyEntryUpsert(
      [],
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
    )
    expect((messages[0] as { running?: boolean }).running).toBeUndefined()
  })

  it('clears inferred running when the function_result pairs in', () => {
    let messages = applyEntryUpsert(
      [],
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
      { working: true },
    )
    messages = applyEntryUpsert(messages, resultItem('fr-fc-1', 'fc-1', 'ok'), {
      working: true,
    })
    expect(messages).toHaveLength(1)
    expect(messages[0]).toMatchObject({ running: false })
    // A later snapshot of the same entry keeps the paired call done.
    messages = applyEntryUpsert(
      messages,
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'shell::run',
          arguments: {},
        },
      ]),
      { working: true },
    )
    expect(messages[0]).toMatchObject({ running: false })
  })

  it('never marks a pending-approval call as running', () => {
    const local: Message = {
      id: 'local-1',
      role: 'function-call',
      functionId: 'shell::run',
      input: {},
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
      { working: true },
    )
    expect(next[0]).toMatchObject({ pendingApproval: true })
    expect((next[0] as { running?: boolean }).running).toBeFalsy()
  })
})

describe('transcriptToMessages — running inference on hydration', () => {
  const call = (id: string) =>
    ({
      type: 'function_call',
      id,
      function_id: 'shell::run',
      arguments: {},
    }) as const

  it('marks unpaired calls of the last assistant entry while working', () => {
    const messages = transcriptToMessages(
      [
        userItem('e-u', 'go'),
        assistantItem('e-a', [call('fc-1')]),
        resultItem('fr-fc-1', 'fc-1', 'ok'),
        assistantItem('e-b', [call('fc-2')]),
      ],
      'sess-1',
      { working: true },
    )
    const rows = messages.filter((m) => m.role === 'function-call')
    expect(rows[0]).toMatchObject({ functionCallId: 'fc-1', running: false })
    expect(rows[1]).toMatchObject({ functionCallId: 'fc-2', running: true })
  })

  it('does not pulse historical unpaired calls (earlier entries or idle sessions)', () => {
    // Unpaired call in an EARLIER entry (interrupted turn) stays still even
    // while working.
    const working = transcriptToMessages(
      [
        assistantItem('e-a', [call('fc-1')]),
        assistantItem('e-b', [{ type: 'text', text: 'done' }]),
      ],
      'sess-1',
      { working: true },
    )
    const row = working.find((m) => m.role === 'function-call')
    expect((row as { running?: boolean }).running).toBeFalsy()

    // Idle session: nothing pulses.
    const idle = transcriptToMessages(
      [assistantItem('e-a', [call('fc-1')])],
      'sess-1',
    )
    expect((idle[0] as { running?: boolean }).running).toBeUndefined()
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
