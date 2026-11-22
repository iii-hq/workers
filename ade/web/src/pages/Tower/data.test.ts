import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import {
  cardForSession,
  checkState,
  fetchPullRequests,
  fetchTowerAsks,
  fetchTowerBoard,
  fetchTowerHolds,
  fetchTowerQuestions,
  fetchTowerRuns,
  isRootTurnEvent,
  mergeTowerQuestions,
  progressFor,
} from './data'
import { towerFixture } from './fixtures'
import type { TowerRelease, TowerTicket } from './model'

function client(call: IiiClient['call']): IiiClient {
  return { browserId: 'test', call } as IiiClient
}

describe('tower live events', () => {
  it('accepts depth-zero starts and parentless completions only', () => {
    expect(isRootTurnEvent({ depth: 0, session_id: 'root' })).toBe(true)
    expect(isRootTurnEvent({ depth: 1, session_id: 'child' })).toBe(false)
    expect(isRootTurnEvent({ session_id: 'root', terminal: true })).toBe(true)
    expect(isRootTurnEvent({ ticket: { id: 'KAN-1' } })).toBe(true)
    expect(
      isRootTurnEvent({ session_id: 'child', parent_session_id: 'root' }),
    ).toBe(false)
    expect(
      isRootTurnEvent({ session_id: 'child', parent: { session_id: 'root' } }),
    ).toBe(false)
  })
})

describe('pull request checks', () => {
  it('does not claim an empty checks list passed', () => {
    expect(checkState([])).toEqual({ checks: 'none', failing: 0 })
  })

  it('unwraps list, checks and view responses and links the card', async () => {
    const call = vi.fn(async (functionId: string) => {
      if (functionId === 'github::pr::list') {
        return {
          value: [
            {
              number: 7,
              title: 'Ship KAN-7',
              url: 'https://github.com/owner/repo/pull/7',
              headRefName: 'feat/KAN-7',
              isDraft: false,
              createdAt: '2026-11-20T00:00:00Z',
              updatedAt: '2026-11-21T00:00:00Z',
            },
          ],
        }
      }
      if (functionId === 'github::pr::checks') {
        return { value: [{ bucket: 'fail', state: 'FAILURE' }] }
      }
      if (functionId === 'github::pr::view') {
        return {
          value: {
            reviewDecision: 'REVIEW_REQUIRED',
            mergeable: 'MERGEABLE',
          },
        }
      }
      throw new Error(functionId)
    }) as IiiClient['call']
    const card = { id: '7', key: 'KAN-7' } as TowerTicket

    const result = await fetchPullRequests(client(call), ['owner/repo'], [card])
    expect(result[0]).toMatchObject({
      repo: 'owner/repo',
      number: 7,
      checks: 'failing',
      failing: 1,
      card,
      waitReason: 'CI failing',
    })
    expect(call).toHaveBeenCalledWith('github::pr::list', {
      repo: 'owner/repo',
      state: 'open',
      limit: 1_000,
    })
    expect(call).toHaveBeenCalledWith('github::pr::checks', {
      repo: 'owner/repo',
      number: 7,
    })
    expect(call).toHaveBeenCalledWith('github::pr::view', {
      repo: 'owner/repo',
      number: 7,
    })
  })
})

