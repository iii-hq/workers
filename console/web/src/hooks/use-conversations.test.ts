import { describe, expect, it } from 'vitest'
import { DEFAULT_SYSTEM_PROMPT_STATE } from '@/components/chat/system-prompt-selection'
import { transcriptToMessages } from '@/lib/sessions/entry-mapper'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import {
  appendMessageToConversation,
  applyCatalogModelFallback,
  completeFailedHydration,
  isUntouchedDraft,
  markBackgroundedStale,
  mergeConversationMeta,
  mergeHydratedConversation,
  mergeHydratedTranscript,
  mergeSessionListSnapshot,
  metadataFor,
  preSendMetaUpdate,
  resolveActiveConversationId,
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

  it('degrades a removed/unknown persisted mode to the default', () => {
    // Pre-upgrade sessions parked in the removed "plan" mode (and any garbage
    // value) must fall back to DEFAULT_MODE, not crash or carry the dead value.
    expect(
      mergeConversationMeta(
        undefined,
        sessionMeta({ metadata: { mode: 'plan' } }),
      ).mode,
    ).toBe('agent')
    expect(
      mergeConversationMeta(
        undefined,
        sessionMeta({ metadata: { mode: 'xyz' } }),
      ).mode,
    ).toBe('agent')
    // A still-valid mode is preserved.
    expect(
      mergeConversationMeta(
        undefined,
        sessionMeta({ metadata: { mode: 'ask' } }),
      ).mode,
    ).toBe('ask')
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

  it('does not downgrade or migrate a started session from a stale zero-count snapshot', () => {
    const next = mergeConversationMeta(
      conversation({ started: true, hydrated: true }),
      sessionMeta({
        message_count: 0,
        metadata: {
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
          },
        },
      }),
    )

    expect(next.started).toBe(true)
    expect(next.skills).toBeUndefined()
    expect(next.systemPrompt?.addons).toEqual([
      { kind: 'skill', name: 'review', body: 'legacy body' },
    ])
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

  it('preserves the parent function call used to open a spawned child', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({
        metadata: {
          parent_session_id: 'console-parent',
          function_call_id: 'call-spawn-1',
          spawned_by: 'agent',
        },
      }),
    )

    expect(next.parentId).toBe('console-parent')
    expect(next.parentFunctionCallId).toBe('call-spawn-1')
  })
})

describe('mergeSessionListSnapshot', () => {
  it('preserves a session created while the initial list request was in flight', () => {
    const draft = conversation({ id: 'draft', draft: true })
    const concurrent = conversation({
      id: 'created-live',
      title: 'created live',
    })
    const listed = sessionMeta({ session_id: 'listed' })

    const next = mergeSessionListSnapshot([draft, concurrent], [listed])

    expect(next.map((item) => item.id)).toEqual([
      'draft',
      'created-live',
      'listed',
    ])
  })
})

describe('markBackgroundedStale', () => {
  // Regression: transcript events subscribe for the ACTIVE session only, so
  // a session backgrounded mid-turn misses entry updates (a function trigger
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

  it('keeps a legacy migration unmodified when hidden durable hydration proves the session started', () => {
    const hidden: TranscriptItem = {
      entry_id: 'e_t1_transient_resume_1',
      message: {
        role: 'user',
        content: [{ type: 'text', text: 'resume internally' }],
        timestamp: 2,
      },
    }
    const candidate = mergeConversationMeta(
      undefined,
      sessionMeta({
        metadata: {
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
          },
        },
      }),
    )

    const fetched = mergeHydratedConversation(candidate, [hidden], [])
    const buffered = mergeHydratedConversation(
      candidate,
      [],
      [{ item: hidden, updated: false }],
    )

    for (const hydrated of [fetched, buffered]) {
      expect(hydrated).toMatchObject({
        started: true,
        hydrated: true,
        messages: [],
      })
      expect(hydrated.skills).toBeUndefined()
      expect(hydrated.systemPrompt?.addons).toEqual([
        { kind: 'skill', name: 'review', body: 'legacy body' },
      ])
      expect(preSendMetaUpdate(hydrated)).toBeNull()
    }
  })
})

