import { describe, expect, it } from 'vitest'
import type { AgentEvent } from '@/types/iii-agent-event'
import { createAgentEventTranslator } from './translate'

describe('createAgentEventTranslator — message_complete', () => {
  const { translate } = createAgentEventTranslator()

  const baseAssistant = {
    role: 'assistant' as const,
    content: [{ type: 'text' as const, text: 'partial reply…' }],
    model: 'qwen/qwen3-4b-2507',
    provider: 'lmstudio',
    timestamp: 0,
  }

  it('emits ONLY assistant-end for a clean stop_reason="end" when body was streamed', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'end' },
      body_streamed: true,
    }
    expect(translate(event)).toEqual([{ kind: 'assistant-end' }])
  })

  it('emits assistant-token blocks and assistant-end for a non-streamed batch message', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: {
        ...baseAssistant,
        stop_reason: 'end',
        content: [{ type: 'text', text: 'hello batch' }],
      },
      body_streamed: false,
    }
    expect(translate(event)).toEqual([
      { kind: 'assistant-token', token: 'hello batch' },
      { kind: 'assistant-end' },
    ])
  })

  it('emits assistant-end + stop-reason notice when the turn hit max_tokens (stop_reason="length")', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'length' },
      body_streamed: true,
    }
    const out = translate(event)
    expect(out[0]).toEqual({ kind: 'assistant-end' })
    expect(out[1]).toMatchObject({ kind: 'stop-reason', reason: 'length' })
  })

  it('emits assistant-end + stop-reason notice carrying error_message when stop_reason="error"', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: {
        ...baseAssistant,
        stop_reason: 'error',
        error_message:
          'lmstudio stream closed mid-response after ~3214 output tokens',
      },
      body_streamed: true,
    }
    const out = translate(event)
    expect(out[0]).toEqual({ kind: 'assistant-end' })
    expect(out[1]).toEqual({
      kind: 'stop-reason',
      reason: 'error',
      message: 'lmstudio stream closed mid-response after ~3214 output tokens',
    })
  })

  it('emits assistant-end + stop-reason on abort', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'aborted' },
      body_streamed: true,
    }
    const out = translate(event)
    expect(out).toHaveLength(2)
    expect((out[1] as { kind: string; reason: string }).reason).toBe('aborted')
  })

  it('does NOT emit a stop-reason notice for function_call (turn will continue)', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'function_call' },
      body_streamed: true,
    }
    expect(translate(event)).toEqual([{ kind: 'assistant-end' }])
  })

  it('returns [] for non-assistant message_complete (user/function_result messages)', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: {
        role: 'user',
        content: [{ type: 'text', text: 'hi' }],
        timestamp: 0,
      },
    }
    expect(translate(event)).toEqual([])
  })

  it('omits the error_message field when none was provided', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'length' },
      body_streamed: true,
    }
    const out = translate(event)
    expect(out[1]).toEqual({ kind: 'stop-reason', reason: 'length' })
  })
})

describe('createAgentEventTranslator — compaction_done', () => {
  const { translate } = createAgentEventTranslator()

  it('translates compaction_done to a single compaction StreamEvent carrying the summary + tokens_before', () => {
    const event: AgentEvent = {
      type: 'compaction_done',
      mode: 'async',
      summary_text: 'older turns about X and Y',
      tokens_before: 12_345,
      compaction_entry_id: 'entry-c-1',
      tail_start_id: 'entry-t-1',
    }
    expect(translate(event, 'sess-1')).toEqual([
      {
        kind: 'compaction',
        mode: 'async',
        summaryText: 'older turns about X and Y',
        tokensBefore: 12_345,
        compactionEntryId: 'entry-c-1',
        tailStartId: 'entry-t-1',
      },
    ])
  })

  it('preserves mode="sync" for pre-flight compactions so the UI can distinguish them', () => {
    const event: AgentEvent = {
      type: 'compaction_done',
      mode: 'sync',
      summary_text: 's',
      tokens_before: 1,
      compaction_entry_id: 'e',
      tail_start_id: null,
    }
    const out = translate(event, 'sess-x')
    expect(out).toHaveLength(1)
    expect(
      (out[0] as { kind: 'compaction'; mode: 'sync' | 'async' }).mode,
    ).toBe('sync')
  })

  it('passes through a null tail_start_id (no tail preserved)', () => {
    const event: AgentEvent = {
      type: 'compaction_done',
      mode: 'async',
      summary_text: 's',
      tokens_before: 0,
      compaction_entry_id: 'e',
      tail_start_id: null,
    }
    const out = translate(event, 'sess-y')
    expect(
      (out[0] as { kind: 'compaction'; tailStartId: string | null })
        .tailStartId,
    ).toBeNull()
  })
})

