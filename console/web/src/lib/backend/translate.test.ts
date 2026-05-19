import { describe, expect, it } from 'vitest'
import type { AgentEvent } from '@/types/iii-agent-event'
import { createTurnStateTranslator } from './translate'

describe('createTurnStateTranslator', () => {
  it('emits fcall-start { pendingApproval: true } when a new entry appears', () => {
    const translate = createTurnStateTranslator()
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
    const translate = createTurnStateTranslator()
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

  it('emits nothing when state leaves function_awaiting_approval (the orchestrator emits the matching function_execution_end)', () => {
    const translate = createTurnStateTranslator()
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
              { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
            ],
          },
        },
        'sess-a',
      ),
    ).toEqual([])
  })

  it('partitions mirrors by sessionId so two chats do not interfere', () => {
    const translate = createTurnStateTranslator()
    const pending = {
      state: 'function_awaiting_approval',
      awaiting_approval: [
        { function_call_id: 'fc-1', function_id: 'shell::shell', args: {} },
      ],
    }
    translate(
      { type: 'turn_state_changed', event_type: 'state:created', new_value: pending },
      'sess-a',
    )
    expect(
      translate(
        { type: 'turn_state_changed', event_type: 'state:created', new_value: pending },
        'sess-b',
      ),
    ).toHaveLength(1)
  })
})
