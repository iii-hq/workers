// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { IiiClient, ObserveSource } from '@/lib/iii-client'
import { useTowerData } from './data'
import { towerFixture } from './fixtures'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

async function settle(): Promise<void> {
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
}

describe('useTowerData live lifecycle', () => {
  it.each([false, true])(
    'keeps scoped card questions and avoids a false stall (harness mirror: %s)',
    async (mirrored) => {
      vi.useFakeTimers()
      const old = Date.now() - 2 * 60 * 60_000
      const card = {
        ...towerFixture.tickets[0],
        project_id: 'p1',
        updated_at: new Date(old).toISOString(),
      }
      const session = {
        ...towerFixture.runs[0].session,
        status: 'done',
        updated_at: old,
        metadata: { kanban_ticket: card.key },
      }
      const call = vi.fn(async (id: string) => {
        if (id === 'session::list') return { sessions: [session] }
        if (id === 'session::get') return { meta: session }
        if (id === 'harness::status')
          return {
            session_id: session.session_id,
            status: 'completed',
            children: [],
            open_asks: 0,
            expects_wake: true,
          }
        if (id === 'session::messages-tail') return { messages: [] }
        if (id === 'kanban::board::get')
          return {
            columns: [
              {
                id: 'in_progress',
                label: 'In progress',
                tickets: [card],
                total: 1,
              },
            ],
            done_status: 'done',
            id_prefix: 'KAN',
          }
        if (id === 'kanban::project::list') return { projects: [] }
        if (id === 'kanban::release::list') return { releases: [] }
        if (id === 'harness::ask::list')
          return {
            entries: [
              {
                session_id: session.session_id,
                question_id: 'question-1',
                turn_id: 'turn-1',
                questions: [
                  {
                    header: 'Scope',
                    question: 'Keep the current scope?',
                    options: [],
                  },
                ],
                about: { class: 'scope' },
                asked_at: old,
                on_expiry: 'block',
                state: 'open',
              },
              ...(mirrored
                ? [
                    {
                      session_id: 'deleted-session',
                      question_id: 'board-deleted',
                      turn_id: 'turn-deleted',
                      questions: [],
                      about: { ticket: 'KAN-DELETED' },
                      asked_at: old + 2,
                      on_expiry: 'block',
                      state: 'open',
                    },
                  ]
                : []),
            ],
          }
        if (id === 'approval::list-pending') return { pending: [] }
        if (id === 'kanban::question::list')
          return {
            questions: [
              {
                comment: {
                  id: 'comment-live',
                  ticket_id: card.key,
                  body: 'Live card question',
                  author: 'backend-engineer',
                  created_at: new Date(old + 1).toISOString(),
                  question: {
                    session_id: 'board-session',
                    question_id: 'board-live',
                    questions: [
                      {
                        header: 'Scope',
                        question: 'Use the linked card?',
                        options: [],
                      },
                    ],
                    asked_at: old + 1,
                  },
                },
                state: 'open',
              },
              {
                comment: {
                  id: 'comment-deleted',
                  ticket_id: 'KAN-DELETED',
                  body: 'Deleted card question',
                  author: 'backend-engineer',
                  created_at: new Date(old + 2).toISOString(),
                  question: {
                    session_id: 'deleted-session',
                    question_id: 'board-deleted',
                    questions: [
                      {
                        header: 'Scope',
                        question: 'Keep the deleted card decision?',
                        options: [],
                      },
                    ],
                    asked_at: old + 2,
                  },
                },
                state: 'open',
              },
            ],
          }
        if (id === 'ade::configuration-id') return { id: 'ade' }
        if (id === 'configuration::get') return { value: {} }
        throw new Error('Progress source unavailable')
      })
      const iii = {
        browserId: 'progress-test',
        call: call as IiiClient['call'],
        on: () => () => {},
        observe: () => () => {},
      } as unknown as IiiClient
      const container = document.createElement('div')
      const root = createRoot(container)
      function Probe() {
        const data = useTowerData(iii, 'p1')
        return (
          <div>
            {JSON.stringify({
              runs: data.runs.map((run) => ({
                state: run.state,
                subtasks: run.subtasks,
              })),
              items: data.items.map((item) => ({
                id: item.id,
                card: item.card?.key,
              })),
            })}
          </div>
        )
      }
      try {
        await act(async () => root.render(<Probe />))
        await settle()
        await settle()
        expect(JSON.parse(container.textContent ?? '{}')).toEqual({
          runs: [{ state: 'waiting', subtasks: null }],
          items: [
            {
              id: `question:${session.session_id}:question-1`,
              card: card.key,
            },
            {
              id: 'question:board-session:board-live',
              card: card.key,
            },
            {
              id: 'question:deleted-session:board-deleted',
            },
          ],
        })
        expect(call).toHaveBeenCalledWith('kanban::ticket::list', {
          parent_id: card.key,
        })
      } finally {
        act(() => root.unmount())
      }
    },
  )

  it('does not carry scoped decision records into a newly selected project', async () => {
    const row = {
      comment: {
        id: 'q',
        ticket_id: 'deleted-card',
        body: 'Question',
        author: 'agent',
        created_at: '2026-11-21T00:00:00Z',
        question: {
          session_id: 's',
          question_id: 'q',
          questions: [],
          asked_at: 1,
        },
      },
      state: 'withdrawn',
    }
    const call = vi.fn(
      async (id: string, payload?: Record<string, unknown>) => {
        if (id === 'kanban::question::list')
          return payload?.project === 'p1'
            ? { questions: [row] }
            : new Promise(() => {})
        if (id === 'kanban::project::list') return { projects: [] }
        if (id === 'kanban::release::list') return { releases: [] }
        if (id === 'kanban::board::get')
          return { columns: [], done_status: 'done', id_prefix: 'KAN' }
        if (id === 'session::list') return { sessions: [] }
        if (id === 'harness::ask::list') return { entries: [] }
        if (id === 'approval::list-pending') return { pending: [] }
        if (id === 'ade::configuration-id') return { id: 'ade' }
        if (id === 'configuration::get') return { value: {} }
        throw new Error(id)
      },
    )
    const iii = {
      browserId: 'scope-switch',
      call: call as IiiClient['call'],
      on: () => () => {},
      observe: () => () => {},
    } as unknown as IiiClient
    const container = document.createElement('div')
    const root = createRoot(container)
    function Probe({ scope }: { scope: string }) {
      const data = useTowerData(iii, scope)
      return (
        <div>
          {JSON.stringify({
            questions: data.questions.map((question) => question.key),
            loading: data.loading.questions,
          })}
        </div>
      )
    }
    try {
      await act(() => root.render(<Probe scope="p1" />))
      await settle()
      expect(JSON.parse(container.textContent ?? '{}')).toEqual({
        questions: ['s/q'],
        loading: false,
      })
      await act(() => root.render(<Probe scope="p2" />))
      await settle()
      expect(JSON.parse(container.textContent ?? '{}')).toEqual({
        questions: [],
        loading: true,
      })
    } finally {
      await act(() => root.unmount())
    }
  })

  it('scopes reads, debounces relevant events, polls PRs, and cleans up', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-11-22T12:00:00Z'))
    const handlers = new Map<string, (payload: unknown) => void>()
    const observers = new Map<string, Set<string>>()
    const disposers: ReturnType<typeof vi.fn>[] = []
    const call = vi.fn(async (functionId: string) => {
      if (functionId === 'kanban::project::list') {
        return {
          projects: [
            {
              id: 'p1',
              name: 'Project',
              description: '',
              repos: ['owner/repo'],
              total: 0,
              done: 0,
              open_questions: 0,
              counts: {},
            },
          ],
        }
      }
      if (functionId === 'kanban::release::list') return { releases: [] }
      if (functionId === 'kanban::board::get') {
        return {
          columns: [],
          done_status: 'done',
          canceled_status: null,
          id_prefix: 'KAN',
        }
      }
      if (functionId === 'session::list') {
        return {
          sessions: [
            {
              session_id: 'plain-ended-chat',
              title: 'Ended chat',
              description: '',
              status: 'done',
              kind: 'user',
              metadata: { fs_scope: { root: '/ordinary-chat' } },
              created_at: 1,
              updated_at: 2,
              message_count: 0,
            },
          ],
        }
      }
      if (functionId === 'session::get') {
        return {
          meta: {
            session_id: 'plain-ended-chat',
            title: 'Ended chat',
            description: '',
            status: 'done',
            kind: 'user',
            metadata: { fs_scope: { root: '/ordinary-chat' } },
            created_at: 1,
            updated_at: 2,
            message_count: 0,
          },
        }
      }
      if (functionId === 'harness::status') {
        return {
          session_id: 'plain-ended-chat',
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
      if (functionId === 'ade::configuration-id') return { id: 'ade' }
      if (functionId === 'configuration::get') return { value: {} }
      if (functionId === 'harness::ask::list') return { entries: [] }
      if (functionId === 'approval::list-pending') return { pending: [] }
      if (functionId === 'kanban::question::list') return { questions: [] }
      if (functionId === 'github::pr::list') return { value: [] }
      throw new Error(`unexpected ${functionId}`)
    })
    const iii = {
      browserId: 'browser',
      call: call as IiiClient['call'],
      on(functionId: string, handler: (payload: unknown) => void) {
        handlers.set(functionId, handler)
        const off = vi.fn(() => handlers.delete(functionId))
        disposers.push(off)
        return off
      },
      observe(source: ObserveSource, target: string) {
        const targets = observers.get(source.function_id) ?? new Set<string>()
        targets.add(target)
        observers.set(source.function_id, targets)
        const off = vi.fn(() => targets.delete(target))
        disposers.push(off)
        return off
      },
    } as unknown as IiiClient
    const emit = (source: string, payload: unknown) => {
      for (const target of observers.get(source) ?? []) {
        handlers.get(target.replace(/::browser$/, ''))?.(payload)
      }
    }
    const count = (functionId: string) =>
      call.mock.calls.filter(([called]) => called === functionId).length

    const container = document.createElement('div')
    const root = createRoot(container)
    function Probe({ scope }: { scope: string }) {
      useTowerData(iii, scope)
      return null
    }
    await act(async () => root.render(<Probe scope="all" />))
    await settle()

    expect(call).toHaveBeenCalledWith('kanban::board::get', {})
    expect(count('github::pr::list')).toBe(1)
    expect(count('shell::exec')).toBe(0)
    expect(observers.has('session::on-status-changed')).toBe(false)
    expect(observers.has('state::on-changed')).toBe(false)

    await act(async () => root.render(<Probe scope="p1" />))
    await settle()
    expect(call).toHaveBeenCalledWith('kanban::board::get', {
      project_id: 'p1',
    })

    const sessionsBeforeChild = count('session::list')
    act(() =>
      emit('harness::on-turn-completed', {
        session_id: 'child',
        parent_session_id: 'root',
      }),
    )
    await act(async () => vi.advanceTimersByTimeAsync(2_100))
    await settle()
    expect(count('session::list')).toBe(sessionsBeforeChild)

    act(() =>
      emit('harness::on-turn-completed', {
        session_id: 'root',
        terminal: true,
      }),
    )
    await act(async () => vi.advanceTimersByTimeAsync(1_999))
    expect(count('session::list')).toBe(sessionsBeforeChild)
    await act(async () => vi.advanceTimersByTimeAsync(1))
    await settle()
    expect(count('session::list')).toBe(sessionsBeforeChild + 1)

    const boardBefore = count('kanban::board::get')
    const sessionsBeforeBoard = count('session::list')
    const prsBeforeBoard = count('github::pr::list')
    act(() => emit('kanban::on-change', { ticket: { id: 'KAN-1' } }))
    await act(async () => vi.advanceTimersByTimeAsync(2_000))
    await settle()
    expect(count('kanban::board::get')).toBe(boardBefore + 1)
    expect(count('session::list')).toBe(sessionsBeforeBoard + 1)
    expect(count('github::pr::list')).toBe(prsBeforeBoard)

    const prsBeforeMinute = count('github::pr::list')
    await act(async () => vi.advanceTimersByTimeAsync(60_000))
    await settle()
    expect(count('github::pr::list')).toBe(prsBeforeMinute)

    window.dispatchEvent(new Event('focus'))
    await settle()
    expect(count('github::pr::list')).toBe(prsBeforeMinute + 1)

    const prsBeforePoll = count('github::pr::list')
    await act(async () => vi.advanceTimersByTimeAsync(5 * 60_000))
    await settle()
    expect(count('github::pr::list')).toBe(prsBeforePoll + 1)

    act(() => root.unmount())
    expect(disposers.every((dispose) => dispose.mock.calls.length === 1)).toBe(
      true,
    )
    const callsAfterUnmount = call.mock.calls.length
    window.dispatchEvent(new Event('focus'))
    await act(async () => vi.advanceTimersByTimeAsync(10 * 60_000))
    expect(call.mock.calls).toHaveLength(callsAfterUnmount)
  })

  it('catches up on focus when observer setup is refused', async () => {
    const call = vi.fn(async (functionId: string) => {
      if (functionId === 'kanban::project::list') return { projects: [] }
      if (functionId === 'kanban::release::list') return { releases: [] }
      if (functionId === 'kanban::board::get') {
        return { columns: [], done_status: 'done', id_prefix: 'KAN' }
      }
      if (functionId === 'session::list') return { sessions: [] }
      if (functionId === 'ade::configuration-id') return { id: 'ade' }
      if (functionId === 'configuration::get') return { value: {} }
      if (functionId === 'harness::ask::list') return { entries: [] }
      if (functionId === 'approval::list-pending') return { pending: [] }
      if (functionId === 'kanban::question::list') return { questions: [] }
      throw new Error(functionId)
    })
    const iii = {
      browserId: 'browser',
      call: call as IiiClient['call'],
      on: () => vi.fn(),
      observe: () => {
        throw new Error('observer unavailable')
      },
    } as unknown as IiiClient
    const container = document.createElement('div')
    const root = createRoot(container)
    function Probe() {
      useTowerData(iii, 'all')
      return null
    }
    await act(async () => root.render(<Probe />))
    await settle()
    const before = call.mock.calls.filter(
      ([functionId]) => functionId === 'kanban::board::get',
    ).length

    window.dispatchEvent(new Event('focus'))
    await settle()
    expect(
      call.mock.calls.filter(
        ([functionId]) => functionId === 'kanban::board::get',
      ),
    ).toHaveLength(before + 1)
    act(() => root.unmount())
  })
})
