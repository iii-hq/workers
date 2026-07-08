import { describe, expect, it } from 'vitest'
import type { SessionMeta } from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import {
  appendMessageToConversation,
  applyCatalogModelFallback,
  mergeConversationMeta,
} from './use-conversations'

function conversation(overrides: Partial<Conversation>): Conversation {
  return {
    id: 'console-1',
    title: 'old session',
    model: 'provider::removed-model',
    mode: 'agent',
    messages: [],
    status: 'done',
    hydrated: false,
    createdAt: 1_000,
    updatedAt: 2_000,
    ...overrides,
  }
}

function sessionMeta(overrides: Partial<SessionMeta>): SessionMeta {
  return {
    session_id: 'console-1',
    title: 'server title',
    description: '',
    status: 'idle',
    metadata: {},
    created_at: 1_000,
    updated_at: 2_000,
    message_count: 0,
    ...overrides,
  }
}

describe('applyCatalogModelFallback', () => {
  it('preserves activity timestamps when replacing stale model ids', () => {
    const fallback = 'provider::current-model'
    const sessions = [
      conversation({
        id: 'stale-model',
        model: 'provider::removed-model',
        updatedAt: 2_000,
      }),
      conversation({
        id: 'draft-missing-model',
        model: null,
        draft: true,
        updatedAt: 3_000,
      }),
    ]

    const next = applyCatalogModelFallback(
      sessions,
      new Set([fallback]),
      fallback,
    )

    expect(next.map((c) => c.model)).toEqual([fallback, fallback])
    expect(next.map((c) => c.updatedAt)).toEqual([2_000, 3_000])
  })

  it('never invents a model for a discovered session (sub-agents)', () => {
    const fallback = 'provider::current-model'
    const sessions = [
      conversation({
        id: 'summarizer-k7m2x',
        model: null,
        updatedAt: 3_000,
      }),
    ]

    const next = applyCatalogModelFallback(
      sessions,
      new Set([fallback]),
      fallback,
    )

    // The chat view derives the display model from the transcript instead.
    expect(next[0].model).toBeNull()
    expect(next).toBe(sessions)
  })
})

describe('mergeConversationMeta', () => {
  it('repairs a stale idle row from authoritative session metadata', () => {
    const existing = conversation({
      status: 'idle',
      messages: [
        {
          id: 'm1',
          role: 'user',
          content: 'run it',
          createdAt: 1_500,
        },
      ],
      hydrated: true,
      updatedAt: 1_500,
    })

    const next = mergeConversationMeta(
      existing,
      sessionMeta({
        status: 'working',
        updated_at: 4_000,
        metadata: {
          model: 'provider::current-model',
          mode: 'agent',
          parent_session_id: 'console-parent',
          depth: 1,
        },
      }),
    )

    expect(next.status).toBe('working')
    expect(next.updatedAt).toBe(4_000)
    expect(next.parentId).toBe('console-parent')
    expect(next.messages).toBe(existing.messages)
    expect(next.hydrated).toBe(true)
  })
})

describe('appendMessageToConversation', () => {
  it('marks a session working as soon as the user send is appended', () => {
    const next = appendMessageToConversation(
      conversation({ status: 'idle', messages: [] }),
      {
        id: 'm1',
        role: 'user',
        content: 'start',
        createdAt: 3_000,
      },
      3_500,
    )

    expect(next.status).toBe('working')
    expect(next.statusReason).toBeUndefined()
    expect(next.updatedAt).toBe(3_500)
  })
})
