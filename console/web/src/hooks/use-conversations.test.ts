import { describe, expect, it } from 'vitest'
import { transcriptToMessages } from '@/lib/sessions/entry-mapper'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import {
  appendMessageToConversation,
  applyCatalogModelFallback,
  markBackgroundedStale,
  mergeConversationMeta,
  mergeHydratedTranscript,
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
  it('restores the parked composer draft from SessionMeta.draft', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({ draft: 'half-typed thought' }),
    )
    expect(next.draftText).toBe('half-typed thought')

    // Absent / empty server drafts map to "nothing to restore".
    expect(mergeConversationMeta(undefined, sessionMeta({})).draftText).toBe(
      undefined,
    )
    expect(
      mergeConversationMeta(undefined, sessionMeta({ draft: '' })).draftText,
    ).toBe(undefined)
  })

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

  it('maps metadata.spawned_by to the sidebar origin discriminant', () => {
    const spawned = (v: unknown) =>
      mergeConversationMeta(
        undefined,
        sessionMeta({
          metadata: { parent_session_id: 'console-parent', spawned_by: v },
        }),
      ).spawnedBy
    expect(spawned('trigger')).toBe('trigger')
    expect(spawned('agent')).toBe('agent')
    // Unknown/absent values (pre-stamp sessions) stay undefined.
    expect(spawned('something-else')).toBeUndefined()
    expect(spawned(undefined)).toBeUndefined()
  })
})

describe('markBackgroundedStale', () => {
  // Regression: transcript events subscribe for the ACTIVE session only, so
  // a session backgrounded mid-turn misses entry updates (a function call
  // freezes as `ƒ …` with empty request/response). Staling it on switch
  // makes re-activation re-hydrate from durable truth.
  it('marks hydrated backgrounded sessions stale, leaves the active one', () => {
    const sessions = [
      conversation({ id: 'active', hydrated: true }),
      conversation({ id: 'backgrounded', hydrated: true }),
      conversation({ id: 'draft', draft: true, hydrated: true }),
      conversation({ id: 'never-opened', hydrated: false }),
    ]

    const next = markBackgroundedStale(sessions, 'active')

    expect(next.find((c) => c.id === 'active')?.hydrated).toBe(true)
    expect(next.find((c) => c.id === 'backgrounded')?.hydrated).toBe(false)
    // Drafts are local-only (no server transcript to refetch).
    expect(next.find((c) => c.id === 'draft')?.hydrated).toBe(true)
    expect(next.find((c) => c.id === 'never-opened')?.hydrated).toBe(false)
  })

  it('returns the same array when nothing needs staling', () => {
    const sessions = [
      conversation({ id: 'active', hydrated: true }),
      conversation({ id: 'never-opened', hydrated: false }),
    ]
    expect(markBackgroundedStale(sessions, 'active')).toBe(sessions)
  })
})

describe('mergeHydratedTranscript', () => {
  const opts = { sessionId: 'console-1', working: false }

  function assistantItem(entryId: string, text: string): TranscriptItem {
    return {
      entry_id: entryId,
      message: {
        role: 'assistant',
        content: [{ type: 'text', text }],
        stop_reason: 'end',
        model: 'm',
        provider: 'p',
        timestamp: 2,
      },
    }
  }
  const toMessages = (items: TranscriptItem[]) =>
    transcriptToMessages(items, 'console-1', { working: false })

  // Regression: an update landing while the hydration fetch was in flight is
  // newer than the snapshot; without the replay the older read wins and
  // `hydrated: true` pins the stale text until the next session switch.
  it('replays a mid-fetch upsert over the older fetched snapshot', () => {
    const merged = mergeHydratedTranscript(
      toMessages([assistantItem('e1', 'old partial')]),
      [],
      [{ item: assistantItem('e1', 'final text'), updated: true }],
      opts,
    )
    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({ id: 'e1:0', content: 'final text' })
  })

  it('keeps live-only messages the read did not return', () => {
    const merged = mergeHydratedTranscript(
      toMessages([assistantItem('e1', 'a')]),
      toMessages([assistantItem('local-1', 'pending')]),
      [],
      opts,
    )
    expect(merged.map((m) => m.id)).toEqual(['e1:0', 'local-1:0'])
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

  it('upserts a durable lifecycle notice over its live fallback', () => {
    const next = appendMessageToConversation(
      conversation({
        messages: [
          {
            id: 'e_t-1_error',
            role: 'system',
            kind: 'notice',
            tone: 'error',
            content: 'response failed',
            createdAt: 3_000,
          },
        ],
      }),
      {
        id: 'e_t-1_error',
        role: 'system',
        kind: 'notice',
        tone: 'error',
        content: 'turn failed [llm.transient] — exact reason',
        createdAt: 3_100,
      },
    )

    expect(next.messages).toHaveLength(1)
    expect(next.messages[0]).toMatchObject({
      id: 'e_t-1_error',
      content: 'turn failed [llm.transient] — exact reason',
    })
  })
})