describe('completeFailedHydration', () => {
  it('reaches a terminal state without replacing live or optimistic messages', () => {
    const messages: Conversation['messages'] = [
      {
        id: 'live-1',
        role: 'assistant',
        content: 'partial live response',
        createdAt: 2_100,
      },
      {
        id: 'optimistic-1',
        role: 'user',
        content: 'pending prompt',
        createdAt: 2_200,
      },
    ]
    const current = conversation({ messages, hydrated: false })

    const next = completeFailedHydration(current)

    expect(next.hydrated).toBe(true)
    expect(next.messages).toBe(messages)
    expect(next.status).toBe(current.status)
    expect(next.updatedAt).toBe(current.updatedAt)
  })

  it('is idempotent after hydration has already completed', () => {
    const current = conversation({ hydrated: true })

    expect(completeFailedHydration(current)).toBe(current)
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
            provisional: true,
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
    expect(next.messages[0]).not.toHaveProperty('provisional')
  })

  it('does not overwrite a durable lifecycle notice with a late live fallback', () => {
    const next = appendMessageToConversation(
      conversation({
        messages: [
          {
            id: 'e_t-1_error',
            role: 'system',
            kind: 'notice',
            tone: 'error',
            content: 'turn failed [llm.permanent] — exact reason',
            createdAt: 3_000,
          },
        ],
      }),
      {
        id: 'e_t-1_error',
        role: 'system',
        kind: 'notice',
        tone: 'error',
        content: 'response failed: fallback reason',
        provisional: true,
        createdAt: 3_100,
      },
    )

    expect(next.messages).toHaveLength(1)
    expect(next.messages[0]).toMatchObject({
      id: 'e_t-1_error',
      content: 'turn failed [llm.permanent] — exact reason',
    })
    expect(next.messages[0]).not.toHaveProperty('provisional')
  })
})

describe('mergeConversationMeta / system_prompt', () => {
  it('defaults when the key is absent', () => {
    const next = mergeConversationMeta(undefined, sessionMeta({}))
    expect(next.systemPrompt).toEqual(DEFAULT_SYSTEM_PROMPT_STATE)
  })

  it('restores a named choice with its strategy and resolved body', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({
        metadata: {
          system_prompt: {
            choice: { named: 'pirate' },
            strategy: 'override',
            named_body: 'Arr.',
          },
        },
      }),
    )
    expect(next.systemPrompt).toEqual({
      choice: { named: 'pirate' },
      strategy: 'override',
      namedBody: 'Arr.',
      customText: '',
      addons: [],
    })
  })

  it('degrades malformed persisted values to the default without throwing', () => {
    // Untrusted wire JSON: a string, a bare `custom`, and a missing name all
    // have to fall back rather than produce a half-built choice.
    for (const system_prompt of [
      'pirate',
      42,
      null,
      { strategy: 'override' },
      { choice: 'custom' },
      { choice: { named: 7 } },
    ]) {
      const next = mergeConversationMeta(
        undefined,
        sessionMeta({ metadata: { system_prompt } }),
      )
      expect(next.systemPrompt).toEqual(DEFAULT_SYSTEM_PROMPT_STATE)
    }
  })

  it('unknown strategies fall back to enrich, never to an invalid value', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({
        metadata: {
          system_prompt: { choice: { named: 'p' }, strategy: 'sideways' },
        },
      }),
    )
    expect(next.systemPrompt?.strategy).toBe('enrich')
  })

  it('restores an ID-only skill subset from session metadata', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({ metadata: { skills: ['review', 'release'] } }),
    )

    expect(next.skills).toEqual(['review', 'release'])
    expect(next.systemPrompt).toEqual(DEFAULT_SYSTEM_PROMPT_STATE)
  })

  it('keeps a zero-count legacy selection intact until hydration proves it empty', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({
        message_count: 0,
        metadata: {
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [
              { kind: 'skill', name: 'review', body: 'legacy body' },
              { kind: 'prompt', name: 'tone', body: 'Be concise.' },
            ],
          },
        },
      }),
    )

    expect(next.skills).toBeUndefined()
    expect(next.started).toBe(false)
    expect(next.systemPrompt?.addons).toEqual([
      { kind: 'skill', name: 'review', body: 'legacy body' },
      { kind: 'prompt', name: 'tone', body: 'Be concise.' },
    ])
    expect(preSendMetaUpdate(next)).toBeNull()
  })

  it('does not write metadata for an ordinary materialized zero-message session', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({ metadata: { model: 'provider::model', mode: 'agent' } }),
    )

    expect(preSendMetaUpdate(next)).toBeNull()
  })

  it('leaves established legacy skill bodies alone', () => {
    const next = mergeConversationMeta(
      undefined,
      sessionMeta({
        message_count: 2,
        metadata: {
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
          },
        },
      }),
    )

    expect(next.skills).toBeUndefined()
    expect(next.started).toBe(true)
    expect(next.systemPrompt?.addons).toEqual([
      { kind: 'skill', name: 'review', body: 'legacy body' },
    ])
  })

  it('persists only selected IDs for new sessions', () => {
    const metadata = metadataFor(
      conversation({
        model: 'provider::model',
        mode: 'agent',
        skills: ['review', 'release'],
        systemPrompt: DEFAULT_SYSTEM_PROMPT_STATE,
      }),
    )

    expect(metadata).toEqual({
      surface: 'console',
      model: 'provider::model',
      mode: 'agent',
      skills: ['review', 'release'],
    })
    expect(JSON.stringify(metadata)).not.toContain('body')
  })

  it('migrates only after empty hydration and preserves the complete metadata object', () => {
    const candidate = mergeConversationMeta(
      undefined,
      sessionMeta({
        metadata: {
          surface: 'console',
          model: 'provider::model',
          mode: 'agent',
          parent_session_id: 'console-parent',
          function_call_id: 'call-1',
          depth: 2,
          spawned_by: 'agent',
          foreign: { keep: true },
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
          },
        },
      }),
    )
    expect(preSendMetaUpdate(candidate)).toBeNull()

    const migrated = mergeHydratedConversation(candidate, [], [])

    const update = preSendMetaUpdate(migrated)
    expect(update).toEqual({
      session_id: 'console-1',
      metadata: {
        surface: 'console',
        model: 'provider::model',
        mode: 'agent',
        parent_session_id: 'console-parent',
        function_call_id: 'call-1',
        depth: 2,
        spawned_by: 'agent',
        foreign: { keep: true },
        skills: ['review'],
      },
    })
    expect(JSON.stringify(update)).not.toContain('legacy body')
    expect(JSON.stringify(migrated)).not.toContain('legacy body')
    expect(preSendMetaUpdate({ ...migrated, draft: true })).toBeNull()
    expect(preSendMetaUpdate({ ...migrated, started: true })).toBeNull()
  })

  it('finalizes delayed legacy metadata after empty hydration already completed', () => {
    const hydratedEmpty = mergeHydratedConversation(
      conversation({ started: false }),
      [],
      [],
    )
    const migrated = mergeConversationMeta(
      hydratedEmpty,
      sessionMeta({
        metadata: {
          foreign: 'preserved',
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
          },
        },
      }),
    )

    expect(migrated.skills).toEqual(['review'])
    expect(migrated.systemPrompt?.addons).toEqual([])
    expect(preSendMetaUpdate(migrated)).toEqual({
      session_id: 'console-1',
      metadata: { foreign: 'preserved', skills: ['review'] },
    })
  })

  it('retains confirmed-empty state when the bodyless set-meta event arrives before write cleanup', () => {
    const legacyMetadata = {
      foreign: 'preserved',
      system_prompt: {
        choice: 'default',
        strategy: 'enrich',
        addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
      },
    }
    const ready = mergeHydratedConversation(
      mergeConversationMeta(
        undefined,
        sessionMeta({ metadata: legacyMetadata }),
      ),
      [],
      [],
    )

    const bodylessEvent = mergeConversationMeta(
      ready,
      sessionMeta({
        metadata: { foreign: 'preserved', skills: ['review'] },
      }),
    )
    expect(preSendMetaUpdate(bodylessEvent)).toBeNull()

    const staleSnapshot = mergeConversationMeta(
      bodylessEvent,
      sessionMeta({ metadata: legacyMetadata }),
    )
    expect(staleSnapshot.systemPrompt?.addons).toEqual([])
    expect(preSendMetaUpdate(staleSnapshot)).toEqual({
      session_id: 'console-1',
      metadata: { foreign: 'preserved', skills: ['review'] },
    })
  })
})

