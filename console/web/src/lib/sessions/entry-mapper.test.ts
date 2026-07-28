import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import {
  applyEntryUpsert,
  applyFcallPatch,
  clearTransientFlags,
  entrySegments,
  splitReactionTask,
  transcriptToMessages,
  triggerFiredSummary,
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
  functionTriggerId: string,
  text: string,
  isError = false,
): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'function_result',
      function_call_id: functionTriggerId,
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
    // The exact format spawn.rs produces (single_event_task).
    const content =
      'Present the results.\n\n<event>\n```json\n{"session_id":"reviewer-1","status":"completed"}\n```\n</event>'
    const [msg] = entrySegments(userItem('e_spawned_1', content))
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
    expect(entrySegments(userItem('e_spawned_ab12', 'do it'))[0]).toMatchObject(
      {
        reaction: true,
      },
    )
    expect(
      entrySegments(userItem('e-2', 'typed by hand'))[0],
    ).not.toHaveProperty('reaction')
  })

  it('marks direct-spawn seed tasks (origin on events, prefix on reads)', () => {
    expect(
      entrySegments(userItem('e-1', 'build the report', { spawn: true }))[0],
    ).toMatchObject({ spawn: true })
    expect(
      entrySegments(userItem('e_spawn_ab12', 'build it'))[0],
    ).toMatchObject({ spawn: true })
    expect(
      entrySegments(userItem('e-2', 'typed by hand'))[0],
    ).not.toHaveProperty('spawn')
  })

  it('hides the machine-authored transient recovery prompt', () => {
    expect(
      entrySegments(
        userItem(
          'e_t-1_transient_resume_1',
          'Resume from the previous partial response',
        ),
      ),
    ).toEqual([])
  })

  it('splits an assistant entry into thought/text/function-trigger segments by block', () => {
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
      ['e-a:2', 'function-trigger'],
    ])
    expect(segments[2]).toMatchObject({
      functionId: 'shell::run',
      input: { command: 'ls' },
      functionTriggerId: 'fc-1',
      sessionId: 'sess-1',
    })
  })

  it('renders nothing for an empty assistant placeholder', () => {
    expect(entrySegments(assistantItem('e-a', []))).toEqual([])
  })

  // Mid-stream, the harness rides the raw in-flight args tail on
  // `_streaming` beside the salvaged fields — the request pane shows the
  // command forming instead of `empty` for the whole stream.
  it('surfaces the streaming arguments tail while wrapper args form', () => {
    const withTarget = entrySegments(
      assistantItem('e-a', [
        {
          type: 'function_call',
          id: 'fc-1',
          function_id: 'agent_trigger',
          arguments: {
            function: 'state::set',
            _streaming: '"payload":{"value":"grow',
          },
        },
      ]),
    )[0]
    expect(withTarget).toMatchObject({
      functionId: 'state::set',
      input: { _streaming: '"payload":{"value":"grow' },
    })
    expect(withTarget).not.toMatchObject({ unresolvedTarget: true })

    const noTarget = entrySegments(
      assistantItem('e-b', [
        {
          type: 'function_call',
          id: 'fc-2',
          function_id: 'agent_trigger',
          arguments: { _streaming: '{"fun' },
        },
      ]),
    )[0]
    expect(noTarget).toMatchObject({
      unresolvedTarget: true,
      input: { _streaming: '{"fun' },
    })
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

  it('maps a called reaction outcome as info, never the failed fallback', () => {
    const [notice] = entrySegments({
      entry_id: 'e-r',
      custom: {
        custom_type: 'reaction',
        data: { status: 'called', summary: 'Reaction dispatched fp::pipe.' },
      },
    })
    expect(notice).toMatchObject({
      id: 'e-r',
      role: 'system',
      kind: 'notice',
      content: 'reaction called — Reaction dispatched fp::pipe.',
      tone: 'info',
    })
  })

  it('maps role:custom read-back messages through the same typed dispatch', () => {
    // The exact `session::messages` wire shape: kind:custom entries come back
    // as a `role: 'custom'` MESSAGE (custom_type + details), not `item.custom`.
    const [err] = entrySegments({
      entry_id: 'e_t_faa6_error',
      message: {
        role: 'custom',
        custom_type: 'error',
        content: [],
        details: {
          reason:
            'remote error (router/provider_unavailable): provider zai unavailable',
        },
        timestamp: 9,
      },
    })
    expect(err).toMatchObject({
      role: 'system',
      tone: 'error',
      content:
        'turn failed — remote error (router/provider_unavailable): provider zai unavailable',
      createdAt: 9,
    })
    const [fired] = entrySegments({
      entry_id: 'e_trigfired_sub_9',
      message: {
        role: 'custom',
        custom_type: 'trigger_fired',
        content: [],
        details: {
          subscription_id: 'sub_9',
          target: 'spawn',
          once: true,
          retired: true,
          fired_at: 4,
        },
        timestamp: 4,
      },
    })
    expect(fired).toMatchObject({ role: 'system', kind: 'trigger-fired' })
    // Unknown custom types still fall back to their display text.
    const [note] = entrySegments({
      entry_id: 'e-x',
      message: {
        role: 'custom',
        custom_type: 'someother',
        content: [],
        display: 'hello from a worker',
        timestamp: 5,
      },
    })
    expect(note).toMatchObject({
      role: 'system',
      content: 'hello from a worker',
    })
  })

  it('maps harness error/notice custom entries to visible system notices', () => {
    const [err] = entrySegments({
      entry_id: 'e_t1_error',
      custom: {
        custom_type: 'error',
        data: { reason: 'provider zai unavailable' },
      },
    })
    expect(err).toMatchObject({
      role: 'system',
      kind: 'notice',
      tone: 'error',
      content: 'turn failed — provider zai unavailable',
    })
    const [notice] = entrySegments({
      entry_id: 'e_t1_max_turns',
      custom: {
        custom_type: 'notice',
        data: { reason: 'max_turns', message: 'max_turns (8) reached' },
      },
    })
    expect(notice).toMatchObject({
      role: 'system',
      tone: 'info',
      content: 'max_turns (8) reached',
    })
  })

  it('maps a trigger_fired custom entry to a turn-less notice carrying its record', () => {
    const [notice] = entrySegments({
      entry_id: 'e_trigfired_sub_1',
      custom: {
        custom_type: 'trigger_fired',
        data: {
          subscription_id: 'sub_1',
          trigger_id: 't-1',
          target: 'spawn',
          model: 'claude-sonnet-4-6',
          once: true,
          retired: true,
          scope: 'cache-repl-pipeline',
          key: 'facts',
          fired_at: 42,
        },
      },
    })
    expect(notice).toMatchObject({
      id: 'e_trigfired_sub_1',
      role: 'system',
      kind: 'trigger-fired',
      createdAt: 42,
      trigger: { subscription_id: 'sub_1', target: 'spawn', retired: true },
    })
    expect((notice as { content: string }).content).toBe(
      'cache-repl-pipeline/facts · spawned claude-sonnet-4-6 · unregistered',
    )
  })

  it('triggerFiredSummary reads spawn and notify targets', () => {
    expect(
      triggerFiredSummary({
        subscription_id: 's',
        target: 'spawn',
        model: 'claude-sonnet-5',
        once: false,
        retired: false,
        fired_at: 0,
      }),
    ).toBe('trigger · spawned claude-sonnet-5')
    expect(
      triggerFiredSummary({
        subscription_id: 's',
        target: 'notify',
        label: 'ping',
        once: true,
        retired: true,
        fired_at: 0,
      }),
    ).toBe('ping · notified this chat · unregistered')
  })

  it('renders a persisted failure with partial-output and recovery context', () => {
    const [notice] = entrySegments({
      entry_id: 'e-t-1-error',
      custom: {
        custom_type: 'error',
        data: {
          status: 'error',
          code: 'llm.transient',
          summary: 'stream ended without a terminal frame',
          partial_result_available: true,
          recovery: { attempted: 1, max_attempts: 1, outcome: 'exhausted' },
          timestamp: 42,
        },
      },
    })
    expect(notice).toMatchObject({
      id: 'e-t-1-error',
      role: 'system',
      tone: 'error',
      createdAt: 42,
    })
    if (notice.role !== 'system') throw new Error('expected a system notice')
    expect(notice.content).toContain('turn failed [llm.transient]')
    expect(notice.content).toContain('partial output above was preserved')
    expect(notice.content).toContain('recovery exhausted (1/1)')
  })

  it('renders recovery and blocked reaction lifecycle entries', () => {
    expect(
      entrySegments({
        entry_id: 'e-recovery',
        custom: {
          custom_type: 'recovery',
          data: { status: 'recovering', summary: 'resuming (1/1)' },
        },
      })[0],
    ).toMatchObject({ tone: 'warn', content: 'resuming (1/1)' })

    expect(
      entrySegments({
        entry_id: 'e-reaction',
        custom: {
          custom_type: 'reaction',
          data: { status: 'blocked', summary: 'facts stage failed' },
        },
      })[0],
    ).toMatchObject({
      tone: 'error',
      content: 'reaction blocked — facts stage failed',
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
      role: 'function-trigger',
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

  it('pairs a function_result entry into the matching function-trigger row', () => {
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
      role: 'function-trigger',
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
      role: 'function-trigger',
      functionId: 'shell::run',
      input: {},
      running: true,
      pendingApproval: true,
      functionTriggerId: 'fc-1',
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
      functionTriggerId: 'fc-1',
    })
  })

  it('carries filesystemAccess through a snapshot re-derivation while pending', () => {
    const local: Message = {
      id: 'local-1',
      role: 'function-trigger',
      functionId: 'shell::fs::read',
      input: {},
      running: false,
      pendingApproval: true,
      functionTriggerId: 'fc-1',
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
      functionTriggerId: 'fc-1',
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
      role: 'function-trigger',
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
      role: 'function-trigger',
      functionId: 'shell::run',
      input: {},
      pendingApproval: true,
      functionTriggerId: 'fc-1',
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
    const rows = messages.filter((m) => m.role === 'function-trigger')
    expect(rows[0]).toMatchObject({ functionTriggerId: 'fc-1', running: false })
    expect(rows[1]).toMatchObject({ functionTriggerId: 'fc-2', running: true })
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
    const row = working.find((m) => m.role === 'function-trigger')
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
  it('patches the row matching functionTriggerId and reports found', () => {
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
        role: 'function-trigger',
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
