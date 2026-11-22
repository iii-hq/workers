import { describe, expect, it, vi } from 'vitest'
import {
  SIDE_CHAT_KEY,
  SIDE_CHAT_QUESTION_KEY,
  SIDE_CHAT_QUOTES_KEY,
} from '@/lib/chat-aside'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import type { DecisionChatRequest } from '@/types/injectable-ui'
import {
  createDecisionSideChat,
  decisionForkPoint,
  decisionSeed,
  findDecisionSideChat,
  SIDE_CHAT_FUNCTIONS,
} from './side-chat'

const request: DecisionChatRequest = {
  key: 'source/question',
  sessionId: 'source',
  kind: 'question',
  title: 'Side chat · Original run',
  sourceTitle: 'Original run',
  agent: 'planner',
  card: { key: 'KAN-1', title: 'Isolate builds' },
  board: { ticket: 'KAN-1', createdAt: '2026-11-21T12:30:00.000Z' },
  createdAt: '2026-11-21T12:00:00.000Z',
  why: 'The builds share a target directory.',
  questionId: 'question',
  questions: [
    {
      header: 'Builds',
      question: 'How should builds be isolated?',
      options: [
        {
          label: 'Separate target directories',
          description: 'Prevents lock contention.',
          recommended: true,
        },
      ],
    },
  ],
  onUseAnswer: vi.fn(),
}

function assistant(
  entryId: string,
  callId: string,
  text = 'I need your decision.',
  functionId = 'harness::ask',
): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'assistant',
      content: [
        ...(text ? [{ type: 'text' as const, text }] : []),
        {
          type: 'function_call',
          id: callId,
          function_id: functionId,
          arguments: {},
        },
      ],
      stop_reason: 'function_call',
      model: 'test',
      provider: 'test',
      timestamp: 2,
    },
  }
}

function user(entryId: string): TranscriptItem {
  return {
    entry_id: entryId,
    message: { role: 'user', content: [], timestamp: 1 },
  }
}

function meta(id: string, metadata: Record<string, unknown> = {}): SessionMeta {
  return {
    session_id: id,
    title: 'Original run',
    description: '',
    status: 'idle',
    metadata,
    created_at: 1,
    updated_at: 1,
    message_count: 2,
  }
}

describe('decision side chat host helpers', () => {
  it('selects the entry before direct and wrapped pending calls', () => {
    const wrapped = assistant('ask', 'outer')
    const call = wrapped.message?.content[1]
    if (call?.type === 'function_call') {
      call.function_id = 'agent_call'
      call.arguments = {
        function: 'harness::ask',
        payload: { function_call_id: 'question' },
      }
    }
    expect(
      decisionForkPoint(
        [user('before'), assistant('ask', 'question')],
        'question',
      ),
    ).toEqual({ entryId: 'before', assistantText: 'I need your decision.' })
    expect(decisionForkPoint([user('before'), wrapped], 'question')).toEqual({
      entryId: 'before',
      assistantText: 'I need your decision.',
    })
    expect(
      decisionForkPoint([assistant('ask', 'question')], 'question'),
    ).toBeNull()
    expect(decisionForkPoint([user('before')], 'missing')).toBeNull()
  })

  it('quotes the request and uses the source title when assistant text is absent', () => {
    const seed = decisionSeed(request)
    expect(seed).toMatchObject({
      messageId: `decision:${request.key}`,
      agent: request.agent,
      full: true,
    })
    expect(seed.text).toContain(request.sourceTitle)
    expect(seed.text).toContain(request.questions?.[0].options[0].description)
    expect(seed.text).toContain(request.why)
    expect(seed.text).toContain(request.board?.createdAt)
  })

  it('quotes a held function and its already-redacted arguments', () => {
    const held = decisionSeed(
      {
        ...request,
        key: 'source/call',
        kind: 'held',
        questionId: undefined,
        questions: undefined,
        held: {
          functionCallId: 'call',
          functionId: 'shell::exec',
          arguments: { command: '[REDACTED]' },
        },
      },
      'Assistant text belongs only to questions.',
    )
    expect(held.text).toContain('shell::exec')
    expect(held.text).toContain('[REDACTED]')
    expect(held.text).not.toContain('What the asker said')
    expect(held.text).not.toContain(request.sourceTitle)
    expect(held.text).not.toContain('Assistant text belongs only to questions.')
  })

  it('finds the exact copy across pages without forking', async () => {
    const exact = meta('copy', { [SIDE_CHAT_QUESTION_KEY]: request.key })
    const call = vi.fn(async (_id: string, input: Record<string, unknown>) =>
      input.cursor
        ? { sessions: [exact] }
        : { sessions: [], next_cursor: 'next' },
    )
    expect(
      await findDecisionSideChat(
        { call: call as IiiClient['call'] },
        request.key,
      ),
    ).toBe(exact)
    expect(call).toHaveBeenLastCalledWith(
      'session::list',
      expect.objectContaining({ cursor: 'next' }),
    )
  })
})