describe('board projections', () => {
  it('keeps the newest two releases, then active and target-ordered planned releases', async () => {
    const release = (
      id: string,
      status: TowerRelease['status'],
      date: string,
    ): TowerRelease => ({
      id,
      name: id,
      status,
      released_at: status === 'released' ? date : null,
      target_at: status === 'planned' ? date : null,
      total: 9,
      done: 8,
      open_questions: 7,
      counts: {},
      projects: [{ project_id: 'p1', total: 3, done: 2, open_questions: 1 }],
    })
    const releases = [
      release('released-old', 'released', '2026-01-01'),
      release('planned-late', 'planned', '2027-02-01'),
      release('released-new', 'released', '2026-11-01'),
      release('active', 'active', ''),
      release('released-mid', 'released', '2026-06-01'),
      release('planned-early', 'planned', '2026-12-01'),
    ]
    const call = vi.fn(async (functionId: string) => {
      if (functionId === 'kanban::project::list') return { projects: [] }
      if (functionId === 'kanban::release::list') return { releases }
      if (functionId === 'kanban::board::get') {
        return { columns: [], done_status: 'done', id_prefix: 'KAN' }
      }
      throw new Error(functionId)
    }) as IiiClient['call']

    const result = await fetchTowerBoard(client(call), 'p1')
    expect(result.releases.map((item) => item.id)).toEqual([
      'released-new',
      'released-mid',
      'active',
      'planned-early',
      'planned-late',
    ])
    expect(result.releases[0]).toMatchObject({
      total: 3,
      done: 2,
      open_questions: 1,
    })
    expect(call).toHaveBeenCalledWith('kanban::board::get', {
      project_id: 'p1',
    })
  })

  it('groups subtask progress without counting todo as in progress', () => {
    const task = (status: string): TowerTicket =>
      ({ status, release_id: 'r1' }) as TowerTicket
    expect(
      progressFor(
        [task('done'), task('in_progress'), task('todo'), task('canceled')],
        [{ id: 'r1', name: 'Release' } as TowerRelease],
        'done',
        'canceled',
      ),
    ).toEqual([
      {
        releaseId: 'r1',
        releaseName: 'Release',
        done: 1,
        inProgress: 1,
        total: 3,
      },
    ])
  })
})

describe('needs-you catch-up', () => {
  it('walks ask, approval and question cursors independently', async () => {
    const call = vi.fn(
      async (functionId: string, payload?: Record<string, unknown>) => {
        if (functionId === 'harness::ask::list') {
          if (!payload?.cursor) {
            return {
              entries: [{ session_id: 's', question_id: 'ask-1' }],
              cursor: 'ask-next',
            }
          }
          return { entries: [{ session_id: 's', question_id: 'ask-2' }] }
        }
        if (functionId === 'approval::list-pending') {
          return payload?.cursor
            ? { pending: [{ function_call_id: 'hold-2' }] }
            : { pending: [{ function_call_id: 'hold-1' }], next_cursor: 'next' }
        }
        if (functionId === 'kanban::question::list') {
          return payload?.cursor
            ? { questions: [{ state: 'answered' }] }
            : { questions: [{ state: 'recorded' }], cursor: 'question-next' }
        }
        throw new Error(functionId)
      },
    ) as IiiClient['call']

    const [asks, holds, questions] = await Promise.all([
      fetchTowerAsks(client(call)),
      fetchTowerHolds(client(call)),
      fetchTowerQuestions(client(call), 'project-1'),
    ])
    expect(asks.map((entry) => entry.question_id)).toEqual(['ask-1', 'ask-2'])
    expect(holds.map((entry) => entry.function_call_id)).toEqual([
      'hold-1',
      'hold-2',
    ])
    expect(questions.map((entry) => entry.state)).toEqual([
      'recorded',
      'answered',
    ])
    expect(call).toHaveBeenCalledWith('approval::list-pending', {
      limit: 200,
      cursor: 'next',
    })
    expect(call).toHaveBeenCalledWith('kanban::question::list', {
      limit: 500,
      project: 'project-1',
      cursor: 'question-next',
    })
  })

  it('merges by session and question with the harness lifecycle authoritative', () => {
    const board = {
      comment: {
        id: 'comment-1',
        ticket_id: 'KAN-1',
        body: 'Question',
        author: 'backend-engineer',
        created_at: '2026-11-22T10:00:00Z',
        question: {
          session_id: 'session-1',
          question_id: 'question-1',
          questions: [
            { header: 'Scope', question: 'Board copy?', options: [] },
          ],
          asked_at: 10,
        },
      },
      state: 'answered' as const,
      answer: {
        id: 'answer-1',
        ticket_id: 'KAN-1',
        body: 'Keep it',
        author: 'Ada',
        created_at: '2026-11-22T11:00:00Z',
        answer: {
          answer_text: 'Keep it',
          outcome: 'answered' as const,
        },
      },
    }
    const harness = {
      session_id: 'session-1',
      question_id: 'question-1',
      turn_id: 'turn-1',
      questions: [{ header: 'Scope', question: 'Harness copy?', options: [] }],
      about: { ticket: 'KAN-2', class: 'scope' as const },
      asked_at: 20,
      on_expiry: 'block' as const,
      state: 'answering' as const,
    }

    expect(mergeTowerQuestions([harness], [board])).toEqual([
      expect.objectContaining({
        key: 'session-1/question-1',
        ticketId: 'KAN-2',
        commentId: 'comment-1',
        state: 'answering',
        askedAt: 20,
        harness,
        board,
      }),
    ])
    expect(mergeTowerQuestions([], [board])[0].settledAt).toBe(
      Date.parse('2026-11-22T11:00:00Z'),
    )
  })

  it('can reconcile one session through the shared ask fetcher', async () => {
    const call = vi.fn(async () => ({ entries: [] })) as IiiClient['call']

    await fetchTowerAsks(client(call), 'session-1')

    expect(call).toHaveBeenCalledWith('harness::ask::list', {
      session_id: 'session-1',
      limit: 200,
    })
  })

  it('keeps approval holds readable when the ask source is down', async () => {
    const call = vi.fn(async (functionId: string) => {
      if (functionId === 'approval::list-pending') return { pending: [] }
      throw new Error('harness unavailable')
    }) as IiiClient['call']
    const [asks, holds] = await Promise.allSettled([
      fetchTowerAsks(client(call)),
      fetchTowerHolds(client(call)),
    ])
    expect(asks.status).toBe('rejected')
    expect(holds).toEqual({ status: 'fulfilled', value: [] })
  })
})

