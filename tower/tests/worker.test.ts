import assert from 'node:assert/strict'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { IIIClient } from 'iii-sdk'
import { defaultConfig } from '../src/config.js'
import { registerTower } from '../src/worker.js'
import type { KanbanQuestionView, SessionMeta, TowerTicket } from '../src/types.js'

const started = Date.parse('2026-11-22T12:00:00Z')
function fake() {
  const handlers = new Map<string, (input: unknown) => unknown>()
  const observers: Array<{ source: { function_id: string }; target: string }> = []
  const registrations = new Map<string, Record<string, unknown>>()
  const questions: KanbanQuestionView[] = []
  const runtime = {
    config: defaultConfig(),
    functions: [] as string[],
    sessions: [] as SessionMeta[],
    tickets: [] as TowerTicket[],
    state: new Map<string, unknown>(),
    observerFailure: '',
    writerStatus: null as Record<string, unknown> | null,
  }
  const call = vi.fn(
    async ({
      function_id: id,
      payload: input,
    }: {
      function_id: string
      payload: Record<string, unknown>
    }): Promise<unknown> => {
      const handler = handlers.get(id)
      if (handler) return handler(input)
      if (id === 'configuration::ensure') return {}
      if (id === 'configuration::get') return { value: runtime.config }
      if (id === 'engine::functions::list')
        return { functions: runtime.functions.map((function_id) => ({ function_id })) }
      if (id === 'state::list_keys') return { keys: [...runtime.state.keys()] }
      if (id === 'state::get') return runtime.state.get(String(input.key)) ?? null
      if (id === 'kanban::project::list')
        return {
          projects: [...new Set(runtime.tickets.map((card) => card.project_id))].map((id) => ({
            id,
            repos: ['owner/repo'],
          })),
        }
      if (id === 'kanban::release::list') return { releases: [] }
      if (id === 'kanban::board::get')
        return {
          columns: [
            {
              id: 'todo',
              label: 'To do',
              tickets: runtime.tickets.filter(
                (card) => !input.project_id || card.project_id === input.project_id,
              ),
              total: runtime.tickets.length,
            },
          ],
          done_status: 'done',
          id_prefix: 'KAN',
        }
      if (id === 'session::list') return { sessions: runtime.sessions }
      if (id === 'harness::ask::list') return { entries: [] }
      if (id === 'kanban::question::list') return { questions }
      if (id === 'harness::status')
        return String(input.session_id).startsWith('tower-')
          ? runtime.writerStatus
          : {
              session_id: input.session_id,
              status: 'running',
              turn_id: 'run',
              children: [],
              open_asks: 0,
              expects_wake: false,
            }
      if (id === 'session::messages-tail') return { messages: [] }
      if (id === 'kanban::ticket::list') return { tickets: [] }
      if (id === 'directory::agents::get') return { id: 'tower-writer' }
      if (id === 'session::ensure')
        return { meta: { kind: 'automation', metadata: { origin: 'tower' } } }
      if (id === 'harness::send') {
        runtime.writerStatus = {
          session_id: input.session_id,
          turn_id: 'writer-turn',
          status: 'running',
        }
        return { turn_id: 'writer-turn' }
      }
      if (id === 'state::set') {
        runtime.state.set(String(input.key), input.value)
        return {}
      }
      throw new Error(id)
    },
  )
  const iii = {
    call,
    registerFunction(
      id: string,
      handler: (input: unknown) => unknown,
      options: Record<string, unknown>,
    ) {
      handlers.set(id, handler)
      registrations.set(id, options)
      return {}
    },
    observe(source: { function_id: string }, target: string) {
      observers.push({ source, target })
      return {
        dispose: async () => {},
        ready: async () => {
          if (source.function_id === runtime.observerFailure) throw new Error('FORBIDDEN')
        },
      }
    },
  } as unknown as IIIClient
  return {
    iii,
    call,
    handlers,
    questions,
    observers,
    registrations,
    runtime,
    invoke: (id: string, input: unknown) => {
      const handler = handlers.get(id)
      assert.ok(handler)
      return handler(input)
    },
  }
}
const towers: Array<Awaited<ReturnType<typeof registerTower>>> = []
afterEach(async () => {
  for (const tower of towers.splice(0)) await tower.dispose()
  vi.useRealTimers()
})