describe('resolveActiveConversationId', () => {
  it('keeps a pending select until that session appears in the list', () => {
    const waiting = resolveActiveConversationId({
      conversationIds: ['draft'],
      activeId: 'draft',
      pendingSelectId: 'worker-session',
    })
    expect(waiting).toEqual({
      activeId: 'worker-session',
      pendingSelectId: 'worker-session',
    })

    const arrived = resolveActiveConversationId({
      conversationIds: ['worker-session', 'draft'],
      activeId: 'draft',
      pendingSelectId: 'worker-session',
    })
    expect(arrived).toEqual({
      activeId: 'worker-session',
      pendingSelectId: null,
    })
  })

  it('falls back to the first conversation when nothing is pending or active', () => {
    expect(
      resolveActiveConversationId({
        conversationIds: ['a', 'b'],
        activeId: 'gone',
        pendingSelectId: null,
      }),
    ).toEqual({ activeId: 'a', pendingSelectId: null })
  })
})

describe('isUntouchedDraft', () => {
  it('recognises the chat nobody has written in yet', () => {
    expect(isUntouchedDraft(conversation({ draft: true, messages: [] }))).toBe(
      true,
    )
  })

  it('refuses a draft that already carries work', () => {
    expect(
      isUntouchedDraft(
        conversation({
          draft: true,
          messages: [],
          draftText: 'half a thought',
        }),
      ),
    ).toBe(false)
    expect(
      isUntouchedDraft(
        conversation({
          draft: true,
          messages: [
            { id: 'm1', role: 'user', content: 'sent', createdAt: 1 },
          ] as Conversation['messages'],
        }),
      ),
    ).toBe(false)
  })

  it('refuses a real session, which is never interchangeable', () => {
    expect(isUntouchedDraft(conversation({ draft: false, messages: [] }))).toBe(
      false,
    )
  })
})
