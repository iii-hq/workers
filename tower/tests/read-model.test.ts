import assert from 'node:assert/strict'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defaultConfig } from '../src/config.js'
import { TowerReadModel, fetchTowerRuns, sourceHash } from '../src/read-model.js'
import type {
  AskEntry,
  KanbanQuestionView,
  SessionMeta,
  TowerClient,
  TowerRelease,
  TowerTicket,
} from '../src/types.js'

const at = Date.parse('2026-11-22T12:00:00Z')
const card: TowerTicket = {
  id: 'c',
  key: 'KAN-1',
  title: 'Ship',
  status: 'in_progress',
  priority: 'medium',
  labels: ['blocked'],
  pills: [],
  created_at: new Date(at).toISOString(),
  updated_at: new Date(at - 29 * 60000).toISOString(),
  project_id: 'p',
  release_id: 'r',
  open_questions: 0,
}
const session: SessionMeta = {
  session_id: 'root',
  title: 'Build',
  description: '',
  kind: 'automation',
  status: 'done',
  created_at: at - 60000,
  updated_at: at - 29 * 60000,
  message_count: 1,
  metadata: { kanban_ticket: 'KAN-1' },
}
const release: TowerRelease = {
  id: 'r',
  name: 'R1',
  status: 'active',
  target_at: new Date(at + 5 * 86400000).toISOString(),
  total: 2,
  done: 1,
  open_questions: 0,
  counts: {},
  projects: [{ project_id: 'p', total: 2, done: 1, open_questions: 0 }],
}
const ask = (id: string, cls: AskEntry['about']['class'] = 'scope'): AskEntry => ({
  session_id: 'root',
  question_id: id,
  turn_id: 't',
  questions: [{ header: 'Ship', question: 'Ship?', options: [] }],
  about: { ticket: 'KAN-1', class: cls },
  asked_at: at - 2 * 86400000,
  on_expiry: 'block',
  state: 'open',
})

function fake(
  options: {
    sessions?: SessionMeta[]
    asks?: AskEntry[]
    questions?: KanbanQuestionView[]
    releases?: TowerRelease[]
    working?: boolean
    repos?: string[]
  } = {},
) {
  const call = vi.fn(async (id: string, input?: Record<string, unknown>): Promise<unknown> => {
    if (id === 'kanban::project::list')
      return { projects: [{ id: 'p', name: 'P', repos: options.repos ?? ['owner/repo'] }] }
    if (id === 'kanban::release::list') return { releases: options.releases ?? [release] }
    if (id === 'kanban::board::get')
      return {
        columns: [{ id: 'in_progress', label: 'In progress', tickets: [card], total: 1 }],
        done_status: 'done',
        id_prefix: 'KAN',
      }
    if (id === 'session::list') return { sessions: options.sessions ?? [session] }
    if (id === 'harness::status')
      return {
        session_id: input?.session_id,
        turn_id: 't',
        step: 1,
        context: { total: 120 },
        status: options.working ? 'running' : 'completed',
        children: [],
        expects_wake: true,
        armed_wakes: [{ expires_at: null }],
        open_asks: 0,
      }
    if (id === 'session::messages-tail')
      return {
        messages: [
          {
            entry_id: 'm',
            message: { role: 'assistant', content: [{ type: 'text', text: 'Building' }] },
          },
        ],
      }
    if (id === 'kanban::ticket::list') return { tickets: [] }
    if (id === 'harness::ask::list') return { entries: options.asks ?? [] }
    if (id === 'kanban::question::list') return { questions: options.questions ?? [] }
    if (id === 'github::pr::list')
      return {
        value: [
          {
            number: 1,
            title: 'KAN-1',
            headRefName: 'feature',
            updatedAt: new Date(at).toISOString(),
          },
        ],
      }
    if (id === 'github::pr::checks') return { value: [{ bucket: 'fail' }] }
    if (id === 'github::pr::view') return { value: {} }
    throw new Error(id)
  })
  return { call, iii: { call: call as TowerClient['call'] } }
}

