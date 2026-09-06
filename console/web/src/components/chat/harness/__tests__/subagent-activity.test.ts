import { describe, expect, it } from 'vitest'
import { formatElapsed } from '@/lib/relative-time'
import type {
  AgentMessage,
  ContentBlock,
  TranscriptItem,
} from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import {
  activityFromAgentMessage,
  displayedSubagentActivity,
  latestSubagentActivity,
  resolveChildSessionId,
} from '../subagent-activity'

function assistant(content: ContentBlock[]): AgentMessage {
  return {
    role: 'assistant',
    content,
    stop_reason: 'end',
    model: 'test-model',
    provider: 'test-provider',
    timestamp: 10,
  }
}

function conversation(overrides: Partial<Conversation>): Conversation {
  return {
    id: 'child-1',
    title: 'child',
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  }
}

describe('sub-agent activity', () => {
  it('formats millisecond and legacy second timestamps as compact elapsed copy', () => {
    const now = Date.UTC(2026, 7, 19, 12, 2)

    expect(formatElapsed(now - 5_000, now)).toBe('just now')
    expect(formatElapsed(now - 45_000, now)).toBe('45s')
    expect(formatElapsed(now - 120_000, now)).toBe('2m')
    expect(formatElapsed((now - 7_200_000) / 1000, now)).toBe('2h')
    expect(formatElapsed(undefined, now)).toBeNull()
  })

  it('maps the latest assistant block to thinking, messaging, or work', () => {
    expect(
      activityFromAgentMessage(
        assistant([{ type: 'thinking', text: 'checking the plan' }]),
      )?.kind,
    ).toBe('thinking')
    expect(
      activityFromAgentMessage(
        assistant([{ type: 'text', text: 'Here is the update' }]),
      )?.kind,
    ).toBe('messaging')
    expect(
      activityFromAgentMessage(
        assistant([
          {
            type: 'function_call',
            id: 'call-1',
            function_id: 'coder::tree',
            arguments: {},
          },
        ]),
      )?.kind,
    ).toBe('working')
  })

  it('uses the newest durable transcript activity when mounted mid-turn', () => {
    const items: TranscriptItem[] = [
      { entry_id: 'one', message: assistant([{ type: 'text', text: 'old' }]) },
      {
        entry_id: 'two',
        message: assistant([{ type: 'thinking', text: 'new' }]),
      },
    ]

    expect(latestSubagentActivity(items)?.kind).toBe('thinking')
  })

  it('lets terminal session status override stale streaming activity', () => {
    const signal = { kind: 'messaging' as const, timestamp: 10 }
    expect(
      displayedSubagentActivity(conversation({ status: 'working' }), signal),
    ).toBe('messaging')
    expect(
      displayedSubagentActivity(conversation({ status: 'done' }), signal),
    ).toBe('ended')
    expect(
      displayedSubagentActivity(
        conversation({ status: 'done', statusReason: 'stopped' }),
        signal,
      ),
    ).toBe('stopped')
    expect(
      displayedSubagentActivity(conversation({ status: 'error' }), signal),
    ).toBe('error')
    expect(
      displayedSubagentActivity(
        conversation({ status: 'working' }),
        signal,
        'reconnecting',
      ),
    ).toBe('disconnected')
  })

  it('uses the same queued and waiting states as composer chips', () => {
    expect(
      displayedSubagentActivity(conversation({ status: 'idle' }), null),
    ).toBe('queued')
    expect(
      displayedSubagentActivity(
        conversation({
          status: 'working',
          statusReason: 'waiting for model',
        }),
        null,
      ),
    ).toBe('waiting')
  })
})

describe('child session correlation', () => {
  const conversations = [
    conversation({
      id: 'child-linked',
      parentId: 'parent-1',
      parentFunctionCallId: 'call-1',
    }),
  ]

  it('prefers the explicit response and request session ids', () => {
    expect(
      resolveChildSessionId({
        responseSessionId: 'child-response',
        requestSessionId: 'child-request',
        functionTriggerId: 'call-1',
        conversations,
      }),
    ).toBe('child-response')
    expect(
      resolveChildSessionId({
        requestSessionId: 'child-request',
        functionTriggerId: 'call-1',
        conversations,
      }),
    ).toBe('child-request')
  })

  it('recovers historical child ids from the durable parent call link', () => {
    expect(
      resolveChildSessionId({
        parentSessionId: 'parent-1',
        functionTriggerId: 'call-1',
        conversations,
      }),
    ).toBe('child-linked')
    expect(
      resolveChildSessionId({
        parentSessionId: 'another-parent',
        functionTriggerId: 'call-1',
        conversations,
      }),
    ).toBeNull()
  })
})
