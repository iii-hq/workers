import { describe, expect, it, vi } from 'vitest'
import {
  SIDE_CHAT_KEY,
  SIDE_CHAT_QUESTION_KEY,
  SIDE_CHAT_QUOTES_KEY,
} from '@/lib/chat-aside'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import { towerFixture } from './fixtures'
import type { AskQuestion, HeldNeed, QuestionNeed } from './model'
import {
  createDecisionSideChat,
  decisionForkPoint,
  decisionKey,
  decisionSeed,
  SIDE_CHAT_FUNCTIONS,
  selectionFromSideChat,
} from './side-chat'

const question = towerFixture.items.find(
  (item): item is QuestionNeed => item.kind === 'question',
) as QuestionNeed
const held = towerFixture.items.find(
  (item): item is HeldNeed => item.kind === 'held',
) as HeldNeed

function assistant(
  entryId: string,
  callId: string,
  wrapped = false,
  functionId = 'harness::ask',
): TranscriptItem {
  return {
    entry_id: entryId,
    message: {
      role: 'assistant',
      content: [
        { type: 'text', text: 'I need your decision.' },
        {
          type: 'function_call',
          id: wrapped ? 'outer-call' : callId,
          function_id: wrapped ? 'agent_call' : functionId,
          arguments: wrapped
            ? {
                function: functionId,
                payload: { function_call_id: callId },
              }
            : {},
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

describe('decision side chat helpers', () => {
  it('selects the entry before direct and wrapped pending calls', () => {
    const before = user('before')
    expect(
      decisionForkPoint([before, assistant('ask', 'question')], 'question'),
    ).toEqual({ entryId: 'before', assistantText: 'I need your decision.' })
    expect(
      decisionForkPoint(
        [before, assistant('ask', 'question', true)],
        'question',
      ),
    ).toEqual({ entryId: 'before', assistantText: 'I need your decision.' })
    expect(
      decisionForkPoint([assistant('ask', 'question')], 'question'),
    ).toBeNull()
    expect(decisionForkPoint([before], 'missing')).toBeNull()
  })

  it('includes the whole pending question, options, why, card and trail', () => {
    const seed = decisionSeed(
      question,
      'The assistant text',
      towerFixture.questions.find(
        (row) => row.questionId === question.questionId,
      ),
    )
    expect(seed.full).toBe(true)
    expect(seed.text).toContain('conversation about a pending decision')
    expect(seed.text).toContain('The assistant text')
    expect(seed.text).toContain(question.entry.about.why)
    expect(seed.text).toContain(question.entry.questions[0].options[0].label)
    expect(seed.text).toContain(question.card?.key)
    expect(seed.text).toContain('Trail:')
    expect(seed.text).toContain(`Asked in ${question.sessionId}`)
  })

  it('maps an exact option label or arbitrary text into the decision draft', () => {
    const single = question.entry.questions[0]
    expect(
      selectionFromSideChat(single, `  ${single.options[1].label}  `),
    ).toEqual({ picked: [single.options[1].label], text: '' })
    expect(selectionFromSideChat(single, 'A different answer')).toEqual({
      picked: [],
      text: 'A different answer',
    })
    const multi: AskQuestion = { ...single, multi_select: true }
    expect(selectionFromSideChat(multi, multi.options[0].label)).toEqual({
      picked: [multi.options[0].label],
      text: '',
    })
  })
})

describe('createDecisionSideChat', () => {
  it('forks once at the selected point and installs isolated metadata and read grants', async () => {
    const calls: Array<[string, Record<string, unknown>]> = []
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get')
        return {
          meta: meta(question.sessionId, {
            kanban_ticket: 'KAN-1',
            spawned_by: 'agent',
          }),
        }
      if (id === 'session::messages')
        return {
          messages: [user('before'), assistant('ask', question.questionId)],
        }
      if (id === 'session::fork')
        return {
          session_id: 'copy',
          meta: meta('copy', {
            kanban_ticket: 'KAN-1',
            spawned_by: 'agent',
            external_source: 'import',
            read_only: true,
            agent_profile: { id: 'deepwork', name: 'deepwork' },
            system_prompt: { text: 'Kit instructions' },
            skills: ['deepwork'],
          }),
        }
      if (id === 'session::messages-tail')
        return { messages: [user('copied-before')] }
      if (id === 'session::set-meta')
        return { meta: meta('copy', input.metadata as Record<string, unknown>) }
      return { settings: {} }
    })
    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      question,
    )

    expect(result.fresh).toBe(true)
    expect(calls).toContainEqual([
      'session::fork',
      {
        session_id: question.sessionId,
        entry_id: 'before',
        title: 'Side chat · Original run',
      },
    ])
    const metadata = calls.find(([id]) => id === 'session::set-meta')?.[1]
      .metadata as Record<string, unknown>
    expect(metadata).toMatchObject({
      [SIDE_CHAT_KEY]: question.sessionId,
      [SIDE_CHAT_QUESTION_KEY]: decisionKey(question),
      side_chat_question_kind: 'question',
      side_chat_from: 'copied-before',
    })
    expect(metadata).not.toHaveProperty('kanban_ticket')
    expect(metadata).not.toHaveProperty('spawned_by')
    for (const key of [
      'external_source',
      'read_only',
      'agent_profile',
      'system_prompt',
      'skills',
    ])
      expect(metadata).not.toHaveProperty(key)
    expect(metadata[SIDE_CHAT_QUOTES_KEY]).toEqual(
      expect.arrayContaining([expect.objectContaining({ full: true })]),
    )
    expect(calls).toContainEqual([
      'session::append',
      {
        session_id: 'copy',
        message: expect.objectContaining({
          role: 'user',
          content: [
            {
              type: 'text',
              text: expect.stringContaining(
                question.entry.questions[0].options[0].label,
              ),
            },
          ],
        }),
      },
    ])
    expect(calls).toContainEqual([
      'approval::set-mode',
      { session_id: 'copy', mode: 'auto' },
    ])
    expect(
      calls
        .filter(([id]) => id === 'approval::add-always-allow')
        .map(([, input]) => input.function_id),
    ).toEqual(SIDE_CHAT_FUNCTIONS)
  })

  it('reuses only the copy tagged for the exact question', async () => {
    const ordinary = meta('ordinary', { [SIDE_CHAT_KEY]: question.sessionId })
    const exact = meta('exact', {
      [SIDE_CHAT_KEY]: question.sessionId,
      [SIDE_CHAT_QUESTION_KEY]: decisionKey(question),
    })
    const call = vi.fn(async (id: string) =>
      id === 'session::list'
        ? { sessions: [ordinary, exact] }
        : { settings: {} },
    )
    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      question,
    )
    expect(result).toMatchObject({ meta: exact, fresh: false })
    expect(call).not.toHaveBeenCalledWith('session::fork', expect.anything())
  })

  it('creates a seeded copy when the original session is gone', async () => {
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
      return { settings: {} }
    })
    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      question,
    )
    expect(result.fresh).toBe(true)
    expect(calls.some(([id]) => id === 'session::fork')).toBe(false)
    expect(calls.find(([id]) => id === 'session::create')?.[1]).toMatchObject({
      kind: 'user',
      metadata: {
        [SIDE_CHAT_KEY]: question.sessionId,
        [SIDE_CHAT_QUESTION_KEY]: decisionKey(question),
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
      createDecisionSideChat({ call: call as IiiClient['call'] }, question),
    ).rejects.toThrow('gate unavailable')
    expect(call).toHaveBeenCalledWith('session::delete', {
      session_id: 'partial',
    })
  })

  it('forks a held call at the strictly previous transcript entry', async () => {
    const calls: Array<[string, Record<string, unknown>]> = []
    const immediatelyBefore: TranscriptItem = {
      entry_id: 'result-before-hold',
      custom: { custom_type: 'wake', data: {} },
    }
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return { meta: meta(held.sessionId) }
      if (id === 'session::messages')
        return {
          messages: [
            user('older-spoken-entry'),
            immediatelyBefore,
            assistant('held-call', held.functionCallId, false, held.functionId),
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
      return { settings: {} }
    })

    const result = await createDecisionSideChat(
      { call: call as IiiClient['call'] },
      held,
    )

    expect(result.meta.metadata).toMatchObject({
      [SIDE_CHAT_QUESTION_KEY]: decisionKey(held),
      side_chat_question_kind: 'held',
    })
    expect(calls).toContainEqual([
      'session::fork',
      {
        session_id: held.sessionId,
        entry_id: 'result-before-hold',
        title: 'Side chat · Original run',
      },
    ])
  })

  it('finds an old ask across paginated session messages', async () => {
    const calls: Array<[string, Record<string, unknown>]> = []
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      calls.push([id, input])
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return { meta: meta(question.sessionId) }
      if (id === 'session::messages')
        return input.cursor
          ? { messages: [assistant('old-ask', question.questionId)] }
          : { messages: [user('old-ask-parent')], next_cursor: 'older-page' }
      if (id === 'session::fork')
        return { session_id: 'paged-copy', meta: meta('paged-copy') }
      if (id === 'session::messages-tail')
        return { messages: [user('copied-parent')] }
      if (id === 'session::set-meta')
        return {
          meta: meta('paged-copy', input.metadata as Record<string, unknown>),
        }
      return { settings: {} }
    })

    await createDecisionSideChat({ call: call as IiiClient['call'] }, question)

    expect(calls).toContainEqual([
      'session::messages',
      expect.objectContaining({ cursor: 'older-page' }),
    ])
    expect(calls).toContainEqual([
      'session::fork',
      expect.objectContaining({ entry_id: 'old-ask-parent' }),
    ])
  })

  it('coalesces concurrent opens of the same decision into one fork', async () => {
    let releaseList = () => {}
    const listed = new Promise<{ sessions: SessionMeta[] }>((resolve) => {
      releaseList = () => resolve({ sessions: [] })
    })
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      if (id === 'session::list') return listed
      if (id === 'session::get') return { meta: meta(question.sessionId) }
      if (id === 'session::messages')
        return {
          messages: [user('parent'), assistant('ask', question.questionId)],
        }
      if (id === 'session::fork')
        return { session_id: 'one-copy', meta: meta('one-copy') }
      if (id === 'session::messages-tail')
        return { messages: [user('copied-parent')] }
      if (id === 'session::set-meta')
        return {
          meta: meta('one-copy', input.metadata as Record<string, unknown>),
        }
      return { settings: {} }
    })
    const iii = { call: call as IiiClient['call'] }

    const first = createDecisionSideChat(iii, question)
    const second = createDecisionSideChat(iii, question)
    expect(second).toBe(first)
    releaseList()
    await Promise.all([first, second])

    expect(
      call.mock.calls.filter(([id]) => id === 'session::list'),
    ).toHaveLength(1)
    expect(
      call.mock.calls.filter(([id]) => id === 'session::fork'),
    ).toHaveLength(1)
  })

  it.each([
    ['target call is absent', [user('only-message')]],
    ['target call has no predecessor', [assistant('ask', question.questionId)]],
  ])('fails closed before creating when the %s', async (_, messages) => {
    const call = vi.fn(async (id: string) => {
      if (id === 'session::list') return { sessions: [] }
      if (id === 'session::get') return { meta: meta(question.sessionId) }
      if (id === 'session::messages') return { messages }
      return { settings: {} }
    })

    await expect(
      createDecisionSideChat({ call: call as IiiClient['call'] }, question),
    ).rejects.toThrow('entry before the pending decision is unavailable')
    expect(call.mock.calls.some(([id]) => id === 'session::fork')).toBe(false)
    expect(call.mock.calls.some(([id]) => id === 'session::create')).toBe(false)
  })

  it('serializes approval grants so read-modify-write updates cannot race', async () => {
    let releaseFirstGrant = () => {}
    const firstGrant = new Promise<void>((resolve) => {
      releaseFirstGrant = resolve
    })
    const grants: string[] = []
    const existing = meta('existing-copy', {
      [SIDE_CHAT_QUESTION_KEY]: decisionKey(question),
    })
    const call = vi.fn(async (id: string, input: Record<string, unknown>) => {
      if (id === 'session::list') return { sessions: [existing] }
      if (id === 'approval::add-always-allow') {
        grants.push(String(input.function_id))
        if (grants.length === 1) await firstGrant
      }
      return { settings: {} }
    })

    const opening = createDecisionSideChat(
      { call: call as IiiClient['call'] },
      question,
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