const models: TowerReadModel[] = []
function model(iii: TowerClient, config = defaultConfig()) {
  const changed = vi.fn()
  const read = new TowerReadModel(iii, () => config, changed)
  models.push(read)
  return { read, changed }
}
afterEach(() => {
  for (const read of models.splice(0)) read.dispose()
  vi.useRealTimers()
})

describe('server read model', () => {
  it('turns a silent root into a stalled run from the deadline timer alone', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(at)
    const { read, changed } = model(fake().iii)
    expect((await read.read('p')).runs[0].state).toBe('waiting')
    await vi.advanceTimersByTimeAsync(60000)
    expect((await read.read('p')).runs[0].state).toBe('stalled')
    expect(changed).toHaveBeenLastCalledWith(
      'p',
      new Date(at + 60000).toISOString(),
      expect.arrayContaining(['runs', 'items']),
    )
  })

  it('shares one repository read across three tabs and all, expires watches and reads a stale watch immediately', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(at)
    const { call, iii } = fake({ sessions: [] })
    const { read } = model(iii)
    await Promise.all([read.watch('p'), read.watch('p'), read.watch('p'), read.watch('all')])
    const count = (id: string) => call.mock.calls.filter(([name]) => name === id).length
    expect(count('github::pr::list')).toBe(1)
    expect(count('github::pr::checks')).toBe(1)
    expect(count('session::list')).toBe(2)
    await read.watch('p')
    await read.watch('p')
    expect(count('session::list')).toBe(2)
    expect((await read.read('p')).prs[0].as_of).toBe(new Date(at).toISOString())
    await vi.advanceTimersByTimeAsync(31 * 60000)
    expect(count('github::pr::list')).toBe(1)
    await read.watch('p')
    expect(count('github::pr::checks')).toBe(2)
    expect((await read.read('p')).prs[0].as_of).toBe(new Date(at + 31 * 60000).toISOString())
  })

  it('uses idle cadence for a live run and watched cadence while tabs renew the watch', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(at)
    const { call, iii } = fake()
    const { read } = model(iii)
    await read.watch('p')
    for (let i = 0; i < 5; i++) {
      await vi.advanceTimersByTimeAsync(60000)
      await read.watch('p')
    }
    expect(call.mock.calls.filter(([id]) => id === 'github::pr::checks')).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(29 * 60000)
    expect(call.mock.calls.filter(([id]) => id === 'github::pr::checks')).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(60000)
    expect(call.mock.calls.filter(([id]) => id === 'github::pr::checks')).toHaveLength(3)
  })

  it('reads no GitHub with no watcher/live run and never calls a held-call or action function', async () => {
    const { call, iii } = fake({ sessions: [] })
    const { read } = model(iii)
    const data = await read.read('all')
    expect(data.prs).toEqual([])
    expect(
      call.mock.calls.some(
        ([id]) =>
          id.startsWith('approval::') ||
          id.startsWith('github::') ||
          /answer|send|resolve/.test(id),
      ),
    ).toBe(false)
    expect(data.items.some((item) => item.kind === 'held')).toBe(false)
    expect(data.repositories).toEqual(['owner/repo'])
  })

  it('excludes writer and side chats from every read and hash, including asks', async () => {
    const excluded = [
      { ...session, session_id: 'tower-p', metadata: { origin: 'tower' } },
      { ...session, session_id: 'side', metadata: { side_chat_of: 'root' } },
    ]
    const { iii, call } = fake({
      sessions: [session, ...excluded],
      asks: excluded.map((s) => ({ ...ask(s.session_id), session_id: s.session_id })),
    })
    const { read } = model(iii)
    const first = await read.read('all')
    expect(first.runs.map((run) => run.session.session_id)).toEqual(['root'])
    expect(first.questions).toEqual([])
    expect(
      call.mock.calls
        .filter(([id]) => id === 'harness::status')
        .map(([, input]) => input?.session_id),
    ).toEqual(['root'])
    expect(sourceHash(first)).toBe(first.sourceHash)
  })

  it('debounces root events for two seconds and ignores child/writer turns', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(at)
    const { call, iii } = fake()
    const { read } = model(iii)
    await read.read('p')
    call.mockClear()
    read.onSource({ depth: 1 }, true)
    read.onSource({ parent_session_id: 'root' }, true)
    await vi.advanceTimersByTimeAsync(2000)
    expect(call).not.toHaveBeenCalled()
    read.onSource({ depth: 0 }, true)
    await vi.advanceTimersByTimeAsync(1000)
    read.onSource({ depth: 0 }, true)
    await vi.advanceTimersByTimeAsync(1999)
    expect(call).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(call).toHaveBeenCalledWith('session::list', expect.anything())
  })

  it('retains completed turn identity when a new turn starts before the debounce rebuild', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(at)
    const { call, iii } = fake()
    const { read } = model(iii)
    await read.read('p')
    const original = call.getMockImplementation()
    assert.ok(original)
    call.mockImplementation(async (id, input) =>
      id === 'harness::status'
        ? {
            ...((await original(id, input)) as Record<string, unknown>),
            turn_id: 'new-running',
            status: 'running',
          }
        : original(id, input),
    )
    read.onSource({ session_id: 'root', turn_id: 'completed-between', status: 'completed' }, true)
    await vi.advanceTimersByTimeAsync(2000)
    const data = await read.read('p')
    expect(data.completed_root_turns).toEqual(['root/completed-between'])
    expect(data.runs[0].status.turn_id).toBe('new-running')
  })

  it('keeps side chats excluded when the session source fails during reconciliation', async () => {
    const options = {
      sessions: [session, { ...session, session_id: 'side', metadata: { side_chat_of: 'root' } }],
      asks: [] as AskEntry[],
    }
    const { call, iii } = fake(options)
    const { read } = model(iii)
    const hash = (await read.read('all')).sourceHash
    options.asks.push({ ...ask('private'), session_id: 'side' })
    const original = call.getMockImplementation()
    assert.ok(original)
    call.mockImplementation(async (id, input) => {
      if (id === 'session::list') throw new Error('sessions unavailable')
      return original(id, input)
    })
    const data = await read.rebuild('all')
    expect(data.questions).toEqual([])
    expect(data.sourceHash).toBe(hash)
    expect(data.errors.sessions).toBe('sessions unavailable')
  })

  it('does not exclude legitimate runs merely because their id starts with tower-', async () => {
    const { read } = model(fake({ sessions: [{ ...session, session_id: 'tower-business' }] }).iii)
    expect((await read.read('all')).runs[0].session.session_id).toBe('tower-business')
  })

  it('announces project-only changes even when cards and runs are unchanged', async () => {
    const { call, iii } = fake({ sessions: [] })
    const { read, changed } = model(iii)
    await read.read('all')
    const original = call.getMockImplementation()
    assert.ok(original)
    call.mockImplementation(async (id, input) =>
      id === 'kanban::project::list'
        ? { projects: [{ id: 'p', name: 'Renamed', repos: ['owner/repo'] }] }
        : original(id, input),
    )
    await read.rebuild('all')
    expect(changed).toHaveBeenLastCalledWith('all', expect.any(String), ['projects'])
  })

  it('keeps board data available when GitHub is missing', async () => {
    const { call, iii } = fake({ sessions: [] })
    const original = call.getMockImplementation()
    assert.ok(original)
    call.mockImplementation(async (id, input) => {
      if (id.startsWith('github::')) throw new Error('function_not_found')
      return original(id, input)
    })
    const { read } = model(iii)
    await read.watch('p')
    const data = await read.read('p')
    expect(data.errors.prs).toBe('function_not_found')
    expect(data.tickets).toEqual([card])
    expect(data.prs).toEqual([])
  })
})

