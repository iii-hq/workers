import { describe, expect, it } from 'vitest'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  skillSelectionForSend,
} from '@/components/chat/system-prompt-selection'
import { transcriptToMessages } from '@/lib/sessions/entry-mapper'
import type {
  MetaUpdatedEvent,
  SessionMeta,
  StatusChangedEvent,
  TranscriptItem,
} from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import {
  appendMessageToConversation,
  applyCatalogModelFallback,
  applyConversationMetadataEvent,
  applyConversationMetadataPatch,
  applyConversationStatusEvent,
  bumpSessionWatchEpoch,
  cancelHydrationRunsForSessions,
  completeFailedHydration,
  completePreSendMetaUpdate,
  type HydrationRun,
  type HydrationUpsert,
  isUntouchedDraft,
  markBackgroundedStale,
  markDurableStarted,
  markUnwatchedStale,
  mergeConversationMeta,
  mergeHydratedConversation,
  mergeHydratedTranscript,
  mergeSessionListSnapshot,
  metadataFor,
  metadataForWrite,
  missingGenerationForDirectoryRefresh,
  preSendMetaUpdate,
  resolveActiveConversationId,
  shouldAcceptReconnectDirectoryRow,
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

  it('does not regress a newer directory event with an older metadata read', () => {
    const existing = conversation({
      status: 'done',
      statusReason: 'stopped',
      serverMetaUpdatedAt: 4_000,
      serverMetadataUpdatedAt: 4_000,
      serverStatusUpdatedAt: 4_000,
      updatedAt: 4_000,
    })

    const next = mergeConversationMeta(
      existing,
      sessionMeta({
        status: 'working',
        status_reason: 'waiting for model',
        updated_at: 3_000,
      }),
    )

    expect(next.status).toBe('done')
    expect(next.statusReason).toBe('stopped')
    expect(next.title).toBe(existing.title)
  })

  it('merges a full snapshot independently across metadata and status clocks', () => {
    const existing = conversation({
      title: 'Old name',
      status: 'done',
      statusReason: 'stopped',
      serverMetaUpdatedAt: 200,
      serverMetadataUpdatedAt: 100,
      serverStatusUpdatedAt: 200,
      updatedAt: 200,
    })

    const next = mergeConversationMeta(
      existing,
      sessionMeta({
        title: 'Frontend',
        status: 'working',
        updated_at: 150,
        metadata: {
          subagent_display: {
            name: 'Frontend',
            icon: 'code',
            color: 'blue',
          },
        },
      }),
    )

    expect(next.title).toBe('Frontend')
    expect(next.subagentAppearance?.name).toBe('Frontend')
    expect(next.serverMetadataUpdatedAt).toBe(150)
    expect(next.status).toBe('done')
    expect(next.statusReason).toBe('stopped')
    expect(next.serverStatusUpdatedAt).toBe(200)
    expect(next.updatedAt).toBe(200)
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

  it('retains raw harness metadata for subsequent whole-object writes', () => {
    const metadata = {
      surface: 'harness',
      parent_session_id: 'console-parent',
      function_call_id: 'call-spawn-1',
      spawned_by: 'agent',
      depth: 2,
      subagent_display: {
        name: '  Frontend  ',
        icon: 'code',
        color: 'purple',
      },
      harness_private: { attempt: 3 },
    }

    const next = mergeConversationMeta(undefined, sessionMeta({ metadata }))

    expect(next.sessionMetadata).toBe(metadata)
    expect(next.subagentAppearance).toEqual({
      name: 'Frontend',
      icon: 'code',
      color: 'purple',
    })
    expect(next.parentId).toBe('console-parent')
    expect(next.parentFunctionCallId).toBe('call-spawn-1')
  })
})

describe('unordered directory events', () => {
  const metadataEvent = (
    timestamp: number,
    name: string,
  ): MetaUpdatedEvent => ({
    session_id: 'console-1',
    title: name,
    description: '',
    metadata: {
      parent_session_id: 'root',
      subagent_display: { name, icon: 'code', color: 'blue' },
    },
    timestamp,
  })
  const statusEvent = (
    timestamp: number,
    status: StatusChangedEvent['status'],
  ): StatusChangedEvent => ({
    session_id: 'console-1',
    status,
    previous_status: status === 'done' ? 'working' : 'idle',
    ...(status === 'done' ? { status_reason: 'stopped' } : {}),
    timestamp,
  })

  it('does not turn a terminal child active when an older status arrives last', () => {
    const done = applyConversationStatusEvent(
      conversation({ status: 'working', serverStatusUpdatedAt: 2_000 }),
      statusEvent(4_000, 'done'),
    )
    const stale = applyConversationStatusEvent(
      done,
      statusEvent(3_000, 'working'),
    )

    expect(stale).toBe(done)
    expect(stale.status).toBe('done')
    expect(stale.statusReason).toBe('stopped')
  })

  it('does not roll back a newer sub-agent name, icon, or color', () => {
    const newest = applyConversationMetadataEvent(
      conversation({ serverMetadataUpdatedAt: 2_000 }),
      metadataEvent(4_000, 'Frontend'),
    )
    const stale = applyConversationMetadataEvent(
      newest,
      metadataEvent(3_000, 'Explorer'),
    )

    expect(stale).toBe(newest)
    expect(stale.subagentAppearance).toEqual({
      name: 'Frontend',
      icon: 'code',
      color: 'blue',
    })
  })

  it('orders status and metadata independently because their payloads are partial', () => {
    const renamed = applyConversationMetadataEvent(
      conversation({
        status: 'working',
        serverMetadataUpdatedAt: 2_000,
        serverStatusUpdatedAt: 2_000,
      }),
      metadataEvent(5_000, 'Reviewer'),
    )
    const completed = applyConversationStatusEvent(
      renamed,
      statusEvent(4_000, 'done'),
    )

    expect(completed.status).toBe('done')
    expect(completed.subagentAppearance?.name).toBe('Reviewer')
    expect(completed.updatedAt).toBe(5_000)
  })

  it('keeps pending skill edits while applying newer sub-agent metadata', () => {
    const candidate = applyConversationMetadataPatch(
      mergeConversationMeta(
        undefined,
        sessionMeta({
          metadata: {
            parent_session_id: 'root',
            system_prompt: {
              choice: 'default',
              strategy: 'enrich',
              addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
            },
          },
        }),
      ),
      { skills: ['release'] },
      2_500,
    )
    const metadata = {
      parent_session_id: 'root',
      skills: ['stale'],
      subagent_display: { name: 'Reviewer', icon: 'review', color: 'purple' },
    }

    const next = applyConversationMetadataEvent(candidate, {
      session_id: 'console-1',
      title: 'Reviewer',
      description: '',
      metadata,
      timestamp: 3_000,
    })

    expect(next.skills).toEqual(['release'])
    expect(next.legacySkillMigration?.state).toBe('candidate')
    expect(next.sessionMetadata).toBe(metadata)
    expect(next.subagentAppearance).toEqual({
      name: 'Reviewer',
      icon: 'review',
      color: 'purple',
    })
    expect(next.serverMetadataUpdatedAt).toBe(3_000)
  })
})

describe('reconnect hydration', () => {
  it('cancels only pre-reconnect reads for sessions being refreshed', () => {
    const first = {
      cancelled: false,
      connectionEpoch: 0,
      watchEpoch: 0,
      upserts: [],
    }
    const second = {
      cancelled: false,
      connectionEpoch: 0,
      watchEpoch: 0,
      upserts: [],
    }
    const runs = new Map<string, HydrationRun>([
      ['first', first],
      ['second', second],
    ])
    const firstBuffer: HydrationUpsert[] = []
    const secondBuffer: HydrationUpsert[] = []
    const buffers = new Map<string, HydrationUpsert[]>([
      ['first', firstBuffer],
      ['second', secondBuffer],
    ])

    cancelHydrationRunsForSessions(['first'], runs, buffers)

    expect(first.cancelled).toBe(true)
    expect(runs.has('first')).toBe(false)
    expect(buffers.has('first')).toBe(false)
    expect(second.cancelled).toBe(false)
    expect(runs.get('second')).toBe(second)
    expect(buffers.get('second')).toBe(secondBuffer)
  })

  it('keeps lifecycle epochs isolated between concurrently mounted panels', () => {
    const epochs = new Map([
      ['panel-a', 1],
      ['panel-b', 1],
    ])

    bumpSessionWatchEpoch(epochs, 'panel-b')

    expect(epochs.get('panel-a')).toBe(1)
    expect(epochs.get('panel-b')).toBe(2)
  })

  it('accepts a fresh reconnect row across unrelated lookup generations', () => {
    expect(
      shouldAcceptReconnectDirectoryRow({
        currentGeneration: 2,
      }),
    ).toBe(true)
  })

  it('never lets a reconnect list override a definitive missing tombstone', () => {
    expect(
      shouldAcceptReconnectDirectoryRow({
        currentGeneration: 2,
        missingGeneration: 2,
      }),
    ).toBe(false)
  })

  it('expires a negative tombstone before a later reconnect snapshot', () => {
    const tombstone = {
      lookupGeneration: 2,
      directoryRefreshGeneration: 4,
    }

    expect(missingGenerationForDirectoryRefresh(tombstone, 4)).toBe(2)
    expect(missingGenerationForDirectoryRefresh(tombstone, 5)).toBeUndefined()
  })
})

describe('metadataFor', () => {
  it('preserves harness linkage and appearance across whole-object writes', () => {
    const next = metadataFor(
      conversation({
        model: 'provider::current-model',
        workingDir: null,
        sessionMetadata: {
          surface: 'harness',
          parent_session_id: 'console-parent',
          parent_turn_id: 'turn-parent',
          function_call_id: 'call-spawn-1',
          depth: 1,
          spawned_by: 'agent',
          skills: ['stale'],
          fs_scope: { root: '/stale' },
          subagent_display: {
            name: 'Frontend',
            icon: 'code',
            color: 'blue',
          },
        },
      }),
    )

    expect(next).toEqual({
      surface: 'console',
      model: 'provider::current-model',
      mode: 'agent',
      parent_session_id: 'console-parent',
      parent_turn_id: 'turn-parent',
      function_call_id: 'call-spawn-1',
      depth: 1,
      spawned_by: 'agent',
      subagent_display: {
        name: 'Frontend',
        icon: 'code',
        color: 'blue',
      },
    })
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

describe('markUnwatchedStale', () => {
  it('keeps every watched panel live while staling only hidden sessions', () => {
    const sessions = [
      conversation({ id: 'left-panel', hydrated: true }),
      conversation({ id: 'right-panel', hydrated: true }),
      conversation({ id: 'hidden', hydrated: true }),
      conversation({ id: 'draft', draft: true, hydrated: true }),
    ]

    const next = markUnwatchedStale(
      sessions,
      new Set(['left-panel', 'right-panel']),
    )

    expect(next.find((c) => c.id === 'left-panel')?.hydrated).toBe(true)
    expect(next.find((c) => c.id === 'right-panel')?.hydrated).toBe(true)
    expect(next.find((c) => c.id === 'hidden')?.hydrated).toBe(false)
    expect(next.find((c) => c.id === 'draft')?.hydrated).toBe(true)
  })

  it('preserves identity when all hydrated server sessions are watched', () => {
    const sessions = [
      conversation({ id: 'left-panel', hydrated: true }),
      conversation({ id: 'right-panel', hydrated: true }),
    ]

    expect(
      markUnwatchedStale(sessions, new Set(['left-panel', 'right-panel'])),
    ).toBe(sessions)
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
      metadata: {
        foreign: 'preserved',
        surface: 'console',
        mode: 'agent',
        skills: ['review'],
      },
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
      metadata: {
        foreign: 'preserved',
        surface: 'console',
        mode: 'agent',
        skills: ['review'],
      },
    })
  })

  it('rebuilds every Console-owned field from the current ready conversation', () => {
    const ready = mergeHydratedConversation(
      mergeConversationMeta(
        undefined,
        sessionMeta({
          metadata: {
            surface: 'legacy-surface',
            model: 'provider::old',
            mode: 'agent',
            title_manual: true,
            fs_scope: { root: '/old' },
            memory_bank: 'old-bank',
            parent_session_id: 'console-parent',
            foreign: { keep: true },
            system_prompt: {
              choice: 'default',
              strategy: 'enrich',
              addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
            },
          },
        }),
      ),
      [],
      [],
    )
    const current = applyConversationMetadataPatch(
      ready,
      {
        title: 'renamed',
        titleManual: false,
        model: 'provider::new',
        mode: 'ask',
        workingDir: null,
        memoryBank: null,
        systemPrompt: {
          choice: { named: 'pirate' },
          strategy: 'override',
          namedBody: 'Arr.',
          customText: '',
          addons: [{ kind: 'prompt', name: 'tone', body: 'Be brief.' }],
        },
        skills: ['release'],
      },
      3_000,
    )
    const afterEvent = mergeConversationMeta(
      current,
      sessionMeta({
        title: 'stale title',
        metadata: {
          surface: 'console',
          model: 'provider::old',
          mode: 'agent',
          title_manual: true,
          fs_scope: { root: '/old' },
          memory_bank: 'old-bank',
          parent_session_id: 'console-parent',
          foreign: { keep: true },
          skills: ['review'],
        },
      }),
    )

    expect(afterEvent).toMatchObject({
      title: 'renamed',
      titleManual: false,
      model: 'provider::new',
      mode: 'ask',
      workingDir: null,
      memoryBank: null,
      skills: ['release'],
    })
    expect(preSendMetaUpdate(afterEvent)).toEqual({
      session_id: 'console-1',
      metadata: {
        surface: 'console',
        model: 'provider::new',
        mode: 'ask',
        parent_session_id: 'console-parent',
        foreign: { keep: true },
        system_prompt: {
          choice: { named: 'pirate' },
          strategy: 'override',
          named_body: 'Arr.',
          addons: [{ kind: 'prompt', name: 'tone', body: 'Be brief.' }],
        },
        skills: ['release'],
      },
    })
  })

  it.each([
    { label: 'latest subset', selection: ['release'], skills: ['release'] },
    { label: 'explicit All', selection: undefined, skills: undefined },
  ])(
    'keeps $label through candidate picker, meta event, and empty hydration',
    ({ selection, skills }) => {
      const legacyMetadata = {
        surface: 'console',
        mode: 'agent',
        parent_session_id: 'console-parent',
        foreign: { keep: true },
        system_prompt: {
          choice: 'default',
          strategy: 'enrich',
          addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
        },
      }
      const candidate = mergeConversationMeta(
        undefined,
        sessionMeta({ metadata: legacyMetadata }),
      )
      const picked = applyConversationMetadataPatch(
        candidate,
        { skills: selection },
        3_000,
      )
      const afterEvent = mergeConversationMeta(
        picked,
        sessionMeta({
          metadata: {
            surface: 'console',
            mode: 'agent',
            skills: ['stale'],
          },
        }),
      )
      expect(afterEvent.systemPrompt?.addons).toEqual([
        { kind: 'skill', name: 'review', body: 'legacy body' },
      ])
      const migrated = mergeHydratedConversation(afterEvent, [], [])

      expect(preSendMetaUpdate(migrated)).toEqual({
        session_id: 'console-1',
        metadata: {
          surface: 'console',
          mode: 'agent',
          parent_session_id: 'console-parent',
          foreign: { keep: true },
          ...(skills ? { skills } : {}),
        },
      })
    },
  )

  it.each([
    { label: 'latest subset', selection: ['release'], skills: ['release'] },
    { label: 'explicit All', selection: undefined, skills: undefined },
  ])(
    'keeps $label through ready picker, meta event, and immediate pre-send',
    ({ selection, skills }) => {
      const legacyMetadata = {
        surface: 'console',
        mode: 'agent',
        parent_session_id: 'console-parent',
        foreign: { keep: true },
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
      const picked = applyConversationMetadataPatch(
        ready,
        { skills: selection },
        3_000,
      )
      expect(preSendMetaUpdate(picked)?.metadata.skills).toEqual(skills)

      const afterEvent = mergeConversationMeta(
        picked,
        sessionMeta({
          metadata: {
            surface: 'console',
            mode: 'agent',
            skills: ['stale'],
          },
        }),
      )

      expect(preSendMetaUpdate(afterEvent)).toEqual({
        session_id: 'console-1',
        metadata: {
          surface: 'console',
          mode: 'agent',
          parent_session_id: 'console-parent',
          foreign: { keep: true },
          ...(skills ? { skills } : {}),
        },
      })
    },
  )

  it.each([
    { label: 'explicit All', selection: undefined, selected: undefined },
    {
      label: 'latest subset',
      selection: ['release'],
      selected: ['release'],
    },
  ])(
    'keeps $label and current metadata through ack, user-only start, and a delayed candidate event',
    ({ selection, selected }) => {
      const legacyMetadata = {
        surface: 'console',
        model: 'provider::old',
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
      }
      const ready = mergeHydratedConversation(
        applyConversationMetadataPatch(
          mergeConversationMeta(
            undefined,
            sessionMeta({ metadata: legacyMetadata }),
          ),
          { model: 'provider::new', mode: 'ask', skills: selection },
          3_000,
        ),
        [],
        [],
      )
      const pendingEdits = ready.legacySkillMigration?.edits
      const acknowledged = completePreSendMetaUpdate(ready, pendingEdits)

      expect(acknowledged.legacySkillMigration).toMatchObject({
        state: 'empty',
        edits: { model: 'provider::new', mode: 'ask' },
      })
      expect(
        Object.hasOwn(acknowledged.legacySkillMigration?.edits ?? {}, 'skills'),
      ).toBe(true)
      expect(preSendMetaUpdate(acknowledged)).toBeNull()

      const userOnly = markDurableStarted(acknowledged, false)
      expect(userOnly.started).toBe(true)
      expect(userOnly.legacySkillMigration).toMatchObject({ state: 'empty' })

      const delayed = mergeConversationMeta(
        userOnly,
        sessionMeta({
          metadata: {
            surface: 'console',
            model: 'provider::old',
            mode: 'agent',
            system_prompt: legacyMetadata.system_prompt,
          },
        }),
      )

      expect(delayed).toMatchObject({
        started: true,
        model: 'provider::new',
        mode: 'ask',
        skills: selected,
      })
      expect(delayed.systemPrompt?.addons).toEqual([])
      expect(
        skillSelectionForSend(delayed.skills, {
          turnEstablished: false,
          willQueue: false,
        }),
      ).toEqual(selected)
      expect(metadataForWrite(delayed)).toEqual({
        surface: 'console',
        model: 'provider::new',
        mode: 'ask',
        parent_session_id: 'console-parent',
        function_call_id: 'call-1',
        depth: 2,
        spawned_by: 'agent',
        foreign: { keep: true },
        ...(selected ? { skills: selected } : {}),
      })

      const assistantStarted = markDurableStarted(delayed, true)
      expect(assistantStarted.started).toBe(true)
      expect(assistantStarted.legacySkillMigration).toMatchObject({
        state: 'ready',
      })
    },
  )

  it.each([
    { label: 'explicit All', selection: undefined, selected: undefined },
    {
      label: 'latest subset',
      selection: ['release'],
      selected: ['release'],
    },
  ])(
    'keeps acknowledged $label and metadata through assistant evidence and a delayed candidate event',
    ({ selection, selected }) => {
      const legacyMetadata = {
        surface: 'console',
        model: 'provider::old',
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
      }
      const ready = mergeHydratedConversation(
        applyConversationMetadataPatch(
          mergeConversationMeta(
            undefined,
            sessionMeta({ metadata: legacyMetadata }),
          ),
          { model: 'provider::new', mode: 'ask', skills: selection },
          3_000,
        ),
        [],
        [],
      )
      const acknowledged = completePreSendMetaUpdate(
        ready,
        ready.legacySkillMigration?.edits,
      )

      const assistantStarted = markDurableStarted(acknowledged, true)
      expect(assistantStarted.legacySkillMigration).toMatchObject({
        state: 'empty',
      })

      const delayed = mergeConversationMeta(
        assistantStarted,
        sessionMeta({
          message_count: 1,
          metadata: {
            surface: 'console',
            model: 'provider::old',
            mode: 'agent',
            system_prompt: legacyMetadata.system_prompt,
          },
        }),
      )
      const edited = applyConversationMetadataPatch(
        delayed,
        { memoryBank: 'current-bank' },
        4_000,
      )

      expect(edited).toMatchObject({
        started: true,
        model: 'provider::new',
        mode: 'ask',
        memoryBank: 'current-bank',
        skills: selected,
      })
      expect(edited.systemPrompt?.addons).toEqual([])
      expect(metadataForWrite(edited)).toEqual({
        surface: 'console',
        model: 'provider::new',
        mode: 'ask',
        memory_bank: 'current-bank',
        parent_session_id: 'console-parent',
        function_call_id: 'call-1',
        depth: 2,
        spawned_by: 'agent',
        foreign: { keep: true },
        ...(selected ? { skills: selected } : {}),
      })
    },
  )

  it('clears an unconfirmed legacy candidate when assistant evidence arrives', () => {
    const candidate = mergeConversationMeta(
      undefined,
      sessionMeta({
        metadata: {
          surface: 'console',
          mode: 'agent',
          system_prompt: {
            choice: 'default',
            strategy: 'enrich',
            addons: [{ kind: 'skill', name: 'review', body: 'legacy body' }],
          },
        },
      }),
    )

    const established = markDurableStarted(candidate, true)

    expect(established.started).toBe(true)
    expect(established.legacySkillMigration).toBeUndefined()
    expect(established.skills).toBeUndefined()
    expect(established.systemPrompt?.addons).toEqual([
      { kind: 'skill', name: 'review', body: 'legacy body' },
    ])
  })

  it('uses the preserved metadata base for ordinary writes after migration acknowledgment', () => {
    const ready = mergeHydratedConversation(
      mergeConversationMeta(
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
      ),
      [],
      [],
    )
    const acknowledged = completePreSendMetaUpdate(
      ready,
      ready.legacySkillMigration?.edits,
    )
    const edited = applyConversationMetadataPatch(
      acknowledged,
      { memoryBank: 'current-bank' },
      3_000,
    )

    expect(metadataForWrite(edited)).toEqual({
      surface: 'console',
      model: 'provider::model',
      mode: 'agent',
      memory_bank: 'current-bank',
      parent_session_id: 'console-parent',
      function_call_id: 'call-1',
      depth: 2,
      spawned_by: 'agent',
      foreign: { keep: true },
      skills: ['review'],
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