describe('createAgentEventTranslator — turn_state_changed', () => {
  it('emits fcall-start { pendingApproval: true } when a new entry appears', () => {
    const { translate } = createAgentEventTranslator()
    const event: AgentEvent = {
      type: 'turn_state_changed',
      event_type: 'state:updated',
      new_value: {
        state: 'function_awaiting_approval',
        awaiting_approval: [
          {
            function_call_id: 'fc-1',
            function_id: 'shell::fs::write',
            args: { path: '/tmp/x' },
          },
        ],
      },
      old_value: {
        state: 'function_execute',
        awaiting_approval: [],
      },
    }
    expect(translate(event, 'sess-a')).toEqual([
      {
        kind: 'fcall-start',
        functionId: 'shell::fs::write',
        input: { path: '/tmp/x' },
        pendingApproval: true,
        functionCallId: 'fc-1',
        sessionId: 'sess-a',
      },
    ])
  })

  it('emits nothing when the awaiting_approval list is unchanged', () => {
    const { translate } = createAgentEventTranslator()
    const same = {
      state: 'function_awaiting_approval',
      awaiting_approval: [
        { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
      ],
    }
    translate(
      {
        type: 'turn_state_changed',
        event_type: 'state:created',
        new_value: same,
      },
      'sess-a',
    )
    expect(
      translate(
        {
          type: 'turn_state_changed',
          event_type: 'state:updated',
          new_value: same,
          old_value: same,
        },
        'sess-a',
      ),
    ).toEqual([])
  })

  it('clears the pending prompt when a call leaves function_awaiting_approval', () => {
    const { translate } = createAgentEventTranslator()
    translate(
      {
        type: 'turn_state_changed',
        event_type: 'state:created',
        new_value: {
          state: 'function_awaiting_approval',
          awaiting_approval: [
            { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
          ],
        },
      },
      'sess-a',
    )
    // The resolved call drops out of awaiting_approval — its prompt must be
    // cleared explicitly. Relying on the matching function_execution_end is
    // unsafe: parallel/approval-resolved ends arrive uncorrelated to the card.
    expect(
      translate(
        {
          type: 'turn_state_changed',
          event_type: 'state:updated',
          new_value: { state: 'function_execute', awaiting_approval: [] },
          old_value: {
            state: 'function_awaiting_approval',
            awaiting_approval: [
              {
                function_call_id: 'fc-1',
                function_id: 'shell::shell',
                args: {},
              },
            ],
          },
        },
        'sess-a',
      ),
    ).toEqual([{ kind: 'fcall-approval-cleared', functionCallId: 'fc-1' }])
  })

  it('threads function_call_id onto fcall-end so the consumer can correlate', () => {
    const { translate } = createAgentEventTranslator()
    const out = translate(
      {
        type: 'function_execution_end',
        function_call_id: 'fc-7',
        function_id: 'shell::fs::ls',
        result: { content: [], details: {} },
        is_error: false,
        duration_ms: 30,
      },
      'sess-a',
    )
    expect(out).toEqual([
      {
        kind: 'fcall-end',
        output: { content: [], details: {} },
        durationMs: 30,
        functionCallId: 'fc-7',
      },
    ])
  })

  it('partitions mirrors by sessionId so two chats do not interfere', () => {
    const { translate } = createAgentEventTranslator()
    const pending = {
      state: 'function_awaiting_approval',
      awaiting_approval: [
        { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
      ],
    }
    translate(
      {
        type: 'turn_state_changed',
        event_type: 'state:created',
        new_value: pending,
      },
      'sess-a',
    )
    expect(
      translate(
        {
          type: 'turn_state_changed',
          event_type: 'state:created',
          new_value: pending,
        },
        'sess-b',
      ),
    ).toHaveLength(1)
  })
})