describe('context accounting', () => {
  it('caches totals by turn and step and estimates the messages tail without context or a turn', async () => {
    const { call, iii } = fake()
    const cache = new Map<string, { key: string; total: number }>()
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(120)
    const original = call.getMockImplementation()
    assert.ok(original)
    call.mockImplementation(async (id, input) =>
      id === 'harness::status'
        ? { ...((await original(id, input)) as Record<string, unknown>), context: { total: 900 } }
        : original(id, input),
    )
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(120)
    call.mockImplementation(async (id, input) =>
      id === 'harness::status'
        ? {
            ...((await original(id, input)) as Record<string, unknown>),
            step: 2,
            context: { total: 900 },
          }
        : original(id, input),
    )
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(900)
    call.mockImplementation(async (id, input) =>
      id === 'harness::status' ? null : original(id, input),
    )
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(2)
  })
})

describe('fallback cache', () => {
  it('keeps a tail estimate for the same turn and step, and ignores context without a turn', async () => {
    const { call, iii } = fake()
    const original = call.getMockImplementation()
    assert.ok(original)
    let step = 1
    let turn: string | null = 't'
    let text = 'tiny'
    call.mockImplementation(async (id, input) => {
      if (id === 'harness::status')
        return {
          ...((await original(id, input)) as Record<string, unknown>),
          turn_id: turn,
          step,
          context: turn ? null : { total: 999 },
        }
      if (id === 'session::messages-tail')
        return {
          messages: [
            { entry_id: 'm', message: { role: 'assistant', content: [{ type: 'text', text }] } },
          ],
        }
      return original(id, input)
    })
    const cache = new Map<string, { key: string; total: number }>()
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(1)
    text = 'x'.repeat(80)
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(1)
    step = 2
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(20)
    turn = null
    expect((await fetchTowerRuns(iii, cache)).rows[0].context_size).toBe(20)
  })
})