describe('createDecisionSideChat', () => {
  it('forks once at the pending call and installs metadata and read grants', async () => {
    const calls: Array<[string, Record<string, unknown>]> = []
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get')
        return { meta: meta(request.sessionId, { kanban_ticket: 'KAN-1' }) }
      if (id === 'session::messages')
        return {
          messages: [
            user('before'),
            assistant('ask', request.questionId ?? ''),
          ],
        }
      if (id === 'session::fork')
        return {
          session_id: 'copy',
          meta: meta('copy', {
            kanban_ticket: 'KAN-1',
            agent_profile: { id: 'deepwork' },
          }),
        }
      if (id === 'session::messages-tail')
        return { messages: [user('copied-before')] }
      if (id === 'session::set-meta')
        return { meta: meta('copy', input.metadata as Record<string, unknown>) }
      return {}
    })
    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      request,
    )

    expect(result.fresh).toBe(true)
    expect(calls).toContainEqual([
      'session::fork',
      {
        session_id: request.sessionId,
        entry_id: 'before',
        title: request.title,
      },
    ])
    const metadata = calls.find(([id]) => id === 'session::set-meta')?.[1]
      .metadata as Record<string, unknown>
    expect(metadata).toMatchObject({
      [SIDE_CHAT_KEY]: request.sessionId,
      [SIDE_CHAT_QUESTION_KEY]: request.key,
      side_chat_question_kind: 'question',
      side_chat_from: 'copied-before',
    })
    expect(metadata).not.toHaveProperty('kanban_ticket')
    expect(metadata).not.toHaveProperty('agent_profile')
    expect(metadata[SIDE_CHAT_QUOTES_KEY]).toEqual(
      expect.arrayContaining([expect.objectContaining({ full: true })]),
    )
    expect(
      calls
        .filter(([id]) => id === 'approval::add-always-allow')
        .map(([, input]) => input.function_id),
    ).toEqual(SIDE_CHAT_FUNCTIONS)
  })

  it('reuses one copy per key', async () => {
    const existing = meta('copy', {
      [SIDE_CHAT_QUESTION_KEY]: request.key,
    })
    const call = vi.fn(async (id: string) =>
      id === 'session::list' ? { sessions: [existing] } : {},
    )
    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      request,
    )
    expect(result).toMatchObject({ meta: existing, fresh: false })
    expect(call).not.toHaveBeenCalledWith('session::fork', expect.anything())
  })

  it('creates a titled seeded copy when the original session is gone', async () => {
    const calls: Array<[string, Record<string, unknown>]> = []
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return null
      if (id === 'session::create')
        return {
          session_id: 'fresh',
          meta: meta('fresh', input.metadata as Record<string, unknown>),
        }
      return {}
    })
    await createDecisionSideChat({ call: call as IiiClient['call'] }, request)
    expect(calls.find(([id]) => id === 'session::create')?.[1]).toMatchObject({
      title: request.title,
      kind: 'user',
      metadata: {
        [SIDE_CHAT_QUESTION_KEY]: request.key,
        side_chat_question_kind: 'question',
      },
    })
  })

  it('deletes a partial copy when approval setup fails', async () => {
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return null
      if (id === 'session::create')
        return {
          session_id: 'partial',
          meta: meta('partial', input.metadata as Record<string, unknown>),
        }
      if (id === 'approval::set-mode') throw new Error('gate unavailable')
      return {}
    })
    await expect(
      createDecisionSideChat({ call: call as IiiClient['call'] }, request),
    ).rejects.toThrow('gate unavailable')
    expect(call).toHaveBeenCalledWith('session::delete', {
      session_id: 'partial',
    })
  })

  it('forks a held call at the strictly previous transcript entry', async () => {
    const held: DecisionChatRequest = {
      ...request,
      key: 'source/held-call',
      kind: 'held',
      questionId: undefined,
      questions: undefined,
      held: {
        functionCallId: 'held-call',
        functionId: 'shell::exec',
        arguments: { command: '[REDACTED]' },
      },
    }
    const immediatelyBefore: TranscriptItem = {
      entry_id: 'result-before-hold',
      custom: { custom_type: 'wake', data: {} },
    }
    const calls: Array<[string, Record<string, unknown>]> = []
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return { meta: meta(held.sessionId) }
      if (id === 'session::messages')
        return {
          messages: [
            user('older'),
            immediatelyBefore,
            assistant('hold', 'held-call', '', 'shell::exec'),
          ],
        }
      if (id === 'session::fork')
        return { session_id: 'held-copy', meta: meta('held-copy') }
      if (id === 'session::messages-tail')
        return { messages: [immediatelyBefore] }
      if (id === 'session::set-meta')
        return {
          meta: meta('held-copy', input.metadata as Record<string, unknown>),
        }
      return {}
    })
    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      held,
    )
    expect(result.meta.metadata).toMatchObject({
      [SIDE_CHAT_QUESTION_KEY]: held.key,
      side_chat_question_kind: 'held',
    })
    expect(calls).toContainEqual([
      'session::fork',
      {
        session_id: held.sessionId,
        entry_id: 'result-before-hold',
        title: held.title,
      },
    ])
  })

  it('finds an old pending call across paginated session messages', async () => {
    const calls: Array<[string, Record<string, unknown>]> = []
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return { meta: meta(request.sessionId) }
      if (id === 'session::messages')
        return input.cursor
          ? { messages: [assistant('old-ask', 'question')] }
          : { messages: [user('old-parent')], next_cursor: 'older-page' }
      if (id === 'session::fork')
        return { session_id: 'copy', meta: meta('copy') }
      if (id === 'session::messages-tail')
        return { messages: [user('copy-parent')] }
      if (id === 'session::set-meta')
        return { meta: meta('copy', input.metadata as Record<string, unknown>) }
      return {}
    })
    await createDecisionSideChat({ call: call as IiiClient['call'] }, request)
    expect(calls).toContainEqual([
      'session::messages',
      expect.objectContaining({ cursor: 'older-page' }),
    ])
    expect(calls).toContainEqual([
      'session::fork',
      expect.objectContaining({ entry_id: 'old-parent' }),
    ])
  })

  it.each([
    ['target call is absent', [user('only-message')]],
    ['target call has no predecessor', [assistant('ask', 'question')]],
  ])('fails closed before creating when the %s', async (_, messages) => {
    const call = vi.fn(async (id: string) => {
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return { meta: meta(request.sessionId) }
      if (id === 'session::messages') return { messages }
      return {}
    })
    await expect(
      createDecisionSideChat({ call: call as IiiClient['call'] }, request),
    ).rejects.toThrow('entry before the pending decision is unavailable')
    expect(call).not.toHaveBeenCalledWith('session::fork', expect.anything())
  })

  it('coalesces concurrent opens of the same key into one fork', async () => {
    let releaseList = () => {}
    const listed = new Promise<{ sessions: SessionMeta[] }>((resolve) => {
      releaseList = () => resolve({ sessions: [] })
    })
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      if (id === 'session::list') return listed
      if (id === 'session::get') return { meta: meta(request.sessionId) }
      if (id === 'session::messages')
        return { messages: [user('before'), assistant('ask', 'question')] }
      if (id === 'session::fork')
        return { session_id: 'copy', meta: meta('copy') }
      if (id === 'session::messages-tail') return { messages: [user('before')] }
      if (id === 'session::set-meta')
        return { meta: meta('copy', input.metadata as Record<string, unknown>) }
      return {}
    })
    const iii = { call: call as IiiClient['call'] }
    const first = createDecisionSideChat(iii, request)
    const second = createDecisionSideChat(iii, request)
    expect(second).toBe(first)
    releaseList()
    await Promise.all([first, second])
    expect(
      call.mock.calls.filter(([id]) => id === 'session::fork'),
    ).toHaveLength(1)
  })

  it('serializes approval grants for an existing copy', async () => {
    let releaseFirstGrant = () => {}
    const firstGrant = new Promise<void>((resolve) => {
      releaseFirstGrant = resolve
    })
    const grants: string[] = []
    const existing = meta('copy', { [SIDE_CHAT_QUESTION_KEY]: request.key })
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      if (id === 'session::list') return { sessions: [existing] }
      if (id === 'approval::add-always-allow') {
        grants.push(String(input.function_id))
        if (grants.length === 1) await firstGrant
      }
      return {}
    })
    const opening = createDecisionSideChat(
      { call: call as IiiClient['call'] },
      request,
    )
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    expect(grants).toEqual([SIDE_CHAT_FUNCTIONS[0]])
    releaseFirstGrant()
    await opening
    expect(grants).toEqual(SIDE_CHAT_FUNCTIONS)
  })
})