describe('run catch-up', () => {
  it('resolves card references with the same case and whitespace rules as kanban', () => {
    expect(
      cardForSession(
        {
          ...towerFixture.runs[0].session,
          metadata: { kanban_ticket: '  kan-616 ' },
        },
        towerFixture.tickets,
      )?.key,
    ).toBe('KAN-616')
  })
  it('skips an empty root without hiding roots that have harness status', async () => {
    const meta = (session_id: string): SessionMeta => ({
      session_id,
      title: session_id,
      description: '',
      status: 'done',
      kind: 'user',
      created_at: 1,
      updated_at: 2,
      message_count: 0,
    })
    const call = vi.fn(
      async (functionId: string, payload?: Record<string, unknown>) => {
        if (functionId === 'session::list')
          return { sessions: [meta('empty'), meta('run')] }
        if (functionId === 'ade::configuration-id') return { id: 'ade' }
        if (functionId === 'configuration::get') return { value: {} }
        if (functionId === 'session::get')
          return { meta: meta(String(payload?.session_id)) }
        if (functionId === 'harness::status') {
          return payload?.session_id === 'empty'
            ? null
            : {
                session_id: 'run',
                status: 'completed',
                children: [],
                expects_wake: false,
                open_asks: 0,
              }
        }
        if (functionId === 'session::messages-tail') return { messages: [] }
        if (functionId === 'harness::metrics') {
          return {
            complete: true,
            totals: { sessions: 1, input_tokens: null, output_tokens: null },
          }
        }
        throw new Error(functionId)
      },
    ) as IiiClient['call']

    const result = await fetchTowerRuns(client(call))
    expect(result.rows.map((row) => row.session.session_id)).toEqual(['run'])
    expect(call).toHaveBeenCalledWith('session::list', {
      roots: true,
      kinds: ['user', 'automation'],
      limit: 200,
    })
    expect(call).toHaveBeenCalledWith('harness::status', {
      session_id: 'run',
      verbose: true,
    })
  })
})