describe('CEO view', () => {
  it('applies all three risk rules, project escalation, exceptions, and a seven-day median', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(at)
    const asks = [
      ask('routine', 'routine'),
      { ...ask('scope'), about: { ticket: 'KAN-1', class: 'scope' as const, blocks: ['KAN-2'] } },
      { ...ask('done-a'), state: 'answered' as const, asked_at: at - 3000, settled_at: at - 1000 },
      { ...ask('done-b'), state: 'answered' as const, asked_at: at - 9000, settled_at: at - 1000 },
      {
        ...ask('old'),
        state: 'answered' as const,
        asked_at: at - 9 * 86400000,
        settled_at: at - 8 * 86400000,
      },
    ]
    const config = defaultConfig()
    config.escalate.p = ['routine']
    const { read } = model(
      fake({
        asks,
        releases: [
          release,
          { ...release, id: 'past', target_at: new Date(at - 1).toISOString() },
          { ...release, id: 'soon', target_at: new Date(at + 86400000).toISOString() },
          { ...release, id: 'fine' },
        ],
      }).iii,
      config,
    )
    const data = await read.read('p')
    expect(data.direction.map((item) => [item.release.id, item.reasons])).toEqual([
      ['r', ['escalated-question']],
      ['past', ['past-target']],
      ['soon', ['open-cards']],
      ['fine', []],
    ])
    expect(
      data.items.map((item) => [
        item.kind === 'question' ? item.questionClass : item.kind,
        item.urgency,
      ]),
    ).toEqual([
      ['routine', 'now'],
      ['scope', 'no-rush'],
    ])
    expect(data.exceptions.blocked_cards).toEqual([card])
    expect(data.exceptions.old_questions).toHaveLength(2)
    expect(data.median_decision_wait_ms).toBe(5000)
  })
})
