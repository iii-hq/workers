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

  it('emits nothing for a clean stop_reason="end" (content renders from session events)', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'end' },
      body_streamed: true,
    }
    expect(translate(event)).toEqual([])
  })

  it('does NOT replay content for non-streamed batch messages (session events own the transcript)', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: {
        ...baseAssistant,
        stop_reason: 'end',
        content: [{ type: 'text', text: 'hello batch' }],
      },
      body_streamed: false,
    }
    expect(translate(event)).toEqual([])
  })

  it('emits a stop-reason notice when the turn hit max_tokens (stop_reason="length")', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'length' },
      body_streamed: true,
    }
    expect(translate(event)).toEqual([
      { kind: 'stop-reason', reason: 'length' },
    ])
  })

  it('emits a stop-reason notice carrying error_message when stop_reason="error"', () => {
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
    expect(translate(event)).toEqual([
      {
        kind: 'stop-reason',
        reason: 'error',
        message:
          'lmstudio stream closed mid-response after ~3214 output tokens',
      },
    ])
  })

  it('emits a stop-reason notice on abort', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'aborted' },
      body_streamed: true,
    }
    const out = translate(event)
    expect(out).toHaveLength(1)
    expect((out[0] as { kind: string; reason: string }).reason).toBe('aborted')
  })

  it('does NOT emit a stop-reason notice for function_call (turn will continue)', () => {
    const event: AgentEvent = {
      type: 'message_complete',
      message: { ...baseAssistant, stop_reason: 'function_call' },
      body_streamed: true,
    }
    expect(translate(event)).toEqual([])
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
    expect(translate(event)).toEqual([
      { kind: 'stop-reason', reason: 'length' },
    ])
  })
})

describe('createAgentEventTranslator — message_update / compaction_done / agent_end', () => {
  const { translate } = createAgentEventTranslator()

  it('emits nothing for message_update — tokens render from session::message-updated snapshots', () => {
    const event: AgentEvent = {
      type: 'message_update',
      message: {
        role: 'assistant',
        content: [{ type: 'text', text: 'par' }],
        stop_reason: 'end',
        model: 'm',
        provider: 'p',
        timestamp: 0,
      },
      llm_event: {
        type: 'text_delta',
        delta: 'par',
        partial: {
          role: 'assistant',
          content: [],
          stop_reason: 'end',
          model: 'm',
          provider: 'p',
          timestamp: 0,
        },
      },
    }
    expect(translate(event)).toEqual([])
  })

  it('emits nothing for compaction_done — the marker renders from the compaction custom entry', () => {
    const event: AgentEvent = {
      type: 'compaction_done',
      mode: 'async',
      summary_text: 'older turns about X and Y',
      tokens_before: 12_345,
      compaction_entry_id: 'entry-c-1',
      tail_start_id: 'entry-t-1',
    }
    expect(translate(event, 'sess-1')).toEqual([])
  })

  it('maps agent_end to assistant-end (the turn-over signal)', () => {
    const event: AgentEvent = { type: 'agent_end', messages: [] }
    expect(translate(event)).toEqual([{ kind: 'assistant-end' }])
  })
})

describe('createAgentEventTranslator — function execution lifecycle', () => {
  const { translate } = createAgentEventTranslator()

  it('maps function_execution_end to fcall-end carrying the functionCallId', () => {
    const event: AgentEvent = {
      type: 'function_execution_end',
      function_call_id: 'fc-9',
      function_id: 'shell::run',
      result: { content: [{ type: 'text', text: 'ok' }], details: {} },
      is_error: false,
      duration_ms: 12,
    }
    expect(translate(event, 'sess-1')).toEqual([
      {
        kind: 'fcall-end',
        output: { content: [{ type: 'text', text: 'ok' }], details: {} },
        durationMs: 12,
        functionCallId: 'fc-9',
      },
    ])
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