function question(
  id: string,
  created: string,
  asked: number,
  state = 'recorded',
): KanbanQuestionView {
  return {
    comment: {
      id,
      ticket_id: 'KAN-1',
      body: id,
      author: 'agent',
      created_at: created,
      recorded_by: state === 'recorded' ? 'agent' : undefined,
      question: { session_id: 's', question_id: id, asked_at: asked, questions: [] },
    },
    state: state as KanbanQuestionView['state'],
  }
}

describe('worker functions without an engine', () => {
  it('registers all read contracts, the configuration, source events, and tolerates missing UI/optional workers', async () => {
    const f = fake()
    towers.push(await registerTower(f.iii, ['/missing/tower-ui']))
    expect(
      [...f.handlers.keys()].filter(
        (id) => !f.registrations.get(id)?.metadata && id !== 'tower::on-changed',
      ),
    ).toEqual([
      'tower::overview',
      'tower::needs-you::list',
      'tower::runs::list',
      'tower::prs::list',
      'tower::decided::list',
      'tower::assumptions::list',
      'tower::watch',
      'tower::summary::refresh',
    ])
    const overview = await f.invoke('tower::overview', { scope: 'all' })
    expect(overview).toMatchObject({
      items: [],
      runs: [],
      prs: [],
      availability: { github: false, approval_gate: false },
      prs_interval_min: 5,
      summary: { result: null, fresh: false, error: 'set the summary model in the tower settings' },
    })
    expect(f.handlers.has('tower::ui-content')).toBe(false)
    expect(f.observers.map(({ source }) => source.function_id)).toEqual(
      expect.arrayContaining([
        'harness::on-ask',
        'harness::on-ask-settled',
        'harness::on-turn-started',
        'harness::on-turn-completed',
        'kanban::on-change',
      ]),
    )
    expect(f.call).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'configuration::ensure',
        payload: expect.objectContaining({ id: 'tower', initial_value: defaultConfig() }),
      }),
    )
    expect(
      f.call.mock.calls.some(([request]) =>
        /answer|resolve|approve|harness::send/.test(request.function_id),
      ),
    ).toBe(false)
  })

  it('accepts the Engine stamp on every public RPC while keeping strict requests', async () => {
    const f = fake()
    towers.push(await registerTower(f.iii))
    for (const id of f.handlers.keys()) {
      if (f.registrations.get(id)?.metadata || id === 'tower::on-changed') continue
      await expect(
        f.invoke(id, { scope: 'all', _caller_worker_id: 'ade-9' }),
      ).resolves.toBeDefined()
      await expect(f.invoke(id, { scope: 'all', surprise: true })).rejects.toThrow()
    }
  })

  it('fails boot when an observer is rejected and registers the sinks as callable internals', async () => {
    const f = fake()
    f.runtime.observerFailure = 'harness::on-turn-completed'
    await expect(registerTower(f.iii)).rejects.toThrow(
      'Tower observer registration failed for harness::on-turn-completed',
    )
    for (const { target } of f.observers) {
      expect(f.registrations.get(target)).toMatchObject({ metadata: { internal: true } })
      expect(f.registrations.get(target)).not.toHaveProperty('private', true)
    }
  })

  it('rejects unknown project IDs before creating watch or writer state', async () => {
    const f = fake()
    f.runtime.config.summary.model = 'openai::writer-model'
    const tower = await registerTower(f.iii)
    towers.push(tower)
    f.call.mockClear()
    for (const id of ['tower::watch', 'tower::summary::refresh']) {
      await expect(f.invoke(id, { scope: 'unknown', _caller_worker_id: 'agent' })).rejects.toThrow(
        'unknown Tower project scope',
      )
    }
    expect(tower.read.knownScopes()).toEqual(['all'])
    expect(tower.read.activeScopes()).toEqual([])
    expect(f.call.mock.calls.map(([request]) => request.function_id)).toEqual([
      'kanban::project::list',
      'kanban::project::list',
    ])
  })

  it('wakes the scheduler at the configured cadence without another watch', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(started)
    const f = fake()
    f.runtime.config.summary = {
      model: 'openai::writer-model',
      interval_min: 0.2,
      language: 'English',
    }
    towers.push(await registerTower(f.iii))
    await f.invoke('tower::watch', { scope: 'all' })
    const sends = () =>
      f.call.mock.calls.filter(([request]) => request.function_id === 'harness::send')
    await vi.advanceTimersByTimeAsync(100)
    expect(sends()).toHaveLength(1)
    f.runtime.writerStatus = { ...f.runtime.writerStatus, status: 'failed' }
    await f.invoke('tower::turn-completed', {
      input: { session_id: 'tower-all', turn_id: 'writer-turn', status: 'completed' },
    })
    await vi.advanceTimersByTimeAsync(11999)
    expect(sends()).toHaveLength(1)
    // A real later turn has stamped the writer profile on the session metadata.
    const fakeCall = f.call.getMockImplementation()
    assert.ok(fakeCall)
    f.call.mockImplementation((request) =>
      request.function_id === 'session::ensure'
        ? Promise.resolve({
            meta: {
              kind: 'automation',
              metadata: { origin: 'tower', agent_profile: { id: 'tower-writer' } },
            },
          })
        : fakeCall(request),
    )
    await vi.advanceTimersByTimeAsync(1)
    expect(sends()).toHaveLength(2)
  })

  it('reconciles a saved pending turn at boot before any overview or watch', async () => {
    const f = fake()
    const result = {
      checked_at: new Date(started).toISOString(),
      unchanged: false,
      brief: { headline: 'Recovered', text: 'The writer finished.', refs: ['KAN-1'] },
      direction: [],
      diary: [],
      now: {},
      pr_waits: {},
    }
    f.runtime.config.summary.model = 'openai::writer-model'
    f.runtime.state.set('summary:all', {
      pending_turn: { turn_id: 'before-restart', hash: 'recovered-hash', order: 1 },
      send_order: 1,
    })
    f.runtime.writerStatus = {
      session_id: 'tower-all',
      turn_id: 'before-restart',
      status: 'completed',
      result,
    }

    towers.push(await registerTower(f.iii))

    expect(f.runtime.state.get('summary:all')).toMatchObject({
      result,
      pending_turn: null,
      valid_hash: 'recovered-hash',
      send_order: 1,
    })
    expect(f.call.mock.calls.some(([request]) => request.function_id === 'harness::send')).toBe(
      false,
    )
  })

  it('filters assumptions by comment.created_at, and decided excludes recorded/open questions', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(started)
    const f = fake()
    f.questions.push(
      question('old-comment', '2026-11-21T00:00:00Z', started),
      question('new-comment', '2026-11-22T11:00:00Z', 0),
      question('answered', '2026-11-22T10:00:00Z', 0, 'answered'),
      question('open', '2026-11-22T10:00:00Z', 0, 'open'),
    )
    towers.push(await registerTower(f.iii))
    expect(
      await f.invoke('tower::assumptions::list', { scope: 'all', since: '2026-11-22T00:00:00Z' }),
    ).toMatchObject({ questions: [{ commentId: 'new-comment' }] })
    expect(await f.invoke('tower::decided::list', { scope: 'all' })).toMatchObject({
      questions: [{ commentId: 'answered' }],
    })
    await expect(f.invoke('tower::decided::list', { scope: 'all', cursor: 'bad' })).rejects.toThrow(
      'invalid decided cursor',
    )
  })

  it('announces summary staleness at its deadline without a source event or open tab', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(started)
    const f = fake()
    f.runtime.config.summary.model = 'openai::writer-model'
    const checkpoint = new Date(started - 30 * 60000 + 10000).toISOString()
    f.runtime.state.set('summary:all', {
      checked_at: checkpoint,
      valid_hash: 'old',
      result: {
        checked_at: checkpoint,
        unchanged: false,
        brief: { headline: '', text: '', refs: [] },
        diary: [],
        now: {},
        pr_waits: {},
        direction: [],
      },
    })
    towers.push(await registerTower(f.iii))
    expect(await f.invoke('tower::overview', { scope: 'all' })).toMatchObject({
      summary: { fresh: true },
    })
    f.call.mockClear()
    await vi.advanceTimersByTimeAsync(10001)
    expect(f.call).toHaveBeenCalledWith({
      function_id: 'tower::on-changed',
      payload: { scope: 'all', as_of: new Date(started + 10001).toISOString(), parts: ['summary'] },
    })
    expect(await f.invoke('tower::overview', { scope: 'all' })).toMatchObject({
      summary: { fresh: false },
    })
  })

  it('discovers live project scopes at boot and on new root events with no tab open', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(started)
    const f = fake()
    const root: SessionMeta = {
      session_id: 'root',
      title: 'Root',
      description: '',
      kind: 'user',
      status: 'working',
      created_at: started,
      updated_at: started,
      message_count: 0,
      metadata: { kanban_ticket: 'KAN-1' },
    }
    const card: TowerTicket = {
      id: 'c1',
      key: 'KAN-1',
      title: 'Ship',
      status: 'todo',
      priority: 'medium',
      labels: [],
      pills: [],
      created_at: new Date(started).toISOString(),
      updated_at: new Date(started).toISOString(),
      project_id: 'p',
      open_questions: 0,
    }
    f.runtime.sessions.push(root)
    f.runtime.tickets.push(card)
    const tower = await registerTower(f.iii)
    towers.push(tower)
    expect(tower.read.activeScopes()).toEqual(['all', 'p'])
    f.runtime.sessions.push({ ...root, session_id: 'new', metadata: { kanban_ticket: 'KAN-2' } })
    f.runtime.tickets.push({ ...card, id: 'c2', key: 'KAN-2', project_id: 'p2' })
    await f.invoke('tower::turn-started', { input: { session_id: 'new', depth: 0 } })
    await vi.advanceTimersByTimeAsync(2000)
    expect(tower.read.activeScopes()).toEqual(['all', 'p', 'p2'])
  })

  it('notifies availability and summary configuration changes immediately', async () => {
    const f = fake()
    towers.push(await registerTower(f.iii))
    f.call.mockClear()
    f.runtime.functions.push('approval::list-pending', 'approval::set-mode')
    await f.invoke('tower::availability-updated', {})
    expect(f.call).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'tower::on-changed',
        payload: expect.objectContaining({ scope: 'all', parts: ['availability', 'summary'] }),
      }),
    )
    f.runtime.config.summary.language = 'Portuguese'
    await f.invoke('tower::config-updated', {})
    expect(f.call).toHaveBeenCalledWith(
      expect.objectContaining({
        function_id: 'tower::on-changed',
        payload: expect.objectContaining({ scope: 'all', parts: ['summary', 'configuration'] }),
      }),
    )
  })

  it('can summarize the remaining sources when optional GitHub is absent', async () => {
    const f = fake()
    f.runtime.config.summary.model = 'openai::writer-model'
    f.runtime.tickets.push({
      id: 'c',
      key: 'KAN-1',
      title: 'Ship',
      status: 'todo',
      priority: 'medium',
      labels: [],
      pills: [],
      created_at: new Date(started).toISOString(),
      updated_at: new Date(started).toISOString(),
      project_id: 'p',
      open_questions: 0,
    })
    towers.push(await registerTower(f.iii))
    await f.invoke('tower::watch', { scope: 'p' })
    await f.invoke('tower::summary::refresh', { scope: 'p' })
    const sent = f.call.mock.calls.find(([request]) => request.function_id === 'harness::send')
    assert.ok(sent)
    expect(JSON.parse(String(sent[0].payload.message))).toMatchObject({
      repos: [],
      project_id: 'p',
    })
    expect(
      f.call.mock.calls.some(([request]) => request.function_id.startsWith('approval::')),
    ).toBe(false)
  })

  it('validates scope inputs and sends debounced change notifications from observed source events', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(started)
    const f = fake()
    towers.push(await registerTower(f.iii))
    await expect(f.invoke('tower::overview', { scope: '' })).rejects.toThrow()
    await f.invoke('tower::overview', { scope: 'all' })
    f.call.mockClear()
    f.questions.push(question('new', new Date(started).toISOString(), started))
    const observer = f.observers.find(
      (observer) => observer.source.function_id === 'harness::on-ask',
    )
    assert.ok(observer)
    await f.invoke(observer.target, { input: { session_id: 's' } })
    await vi.advanceTimersByTimeAsync(2000)
    expect(f.call).toHaveBeenCalledWith({
      function_id: 'tower::on-changed',
      payload: {
        scope: 'all',
        as_of: new Date(started + 2000).toISOString(),
        parts: expect.arrayContaining(['questions']),
      },
    })
  })
})
